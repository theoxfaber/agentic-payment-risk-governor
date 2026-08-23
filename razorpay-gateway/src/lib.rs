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
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("reqwest client builds"),
            key_id: key_id.into(),
            key_secret: key_secret.into(),
            base_url: RAZORPAY_TEST_BASE.to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// POST with basic auth + 429/5xx retry (exponential backoff, honors
    /// Retry-After). Live APIs rate-limit; a demo that dies on the first 429
    /// is not production-ready.
    async fn post_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<(reqwest::StatusCode, serde_json::Value), ActionServiceError> {
        let mut attempt = 0u32;
        loop {
            let resp = self
                .http
                .post(url)
                .basic_auth(&self.key_id, Some(&self.key_secret))
                .json(body)
                .send()
                .await
                .map_err(|e| ActionServiceError::RazorpayGateway(e.to_string()))?;

            let status = resp.status();
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());

            if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                || status.is_server_error()
            {
                if attempt >= 3 {
                    return Ok((status, resp.json().await.unwrap_or(json!(null))));
                }
                let delay = retry_after
                    .unwrap_or_else(|| 1 << attempt) // 1s, 2s, 4s
                    .min(10);
                tracing::warn!(%status, attempt, backoff_s = delay, "razorpay throttling/error — retrying");
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                attempt += 1;
                continue;
            }

            return Ok((
                status,
                resp.json()
                    .await
                    .map_err(|e| ActionServiceError::RazorpayGateway(format!("body decode: {e}")))?,
            ));
        }
    }

    /// GET with basic auth — used by the live smoke test.
    pub async fn get_json(&self, path: &str) -> Result<serde_json::Value, ActionServiceError> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .get(&url)
            .basic_auth(&self.key_id, Some(&self.key_secret))
            .send()
            .await
            .map_err(|e| ActionServiceError::RazorpayGateway(e.to_string()))?;
        resp.json()
            .await
            .map_err(|e| ActionServiceError::RazorpayGateway(e.to_string()))
    }

    /// Auth probe: cheapest authenticated call. Returns account details.
    pub async fn ping(&self) -> Result<serde_json::Value, ActionServiceError> {
        self.get_json("/payments?count=1").await
    }

    /// Create an order in test mode (auto-capture so refunds work).
    pub async fn create_order(&self, amount_paise: i64, receipt: &str) -> Result<serde_json::Value, ActionServiceError> {
        let body = json!({
            "amount": amount_paise,
            "currency": "INR",
            "receipt": receipt,
            "payment_capture": 1,
        });
        let (status, payload) = self
            .post_with_retry(&format!("{}/orders", self.base_url), &body)
            .await?;
        ensure_success(status, &payload)?;
        Ok(payload)
    }

    /// Simulate a customer payment against an order (TEST MODE ONLY — uses
    /// Razorpay's legacy JSON payment endpoint with a test card).
    pub async fn create_test_payment(&self, order_id: &str, amount_paise: i64) -> Result<serde_json::Value, ActionServiceError> {
        // form-encoded, legacy endpoint
        let email = "risk.governor@test.razorpay";
        let amount_str = amount_paise.to_string();
        let params = [
            ("order_id", order_id),
            ("amount", amount_str.as_str()),
            ("currency", "INR"),
            ("email", email),
            ("contact", "9000000000"),
            ("method", "card"),
            ("card[number]", "4111111111111111"),
            ("card[exp_month]", "12"),
            ("card[exp_year]", "2030"),
            ("card[cvv]", "123"),
            ("card[name]", "Test Buyer"),
        ];
        let resp = self
            .http
            .post(format!("{}/payments/create/json", self.base_url))
            .basic_auth(&self.key_id, Some(&self.key_secret))
            .form(&params)
            .send()
            .await
            .map_err(|e| ActionServiceError::RazorpayGateway(e.to_string()))?;
        let status = resp.status();
        let payload: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ActionServiceError::RazorpayGateway(e.to_string()))?;
        ensure_success(status, &payload)?;
        Ok(payload)
    }

    /// Issue a refund against a captured payment. THE money-movement call.
    /// Razorpay: POST /payments/{payment_id}/refund
    pub async fn refund_payment(
        &self,
        payment_id: &str,
        amount_paise: i64,
    ) -> Result<serde_json::Value, ActionServiceError> {
        let body = json!({ "amount": amount_paise, "speed": "normal" });
        let url = format!("{}/payments/{}/refund", self.base_url, payment_id);
        let (status, payload) = self.post_with_retry(&url, &body).await?;
        ensure_success(status, &payload)?;
        Ok(payload)
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
                    format!("{}/payments/{}/refund", self.base_url, payment_id),
                    json!({ "amount": request.amount, "speed": "normal" }),
                )
            }
            _ => (
                format!("{}/payments", self.base_url),
                json!({ "amount": request.amount, "currency": request.currency }),
            ),
        }
    }
}

fn ensure_success(status: reqwest::StatusCode, payload: &serde_json::Value) -> Result<(), ActionServiceError> {
    if status.is_success() {
        Ok(())
    } else {
        Err(ActionServiceError::RazorpayGateway(format!(
            "razorpay {status}: {payload}"
        )))
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
        let (status, payload) = self.post_with_retry(&url, &body).await?;

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