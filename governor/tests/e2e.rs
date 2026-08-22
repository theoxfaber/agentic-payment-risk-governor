use action_service::ActionService;
use audit_service::{AuditService, InMemoryAuditStore};
use evidence_service::{EvidenceStore, InMemoryEvidenceStore};
use razorpay_gateway::MockGateway;
use risk_governor_replay::ReplayEngine;
use risk_governor_types::*;
use std::sync::Arc;

type TestService = ActionService<
    policy_engine::PolicyEngine,
    risk_engine::RiskEngine,
    evidence_service::EvidenceService<InMemoryEvidenceStore>,
    AuditService<InMemoryAuditStore>,
    MockGateway,
>;

fn wire() -> (Arc<TestService>, Arc<InMemoryEvidenceStore>, Arc<AuditService<InMemoryAuditStore>>, Arc<MockGateway>) {
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
    (
        svc,
        evidence_store,
        Arc::new(AuditService::new(audit_store)),
        gateway,
    )
}

async fn seed(store: &InMemoryEvidenceStore) {
    store
        .seed_agent(AgentHistory {
            agent_id: "agent-1".into(),
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
    store.seed_default_policy_if_missing("m-001").await.unwrap();
}

fn refund(agent: &str, amount: i64, intent: &str) -> AgentActionRequest {
    AgentActionRequest {
        agent_id: agent.into(),
        merchant_id: "m-001".into(),
        action_type: ActionType::Refund,
        amount,
        currency: "INR".into(),
        declared_intent: intent.into(),
        context: serde_json::json!({ "payment_id": "pay_X", "customer_id": "c1" }),
        timestamp: now_utc(),
        correlation_id: generate_correlation_id(),
    }
}

/// Phase 1 checkpoint: one full ALLOW/REVIEW/BLOCK decision set, all logged.
#[tokio::test]
async fn allow_review_block_end_to_end() {
    let (svc, store, audit, gateway) = wire();
    seed(&store).await;

    // ALLOW: small legit refund
    let allow = svc.process_action(refund("agent-1", 50_000, "refund for order #123")).await.unwrap();
    assert_eq!(allow.decision, DecisionOutcome::Allow);

    // REVIEW: above require_approval_above (100k) → deterministic human queue
    let review = svc.process_action(refund("agent-1", 150_000, "refund order #456")).await.unwrap();
    assert_eq!(review.decision, DecisionOutcome::Review);

    // BLOCK: over max_refund_amount hard cap
    let block = svc.process_action(refund("agent-1", 600_000, "refund #789")).await.unwrap();
    assert_eq!(block.decision, DecisionOutcome::Block);

    // Gateway fires ONLY on Allow
    assert_eq!(gateway.calls.lock().unwrap().len(), 1);
    assert_eq!(gateway.calls.lock().unwrap()[0].0, allow.decision_id);

    // Full audit trail per decision: Requested → PolicyEvaluated → RiskScored → DecisionMade (+ RazorpayCalled on allow)
    let trail = audit.trail_for(allow.decision_id).await.unwrap();
    let kinds: Vec<_> = trail.iter().map(|r| r.event_type).collect();
    assert!(kinds.contains(&AuditEventType::ActionRequested));
    assert!(kinds.contains(&AuditEventType::PolicyEvaluated));
    assert!(kinds.contains(&AuditEventType::RiskScored));
    assert!(kinds.contains(&AuditEventType::DecisionMade));
    assert!(kinds.contains(&AuditEventType::RazorpayCalled));

    let blocked_trail = audit.trail_for(block.decision_id).await.unwrap();
    assert!(!blocked_trail.iter().any(|r| r.event_type == AuditEventType::RazorpayCalled));

    // Velocity feedback loop recorded the three processed actions
    let v = store.velocity("agent-1").await.unwrap();
    assert_eq!(v.actions_last_hour, 3);
}

/// Checkpoint question, executable: for any decision, exactly which rule/feature caused it?
#[tokio::test]
async fn replay_explains_the_decision() {
    let (svc, store, audit, _) = wire();
    seed(&store).await;

    let req = refund("agent-1", 600_000, "refund big");
    let d = svc.process_action(req).await.unwrap();
    assert_eq!(d.decision, DecisionOutcome::Block);

    let replay = ReplayEngine::new(audit.clone()).replay(d.decision_id).await.unwrap();

    assert_eq!(replay.decision.decision_id, d.decision_id);
    assert_eq!(replay.risk_model_version, d.risk_result.model_version);
    assert_eq!(replay.evidence_at_decision.merchant_policy.merchant_id, "m-001");

    // The cause is in the snapshot: hard-cap violation names the rule and numbers
    let violation = &replay.decision.policy_result.violated_thresholds[0];
    assert!(violation.contains("600000") && violation.contains("500000"), "violation: {violation}");
}

/// Failure-mode discipline: unknown agent fails closed to an error, never silent-allow.
#[tokio::test]
async fn unknown_agent_fails_closed() {
    let (svc, store, _audit, _gw) = wire();
    seed(&store).await; // seeds agent-1 only

    let result = svc.process_action(refund("ghost-agent", 10_000, "refund")).await;
    assert!(result.is_err(), "unknown agent must error, not silently pass");
}
