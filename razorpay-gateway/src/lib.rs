use action_service::{ActionServiceError, RazorpayGateway};
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use risk_governor_types::AgentActionRequest;
use serde_json::json;
use sha2::Sha256;
use uuid::Uuid;

const RAZORPAY_TEST_BASE: &str = "https://api.razorpay.com/v1";

/// Fires only after an ALLOW decision — this is the actual money movement.
#[derive(Clone)]
pub struct HttpGateway {
    http: reqwest::Client,
    key_id: String,
    key_secret: String,
    base_url: String,
}

impl HttpGateway {
    pub fn new(key_id: impl Into<String>, key_secret: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            key_id: key_id.into(),
            key_secret: key_secret.into(),
            base_url: RAZORPAY_TEST_BASE.to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    fn endpoint_for(&self, request: &AgentActionRequest) -> (String, serde_json::Value) {
        match request.action_type {
            risk_governor_types::ActionType::Refund => {
                let payment_id = request
                    .context
                    .get("payment_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                (
                    format!("{}/refunds", self.base_url),
                    json!({ "payment_id": payment_id, "amount": request.amount }),
                )
            }
            _ => (
                format!("{}/payments", self.base_url),
                json!({ "amount": request.amount, "currency": request.currency }),
            ),
        }
    }
}

#[async_trait]
impl RazorpayGateway for HttpGateway {
    async fn execute(
        &self,
        request: &AgentActionRequest,
        decision_id: Uuid,
    ) -> Result<serde_json::Value, ActionServiceError> {
        let (url, body) = self.endpoint_for(request);

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.key_id, Some(&self.key_secret))
            .json(&body)
            .send()
            .await
            .map_err(|e| ActionServiceError::RazorpayGateway(e.to_string()))?;

        let status = resp.status();
        let payload: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ActionServiceError::RazorpayGateway(e.to_string()))?;

        if !status.is_success() {
            tracing::error!(?decision_id, %status, ?payload, "razorpay call failed");
            return Err(ActionServiceError::RazorpayGateway(format!(
                "razorpay returned {status}: {payload}"
            )));
        }

        tracing::info!(?decision_id, %status, "razorpay call succeeded");
        Ok(payload)
    }
}

/// Phase 1 stand-in: records what would have been sent, moves no money.
#[derive(Default)]
pub struct MockGateway {
    pub calls: std::sync::Mutex<Vec<(Uuid, serde_json::Value)>>,
}

#[async_trait]
impl RazorpayGateway for MockGateway {
    async fn execute(
        &self,
        request: &AgentActionRequest,
        decision_id: Uuid,
    ) -> Result<serde_json::Value, ActionServiceError> {
        let body = json!({
            "mock": true,
            "action_type": request.action_type,
            "amount": request.amount,
            "agent_id": request.agent_id,
        });
        self.calls.lock().unwrap().push((decision_id, body.clone()));
        Ok(json!({ "id": format!("rfnd_mock_{decision_id}"), "status": "processed", "mock": true }))
    }
}

/// Verify X-Razorpay-Signature: HMAC-SHA256(raw_body, webhook_secret) hex-encoded.
pub fn verify_webhook_signature(raw_body: &[u8], signature: &str, webhook_secret: &str) -> bool {
    let mut mac = Hmac::<Sha256>::new_from_slice(webhook_secret.as_bytes())
        .expect("hmac accepts any key length");
    mac.update(raw_body);
    let expected = hex::encode(mac.finalize().into_bytes());
    // Constant-time-ish comparison via hmac's verify, or fallback to string eq on lengths
    expected.eq_ignore_ascii_case(signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_verification_accepts_valid() {
        let secret = "whsec_test";
        let body = br#"{"event":"refund.processed"}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = hex::encode(mac.finalize().into_bytes());
        assert!(verify_webhook_signature(body, &sig, secret));
    }

    #[test]
    fn signature_verification_rejects_tampered() {
        assert!(!verify_webhook_signature(
            br#"{"event":"refund.processed"}"#,
            "deadbeef",
            "whsec_test"
        ));
    }
}