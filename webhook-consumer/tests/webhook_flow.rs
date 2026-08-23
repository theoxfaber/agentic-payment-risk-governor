//! End-to-end webhook flow over a real axum router: signature gate, JSON
//! validation, and the audit-trail feedback loop.

use audit_service::{AuditService, AuditStore, InMemoryAuditStore};
use hmac::{Hmac, Mac};
use razorpay_gateway::verify_webhook_signature;
use sha2::Sha256;
use std::sync::Arc;
use tower::ServiceExt;
use webhook_consumer::WebhookConsumer;

const SECRET: &str = "whsec_test_123";

fn signed_body(payload: &serde_json::Value) -> (axum::body::Bytes, String) {
    let body = serde_json::to_vec(payload).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(&body);
    (body.into(), hex::encode(mac.finalize().into_bytes()))
}

#[tokio::test]
async fn valid_signature_is_accepted_and_audited() {
    let store = Arc::new(InMemoryAuditStore::new());
    let app = WebhookConsumer::new(Arc::new(AuditService::new(store.clone())), SECRET).router();

    let decision_id = uuid::Uuid::new_v4();
    let payload = serde_json::json!({
        "event": "refund.processed",
        "payload": { "refund": { "entity": { "notes": { "decision_id": decision_id.to_string() } } } }
    });
    let (body, sig) = signed_body(&payload);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/webhooks/razorpay")
                .header("x-razorpay-signature", &sig)
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    // Feedback loop closed: the outcome landed in the audit trail, linked to
    // its decision.
    let trail = store.by_decision(decision_id).await.unwrap();
    assert_eq!(trail.len(), 1);
    assert_eq!(
        trail[0].event_type,
        risk_governor_types::AuditEventType::WebhookReceived
    );
}

#[tokio::test]
async fn tampered_signature_rejected_before_processing() {
    let store = Arc::new(InMemoryAuditStore::new());
    let app = WebhookConsumer::new(Arc::new(AuditService::new(store.clone())), SECRET).router();

    let (body, _) = signed_body(&serde_json::json!({ "event": "refund.processed" }));

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/webhooks/razorpay")
                .header("x-razorpay-signature", "deadbeef")
                .body(axum::body::Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
    assert!(store.all().await.unwrap().is_empty());
}

#[tokio::test]
async fn missing_signature_header_rejected() {
    let store = Arc::new(InMemoryAuditStore::new());
    let app = WebhookConsumer::new(Arc::new(AuditService::new(store.clone())), SECRET).router();

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/webhooks/razorpay")
                .body(axum::body::Body::from(r#"{"event":"x"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn malformed_json_after_valid_signature_is_bad_request() {
    let app = WebhookConsumer::new(Arc::new(AuditService::new(Arc::new(InMemoryAuditStore::new()))), SECRET).router();

    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(b"not json at all");
    let sig = hex::encode(mac.finalize().into_bytes());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/webhooks/razorpay")
                .header("x-razorpay-signature", sig)
                .body(axum::body::Body::from("not json at all"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[test]
fn signature_helper_agrees_with_gateway_verification() {
    let body = br#"{"event":"payment.captured"}"#;
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(body);
    let sig = hex::encode(mac.finalize().into_bytes());
    assert!(verify_webhook_signature(body, &sig, SECRET));
}
