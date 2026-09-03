//! Boot-time defaults: the shipped demo graph, seeded entities, and shared
//! test-support builders used by handler and router tests.

use crate::backends::EvidenceBackend;
use investigation_engine::CustomerBehavior;
use risk_governor_types::*;
use std::collections::HashMap;
use std::sync::Arc;

/// Default graph shipped with the server: a few isolated customers sharing
/// nothing. Unknown customer_ids land here → "no_cluster" → Unsupported
/// verdict → zero added friction for ordinary traffic.
pub(crate) fn default_graph_and_behaviors() -> (Arc<risk_graph::PropertyGraph>, HashMap<String, CustomerBehavior>) {
    let mut b = risk_graph::GraphBuilder::new();
    for c in ["cust_agent-trusted-01", "cust_agent-sketchy-99"] {
        b = b.entity(risk_graph::EntityKind::Customer, c);
    }
    let graph = Arc::new(b.build());
    (graph, HashMap::new())
}

pub(crate) async fn seed_demo_entities(store: &EvidenceBackend) -> Result<(), anyhow::Error> {
    let agents = [
        AgentHistory {
            agent_id: "agent-trusted-01".into(),
            total_actions_30d: 30,
            total_volume_30d: 1_500_000,
            avg_amount: 50_000,
            max_amount: 100_000,
            std_amount: 15_000,
            refund_rate: 0.05,
            block_rate: 0.02,
            review_rate: 0.03,
            first_seen: now_utc() - chrono::Duration::days(90),
            last_action: now_utc() - chrono::Duration::hours(2),
            action_type_distribution: Default::default(),
            anomaly_flags: vec![],
        },
        AgentHistory {
            agent_id: "agent-sketchy-99".into(),
            total_actions_30d: 300,
            total_volume_30d: 18_000_000,
            avg_amount: 60_000,
            max_amount: 120_000,
            std_amount: 20_000,
            refund_rate: 0.25,
            block_rate: 0.10,
            review_rate: 0.15,
            first_seen: now_utc() - chrono::Duration::days(3),
            last_action: now_utc() - chrono::Duration::minutes(10),
            action_type_distribution: Default::default(),
            anomaly_flags: vec!["rapid_fire".into()],
        },
    ];
    match store {
        EvidenceBackend::Mem(s) => {
            for a in agents {
                s.seed_agent(a).await;
            }
            s.seed_default_policy_if_missing("merchant-001").await?;
        }
        EvidenceBackend::Pg(s) => {
            let mut seed = serde_json::Map::new();
            seed.insert(
                "agents".into(),
                serde_json::to_value(&agents).expect("agents serialize"),
            );
            // Merchant policy mirrors InMemory defaults (seed_default_policy_if_missing).
            let policy = MerchantPolicy {
                merchant_id: "merchant-001".into(),
                max_refund_amount: 500_000,
                max_payout_amount: 1_000_000,
                max_payment_link_amount: 250_000,
                daily_refund_limit: 2_000_000,
                daily_payout_limit: 5_000_000,
                velocity_threshold_per_hour: 10,
                allowed_countries: vec![],
                blocked_countries: vec![],
                require_approval_above: 100_000,
                custom_rules: vec![],
                risk_tier: RiskTier::Standard,
                pmla_retention_days: 1825,
                fri_score: None,
            };
            seed.insert(
                "merchants".into(),
                serde_json::json!([serde_json::to_value(&policy).expect("policy serialize")]),
            );
            s.seed_from_json(&serde_json::Value::Object(seed).to_string()).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Shared builders for handler/router tests. In-memory stores, mock
    //! gateway, fixed API key — fully offline.

    use super::*;
    use crate::backends::{AuditBackend, EvidenceBackend, Gateway};
    use crate::state::{AppState, Metrics};
    use action_service::ActionService;
    use audit_service::{AuditService, InMemoryAuditStore};
    use evidence_service::{EvidenceService, InMemoryEvidenceStore};
    use investigation_engine::{Baseline, GraphInvestigator};
    use razorpay_gateway::MockGateway;
    use std::collections::HashMap as Map;
    use std::num::NonZeroUsize;

    pub(crate) const TEST_KEY: &str = "rgov_test_key";

    pub(crate) async fn test_state() -> Arc<AppState> {
        let evidence_store = Arc::new(InMemoryEvidenceStore::new());
        seed_demo_entities(&EvidenceBackend::Mem(evidence_store.clone()))
            .await
            .unwrap();
        let audit_store = Arc::new(InMemoryAuditStore::new());
        let gateway = Arc::new(Gateway::Mock(Arc::new(MockGateway::default())));
        let (graph, behaviors) = default_graph_and_behaviors();
        let investigator = GraphInvestigator::new(graph.clone(), behaviors.clone(), Map::new(), Baseline::default());
        let svc = Arc::new(
            ActionService::new(
                Arc::new(policy_engine::PolicyEngine::new()),
                Arc::new(risk_engine::RiskEngine::default()),
                Arc::new(EvidenceService::new(Arc::new(EvidenceBackend::Mem(evidence_store)))),
                Arc::new(AuditService::new(Arc::new(AuditBackend::Mem(audit_store.clone())))),
                gateway.clone(),
            )
            .with_investigator(investigator.into_trait())
            .with_learned_scorer(Arc::new(action_service::learned::DefaultLearnedScorer::from_embedded())),
        );
        Arc::new(AppState {
            svc,
            audit: Arc::new(AuditService::new(Arc::new(AuditBackend::Mem(audit_store)))),
            gateway,
            decisions: tokio::sync::RwLock::new(lru::LruCache::new(NonZeroUsize::new(1_000).unwrap())),
            idempotency: tokio::sync::Mutex::new(Map::new()),
            metrics: Arc::new(Metrics::default()),
            pg: None,
            api_key: TEST_KEY.into(),
            review_key: None,
            anchor_key: None,
            webhook_secret: None,
            graph,
            behaviors,
        })
    }

    pub(crate) fn submit_body(
        agent: &str,
        amount: i64,
    ) -> (axum::http::HeaderMap, axum::Json<crate::routes::SubmitAction>) {
        (
            axum::http::HeaderMap::new(),
            axum::Json(crate::routes::SubmitAction {
                agent_id: agent.into(),
                merchant_id: "merchant-001".into(),
                action_type: ActionType::Refund,
                amount,
                currency: Some("INR".into()),
                declared_intent: "refund for order #1".into(),
                context: serde_json::json!({ "payment_id": "pay_test_123", "payment_state": "captured", "captured_paise": 500000, "refunded_paise": 0 }),
                idempotency_key: None,
            }),
        )
    }
}
