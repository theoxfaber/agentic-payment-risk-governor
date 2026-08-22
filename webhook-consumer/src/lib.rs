use audit_service::{AuditService, AuditStore};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::Router;
use razorpay_gateway::verify_webhook_signature;
use risk_governor_types::*;
use std::sync::Arc;
use uuid::Uuid;

/// Receives Razorpay webhooks (payment.captured, refund.processed, ...),
/// verifies signatures, publishes outcome events, closes the feedback loop.
pub struct WebhookConsumer<S: AuditStore> {
    audit: Arc<AuditService<S>>,
    webhook_secret: String,
}

impl<S: AuditStore + 'static> WebhookConsumer<S> {
    pub fn new(audit: Arc<AuditService<S>>, webhook_secret: impl Into<String>) -> Self {
        Self {
            audit,
            webhook_secret: webhook_secret.into(),
        }
    }

    pub fn router(self) -> Router {
        Router::new()
            .route("/webhooks/razorpay", post(Self::handle))
            .with_state(Arc::new(self))
    }

    async fn handle(
        State(state): State<Arc<Self>>,
        headers: HeaderMap,
        body: axum::body::Bytes,
    ) -> StatusCode {
        // Signature verification FIRST — reject before any processing.
        let signature = match headers.get("x-razorpay-signature").and_then(|v| v.to_str().ok()) {
            Some(sig) => sig,
            None => {
                tracing::warn!("webhook rejected: missing signature header");
                return StatusCode::UNAUTHORIZED;
            }
        };

        if !verify_webhook_signature(&body, signature, &state.webhook_secret) {
            tracing::warn!("webhook rejected: invalid signature");
            return StatusCode::UNAUTHORIZED;
        }

        let payload: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!("webhook rejected: malformed json: {e}");
                return StatusCode::BAD_REQUEST;
            }
        };

        let event = payload
            .get("event")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        tracing::info!(%event, "razorpay webhook accepted");

        state
            .audit
            .record(
                AuditEventType::WebhookReceived,
                payload
                    .pointer("/payload/refund/entity/notes/decision_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| Uuid::parse_str(s).ok()),
                payload,
            )
            .await;

        // Razorpay expects 2xx quickly; heavy processing happens over the bus in Phase 2.
        StatusCode::OK
    }
}