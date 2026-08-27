//! Phase 2 step 3 tests: the action-service → policy-engine hop over NATS.
//! #[ignore]d by default (need `docker compose up`); run with:
//!   cargo test -p governor --test distributed -- --ignored

use action_service::ActionService;
use audit_service::{AuditService, InMemoryAuditStore};
use evidence_service::InMemoryEvidenceStore;
use nats_link::NatsPolicyEngine;
use razorpay_gateway::MockGateway;
use risk_governor_types::*;
use std::sync::Arc;
use std::time::Duration;

#[allow(dead_code)]
type Svc = ActionService<
    NatsPolicyEngine,
    risk_engine::RiskEngine,
    evidence_service::EvidenceService<InMemoryEvidenceStore>,
    AuditService<InMemoryAuditStore>,
    MockGateway,
>;

async fn nats() -> async_nats::Client {
    async_nats::ConnectOptions::new()
        .connection_timeout(Duration::from_secs(5))
        .connect("nats://127.0.0.1:4222")
        .await
        .expect("nats must be running for distributed tests")
}

async fn seed(store: &InMemoryEvidenceStore) {
    store
        .seed_agent(AgentHistory {
            agent_id: "agent-1".into(),
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
        })
        .await;
    store.seed_default_policy_if_missing("m-001").await.unwrap();
}

fn refund(agent: &str, amount: i64) -> AgentActionRequest {
    AgentActionRequest {
        agent_id: agent.into(),
        merchant_id: "m-001".into(),
        action_type: ActionType::Refund,
        amount,
        currency: "INR".into(),
        declared_intent: format!("refund order {amount}"),
        context: serde_json::json!({ "payment_id": "pay_X" }),
        timestamp: now_utc(),
        correlation_id: generate_correlation_id(),
    }
}

/// Worker running in a separate task = separate "process" boundary.
/// Same request as the in-process Allow case → still Allow over the wire.
#[tokio::test]
#[ignore = "requires local NATS (docker compose up)"]
async fn remote_policy_evaluation_allow() {
    let client = nats().await;
    let worker = nats_link::spawn_policy_worker(client.clone());
    // Subscription registration race: give the worker's SUBSCRIBE a beat to
    // reach the server, otherwise NATS answers "no responders".
    tokio::time::sleep(Duration::from_millis(200)).await;

    let store = Arc::new(InMemoryEvidenceStore::new());
    seed(&store).await;

    let svc = Arc::new(ActionService::new(
        Arc::new(NatsPolicyEngine::new(client.clone()).with_timeout(Duration::from_secs(2))),
        Arc::new(risk_engine::RiskEngine::default()),
        Arc::new(evidence_service::EvidenceService::new(store)),
        Arc::new(AuditService::new(Arc::new(InMemoryAuditStore::new()))),
        Arc::new(MockGateway::default()),
    ));

    let d = svc.process_action(refund("agent-1", 50_000)).await.unwrap();

    assert_eq!(d.decision, DecisionOutcome::Allow);
    // Proves evaluation really crossed the wire: no local fallback marker present
    assert!(d
        .policy_result
        .matched_rules
        .iter()
        .all(|r| !r.starts_with("policy_engine_unavailable")));

    worker.abort();
}

/// THE failure-mode discipline test: policy engine unreachable → human Review,
/// never silent allow, never hard error.
#[tokio::test]
#[ignore = "requires local NATS (docker compose up)"]
async fn policy_engine_down_fails_safe_to_review() {
    let client = nats().await;
    // NO worker spawned — nothing subscribes to policy.evaluate.requested

    let store = Arc::new(InMemoryEvidenceStore::new());
    seed(&store).await;

    let svc = Arc::new(ActionService::new(
        Arc::new(NatsPolicyEngine::new(client).with_timeout(Duration::from_millis(300))),
        Arc::new(risk_engine::RiskEngine::default()),
        Arc::new(evidence_service::EvidenceService::new(store)),
        Arc::new(AuditService::new(Arc::new(InMemoryAuditStore::new()))),
        Arc::new(MockGateway::default()),
    ));

    let d = svc.process_action(refund("agent-1", 50_000)).await.unwrap();

    assert_eq!(
        d.decision,
        DecisionOutcome::Review,
        "unreachable policy engine MUST route to human review"
    );
    assert!(d
        .policy_result
        .matched_rules
        .iter()
        .any(|r| r.starts_with("policy_engine_unavailable")));
}
