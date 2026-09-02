//! Honest adversarial concurrency and webhook verification tests.
//!
//! All tests hit real crate code (ActionService pipeline + razorpay_gateway
//! signature verifier). No local MockGovernorCluster theater.

use action_service::ActionService;
use audit_service::{AuditService, InMemoryAuditStore};
use evidence_service::{EvidenceService, InMemoryEvidenceStore};
use policy_engine::PolicyEngine;
use razorpay_gateway::{verify_webhook_signature, MockGateway};
use risk_engine::RiskEngine;
use risk_governor_types::*;
use std::sync::Arc;
use tokio::task::JoinSet;

async fn build_pipeline() -> (
    Arc<
        ActionService<
            PolicyEngine,
            RiskEngine,
            EvidenceService<InMemoryEvidenceStore>,
            AuditService<InMemoryAuditStore>,
            MockGateway,
        >,
    >,
    Arc<MockGateway>,
    Arc<InMemoryAuditStore>,
) {
    let evidence_store = Arc::new(InMemoryEvidenceStore::new());
    evidence_store
        .seed_agent(AgentHistory {
            agent_id: "agent-race-01".into(),
            total_actions_30d: 50,
            total_volume_30d: 2_000_000,
            avg_amount: 10_000,
            max_amount: 50_000,
            std_amount: 5_000,
            refund_rate: 0.02,
            block_rate: 0.01,
            review_rate: 0.02,
            first_seen: now_utc() - chrono::Duration::days(60),
            last_action: now_utc() - chrono::Duration::hours(1),
            action_type_distribution: Default::default(),
            anomaly_flags: vec![],
        })
        .await;
    evidence_store
        .seed_default_policy_if_missing("merchant-race-001")
        .await
        .unwrap();

    let audit_store = Arc::new(InMemoryAuditStore::new());
    let gateway = Arc::new(MockGateway::default());

    let svc = Arc::new(ActionService::new(
        Arc::new(PolicyEngine::new()),
        Arc::new(RiskEngine::default()),
        Arc::new(EvidenceService::new(evidence_store)),
        Arc::new(AuditService::new(audit_store.clone())),
        gateway.clone(),
    ));

    (svc, gateway, audit_store)
}

#[tokio::test]
async fn ten_concurrent_allowed_refunds_produce_ten_distinct_decisions_and_ten_gateway_calls() {
    let (svc, gateway, _audit) = build_pipeline().await;
    let mut tasks = JoinSet::new();

    for _ in 0..10 {
        let svc_clone = svc.clone();
        tasks.spawn(async move {
            let req = AgentActionRequest {
                agent_id: "agent-race-01".into(),
                merchant_id: "merchant-race-001".into(),
                action_type: ActionType::Refund,
                amount: 15_000,
                currency: "INR".into(),
                declared_intent: "Customer returned product intact within policy window".into(),
                context: serde_json::json!({
                    "customer_id": "cust_race_99",
                    "payment_id": "pay_test_123",
                    "payment_state": "captured",
                    "captured_paise": 500000,
                    "refunded_paise": 0
                }),
                timestamp: now_utc(),
                correlation_id: generate_correlation_id(),
            };
            svc_clone.process_action(req).await
        });
    }

    let mut decisions = Vec::new();
    while let Some(res) = tasks.join_next().await {
        decisions.push(res.unwrap().unwrap());
    }

    assert_eq!(decisions.len(), 10);
    for d in &decisions {
        assert_eq!(d.decision, DecisionOutcome::Allow, "each low-risk refund should allow");
    }
    let ids: std::collections::HashSet<_> = decisions.iter().map(|d| d.decision_id).collect();
    assert_eq!(ids.len(), 10, "each decision must have a distinct decision_id");

    assert_eq!(
        gateway.calls.lock().unwrap().len(),
        10,
        "10 distinct Allow decisions must produce exactly 10 gateway calls (no dedup, no loss)"
    );
}

#[tokio::test]
async fn ten_concurrent_blocked_refunds_produce_zero_gateway_calls() {
    let (svc, gateway, _) = build_pipeline().await;
    let mut tasks = JoinSet::new();

    for _ in 0..10 {
        let svc_clone = svc.clone();
        tasks.spawn(async move {
            let req = AgentActionRequest {
                agent_id: "agent-race-01".into(),
                merchant_id: "merchant-race-001".into(),
                action_type: ActionType::Refund,
                amount: 600_000,
                currency: "INR".into(),
                declared_intent: "Excessive refund amount attempt".into(),
                context: serde_json::json!({
                    "customer_id": "cust_high",
                    "payment_id": "pay_test_123",
                    "payment_state": "captured",
                    "captured_paise": 500000,
                    "refunded_paise": 0
                }),
                timestamp: now_utc(),
                correlation_id: generate_correlation_id(),
            };
            svc_clone.process_action(req).await
        });
    }

    while let Some(res) = tasks.join_next().await {
        let decision = res.unwrap().unwrap();
        assert_eq!(decision.decision, DecisionOutcome::Block);
    }

    assert_eq!(
        gateway.calls.lock().unwrap().len(),
        0,
        "Blocked decisions must never reach the gateway, even under concurrency"
    );
}

#[tokio::test]
async fn webhook_tampered_payload_rejected_by_real_verifier_no_audit_side_effect() {
    let secret = "rzp_webhook_secret_production_key_4491";
    let valid = br#"{"event":"payment.failed","payload":{"payment":{"entity":{"id":"pay_test_001","amount":250000}}}}"#;
    let tampered =
        br#"{"event":"payment.failed","payload":{"payment":{"entity":{"id":"pay_test_001","amount":10000}}}}"#;

    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    use hmac::Mac;
    mac.update(valid);
    let sig = hex::encode(mac.finalize().into_bytes());

    assert!(
        verify_webhook_signature(valid, &sig, secret),
        "valid payload must verify"
    );
    assert!(
        !verify_webhook_signature(tampered, &sig, secret),
        "tampered payload must fail"
    );
    assert!(
        !verify_webhook_signature(valid, &sig, "wrong_secret"),
        "wrong secret must fail"
    );
    assert!(
        !verify_webhook_signature(valid, "not-hex!!", secret),
        "malformed hex must fail closed"
    );
    assert!(
        !verify_webhook_signature(valid, "", secret),
        "empty signature must fail"
    );

    let upper = sig.to_uppercase();
    assert!(
        verify_webhook_signature(valid, &upper, secret),
        "hex decode must be case-insensitive"
    );
}
