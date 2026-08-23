//! Gateway behavior without network: mock recording, signature verification,
//! and URL construction for the money-movement endpoints.

use action_service::RazorpayGateway as _;
use hmac::Mac;
use razorpay_gateway::{verify_webhook_signature, MockGateway};
use risk_governor_types::*;
use std::sync::Arc;

fn refund_request(amount: i64) -> AgentActionRequest {
    AgentActionRequest {
        agent_id: "agent-01".into(),
        merchant_id: "merchant-001".into(),
        action_type: ActionType::Refund,
        amount,
        currency: "INR".into(),
        declared_intent: "refund order #1".into(),
        context: serde_json::json!({ "payment_id": "pay_TEST123" }),
        timestamp: now_utc(),
        correlation_id: generate_correlation_id(),
    }
}

#[tokio::test]
async fn mock_gateway_records_every_call_with_decision_id() {
    let gw = Arc::new(MockGateway::default());
    let d1 = generate_correlation_id();
    let d2 = generate_correlation_id();

    gw.execute(&refund_request(50_000), d1).await.unwrap();
    gw.execute(&refund_request(75_000), d2).await.unwrap();

    let calls = gw.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, d1);
    assert_eq!(calls[1].0, d2);
    assert_eq!(calls[1].1["amount"], 75_000);
}

#[tokio::test]
async fn mock_response_is_deterministic_per_decision() {
    let gw = MockGateway::default();
    let id = generate_correlation_id();
    let resp = gw.execute(&refund_request(10_000), id).await.unwrap();
    assert_eq!(resp["id"], format!("rfnd_mock_{id}"));
    assert_eq!(resp["status"], "processed");
}

#[test]
fn webhook_signature_round_trip_and_rejection() {
    let body = br#"{"event":"refund.processed","payload":{}}"#;
    let secret = "whsec_live_or_test";
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let good = hex::encode(mac.finalize().into_bytes());
    assert!(verify_webhook_signature(body, &good, secret));
    // case-insensitive hex comparison
    assert!(verify_webhook_signature(body, &good.to_uppercase(), secret));
    assert!(!verify_webhook_signature(body, &format!("{}0", &good[..63]), secret));
}
