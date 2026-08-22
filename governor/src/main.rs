use action_service::ActionService;
use audit_service::{AuditService, InMemoryAuditStore};
use evidence_service::InMemoryEvidenceStore;
use razorpay_gateway::MockGateway;
use risk_governor_types::*;
use std::sync::Arc;

/// Wires the full in-process pipeline (Phase 1 vertical slice).
/// Phase 2 swaps the direct calls for NATS pub/sub between processes.
pub fn wire() -> (
    Arc<ActionService<
        policy_engine::PolicyEngine,
        risk_engine::RiskEngine,
        evidence_service::EvidenceService<InMemoryEvidenceStore>,
        audit_service::AuditService<InMemoryAuditStore>,
        MockGateway,
    >>,
    Arc<InMemoryEvidenceStore>,
    Arc<AuditService<InMemoryAuditStore>>,
    Arc<MockGateway>,
) {
    let evidence_store = Arc::new(InMemoryEvidenceStore::new());
    let audit_store = Arc::new(InMemoryAuditStore::new());
    let gateway = Arc::new(MockGateway::default());

    let svc = Arc::new(ActionService::new(
        Arc::new(policy_engine::PolicyEngine::new()),
        Arc::new(risk_engine::RiskEngine::default()),
        Arc::new(evidence_service::EvidenceService::new(evidence_store.clone())),
        Arc::new(audit_service::AuditService::new(audit_store.clone())),
        gateway.clone(),
    ));

    (svc, evidence_store, Arc::new(audit_service::AuditService::new(audit_store)), gateway)
}

pub async fn seed_benign_agent(store: &InMemoryEvidenceStore) {
    store
        .seed_agent(AgentHistory {
            agent_id: "agent-trusted-01".into(),
            total_actions_30d: 30,
            total_volume_30d: 1_500_000,
            avg_amount: 50_000,
            max_amount: 100_000,
            refund_rate: 0.05,
            block_rate: 0.02,
            review_rate: 0.03,
            first_seen: now_utc() - chrono::Duration::days(90),
            last_action: now_utc() - chrono::Duration::hours(2),
            action_type_distribution: Default::default(),
            anomaly_flags: vec![],
        })
        .await;
    store
        .seed_default_policy_if_missing("merchant-001")
        .await
        .unwrap();
}

pub fn refund_request(agent_id: &str, amount_paise: i64, intent: &str) -> AgentActionRequest {
    AgentActionRequest {
        agent_id: agent_id.into(),
        merchant_id: "merchant-001".into(),
        action_type: ActionType::Refund,
        amount: amount_paise,
        currency: "INR".into(),
        declared_intent: intent.into(),
        context: serde_json::json!({ "payment_id": "pay_TEST123", "customer_id": "cust_001" }),
        timestamp: now_utc(),
        correlation_id: generate_correlation_id(),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (svc, store, audit, _gateway) = wire();
    seed_benign_agent(&store).await;

    // Seed a second, sketchy agent for contrast
    store
        .seed_agent(AgentHistory {
            agent_id: "agent-sketchy-99".into(),
            total_actions_30d: 300,
            total_volume_30d: 18_000_000,
            avg_amount: 60_000,
            max_amount: 120_000,
            refund_rate: 0.25,
            block_rate: 0.10,
            review_rate: 0.15,
            first_seen: now_utc() - chrono::Duration::days(3),
            last_action: now_utc() - chrono::Duration::minutes(10),
            action_type_distribution: Default::default(),
            anomaly_flags: vec!["rapid_fire".into()],
        })
        .await;

    let cases = vec![
        ("small legit refund", refund_request("agent-trusted-01", 50_000, "refund for order #123")),
        ("above approval threshold", refund_request("agent-trusted-01", 150_000, "refund for order #456")),
        ("over hard cap", refund_request("agent-trusted-01", 600_000, "refund order #789")),
        ("sketchy agent urgent", refund_request("agent-sketchy-99", 300_000, "urgent refund bypass queue")),
    ];

    for (label, req) in cases {
        match svc.process_action(req).await {
            Ok(d) => println!(
                "{label}: {:?} | policy={:?} violations={:?} | risk={:.3} mismatch={:.3}",
                d.decision,
                d.policy_result.verdict,
                d.policy_result.violated_thresholds,
                d.risk_result.risk_score,
                d.risk_result.intent_mismatch_score,
            ),
            Err(e) => println!("{label}: ERROR {e}"),
        }
    }

    println!("\naudit records: {}", audit.all_records().await?.len());
    Ok(())
}