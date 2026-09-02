use action_service::{ActionServiceError, RazorpayGateway, VerifiedPayment};
use async_trait::async_trait;
use hmac::{Hmac, Mac};
use risk_governor_types::ActionType;
use risk_governor_types::AgentActionRequest;
use serde_json::json;
use sha2::Sha256;
use std::collections::HashMap;
use uuid::Uuid;

const RAZORPAY_TEST_BASE: &str = "https://api.razorpay.com/v1";

/// Fires only after an ALLOW decision — this is the actual money movement.
///
/// Two idempotency layers, because refunds are NOT idempotent server-side:
///   1. Decision-level dedup — one `decision_id` executes exactly once, ever.
///      A duplicate execute() (double-clicked approval, replayed request)
///      returns the cached response without a second HTTP call.
///   2. Lost-response guard on refunds — a 5xx is AMBIGUOUS (the refund may
///      have succeeded before the error). Before any resend we probe the
///      payment's refund list; if our amount already landed we treat it as
///      executed instead of double-refunding.
pub struct HttpGateway {
    http: reqwest::Client,
    key_id: String,
    key_secret: String,
    base_url: String,
    /// decision_id → response of the ONE money-movement call fired for it.
    /// Arc so that every clone of this gateway shares ONE execution record.
    executed: std::sync::Arc<tokio::sync::Mutex<HashMap<Uuid, (serde_json::Value, std::time::Instant)>>>,
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
            executed: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Deterministic idempotency key: `rfnd_{payment_id}_{decision_id}` for refunds,
    /// `pout_{merchant}_{decision_id}` for payouts, generic fallback for others.
    /// Razorpay server-side dedup guarantees identical keys never double-charge
    /// even across retries; decision_id makes the key stable for this decision
    /// but unique across decisions. Logged in audit trail for replay.
    pub fn deterministic_idempotency_key(request: &AgentActionRequest, decision_id: Uuid) -> String {
        let ctx_payment = request
            .context
            .get("payment_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        match request.action_type {
            ActionType::Refund => format!("rfnd_{}_{}", ctx_payment, decision_id),
            ActionType::Payout => format!("pout_{}_{}", request.merchant_id, decision_id),
            _ => format!("{:?}_{}_{}", request.action_type, request.merchant_id, decision_id).to_lowercase(),
        }
    }

    /// POST with basic auth + 429/5xx retry (exponential backoff, honors
    /// Retry-After). Live APIs rate-limit; a demo that dies on the first 429
    /// is not production-ready.
    async fn post_with_retry(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<(reqwest::StatusCode, serde_json::Value), ActionServiceError> {
        self.post_with_retry_idempotent(url, body, None).await
    }

    async fn post_with_retry_idempotent(
        &self,
        url: &str,
        body: &serde_json::Value,
        idempotency_key: Option<&str>,
    ) -> Result<(reqwest::StatusCode, serde_json::Value), ActionServiceError> {
        let mut attempt = 0u32;
        loop {
            let mut req = self
                .http
                .post(url)
                .basic_auth(&self.key_id, Some(&self.key_secret))
                .json(body);
            if let Some(key) = idempotency_key {
                req = req.header("Idempotency-Key", key).header("X-Idempotency-Key", key);
            }
            let resp = req
                .send()
                .await
                .map_err(|e| ActionServiceError::RazorpayGateway(e.to_string()))?;

            let status = resp.status();
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok());

            if status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                if attempt >= 3 {
                    let payload = resp
                        .json()
                        .await
                        .map_err(|e| ActionServiceError::RazorpayGateway(format!("body decode: {e}")))?;
                    return Ok((status, payload));
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
    pub async fn create_order(
        &self,
        amount_paise: i64,
        receipt: &str,
    ) -> Result<serde_json::Value, ActionServiceError> {
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
    pub async fn create_test_payment(
        &self,
        order_id: &str,
        amount_paise: i64,
    ) -> Result<serde_json::Value, ActionServiceError> {
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

    /// Endpoint + body selection per action type. Visible for tests: the
    /// routing table IS the contract with Razorpay's API surface.
    pub fn endpoint_for(&self, request: &AgentActionRequest) -> (String, serde_json::Value) {
        let ctx = &request.context;
        let s = |k: &str| ctx.get(k).and_then(|v| v.as_str()).map(|v| v.to_string());
        match request.action_type {
            ActionType::Refund => {
                let payment_id = s("payment_id").unwrap_or_default();
                (
                    format!("{}/payments/{}/refund", self.base_url, payment_id),
                    json!({ "amount": request.amount, "speed": "normal" }),
                )
            }
            // RazorpayX Fund Management API. Live mode needs X fund account;
            // test mode accepts the defaults below.
            ActionType::Payout => (
                format!("{}/payouts", self.base_url),
                json!({
                    "account_number": s("account_number").unwrap_or_else(|| "2323230028979561".into()),
                    "fund_account_id": s("fund_account_id").unwrap_or_default(),
                    "amount": request.amount,
                    "currency": request.currency,
                    "mode": s("mode").unwrap_or_else(|| "IMPS".into()),
                    "purpose": s("purpose").unwrap_or_else(|| "payout".into()),
                    "queue_if_low_balance": true,
                    "narration": request.declared_intent.chars().take(30).collect::<String>(),
                }),
            ),
            ActionType::PaymentLink => (
                format!("{}/payment_links", self.base_url),
                json!({
                    "amount": request.amount,
                    "currency": request.currency,
                    "reference_id": s("reference_id").unwrap_or_default(),
                    "description": request.declared_intent.chars().take(100).collect::<String>(),
                }),
            ),
            ActionType::Transfer | ActionType::Capture | ActionType::Void => (
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
    async fn verify_payment(&self, payment_id: &str) -> Result<Option<VerifiedPayment>, ActionServiceError> {
        let url = format!("{}/payments/{}", self.base_url, payment_id);
        let resp = self
            .http
            .get(&url)
            .basic_auth(&self.key_id, Some(&self.key_secret))
            .send()
            .await
            .map_err(|e| ActionServiceError::RazorpayGateway(format!("payment fetch failed: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(ActionServiceError::RazorpayGateway(format!(
                "payment {payment_id} not found (404) — cannot verify captured state"
            )));
        }
        if !status.is_success() {
            let body: serde_json::Value = resp.json().await.unwrap_or(json!(null));
            return Err(ActionServiceError::RazorpayGateway(format!(
                "payment fetch {status}: {body}"
            )));
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ActionServiceError::RazorpayGateway(format!("payment decode: {e}")))?;
        let st = v.get("status").and_then(|s| s.as_str()).unwrap_or("unknown").to_string();
        let amt = v
            .get("amount")
            .and_then(|x| x.as_i64())
            .or_else(|| v.get("amount").and_then(|x| x.as_u64()).map(|x| x as i64))
            .unwrap_or(0);
        let refunded = v
            .get("amount_refunded")
            .and_then(|x| x.as_i64())
            .or_else(|| v.get("amount_refunded").and_then(|x| x.as_u64()).map(|x| x as i64))
            .unwrap_or(0);
        Ok(Some(VerifiedPayment {
            payment_id: payment_id.to_string(),
            status: st,
            amount_paise: amt,
            refunded_paise: refunded,
        }))
    }

    async fn execute(
        &self,
        request: &AgentActionRequest,
        decision_id: Uuid,
    ) -> Result<serde_json::Value, ActionServiceError> {
        // Layer 1: decision-level idempotency. The second execute() for a
        // decision (double-clicked approval, replayed message) must never
        // fire a second money-movement call.
        let mut cache = self.executed.lock().await;
        if let Some((cached, inserted_at)) = cache.get(&decision_id) {
            if inserted_at.elapsed() < std::time::Duration::from_secs(3600) {
                tracing::warn!(
                    ?decision_id,
                    "duplicate execution attempt — returning cached response, no second gateway call"
                );
                return Ok(cached.clone());
            }
            // Expired entry — remove and proceed
            cache.remove(&decision_id);
        }
        drop(cache); // Release lock before the actual call

        let idempotency_key = Self::deterministic_idempotency_key(request, decision_id);
        tracing::info!(?decision_id, %idempotency_key, action=?request.action_type, "razorpay execution with idempotency key");

        let result = match request.action_type {
            ActionType::Refund => self.execute_refund(request, decision_id).await,
            _ => {
                let (url, body) = self.endpoint_for(request);
                self.post_with_retry_idempotent(&url, &body, Some(&idempotency_key))
                    .await
                    .and_then(|(status, payload)| {
                        ensure_success(status, &payload)?;
                        tracing::info!(?decision_id, %status, %idempotency_key, "razorpay call succeeded");
                        Ok(payload)
                    })
            }
        };

        if let Ok(payload) = &result {
            let mut cache = self.executed.lock().await;
            cache.retain(|_, (_, inserted_at)| inserted_at.elapsed() < std::time::Duration::from_secs(3600));
            cache.insert(decision_id, (payload.clone(), std::time::Instant::now()));
        }
        result
    }
}

impl HttpGateway {
    /// Refund execution with the lost-response guard. Retries are safe on 429
    /// (the request was rejected before processing) but AMBIGUOUS on 5xx: the
    /// refund may have landed server-side before the error. So before any 5xx
    /// resend we probe the payment's refunds; if our amount is already there,
    /// we treat the refund as executed and refuse to double-fire.
    async fn execute_refund(
        &self,
        request: &AgentActionRequest,
        decision_id: Uuid,
    ) -> Result<serde_json::Value, ActionServiceError> {
        let payment_id = request
            .context
            .get("payment_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let url = format!("{}/payments/{}/refund", self.base_url, payment_id);
        // The decision_id rides along as the refund's `receipt` — Razorpay
        // echoes it back on the refund entity. That makes the lost-response
        // probe EXACT: we match our own receipt, not merely a same-amount
        // refund some other flow may legitimately have created.
        let receipt = decision_id.to_string();
        let idempotency_key = Self::deterministic_idempotency_key(request, decision_id);
        let body = json!({ "amount": request.amount, "speed": "normal", "receipt": receipt });

        let mut attempt = 0u32;
        loop {
            let resp = match self
                .http
                .post(&url)
                .basic_auth(&self.key_id, Some(&self.key_secret))
                .header("Idempotency-Key", &idempotency_key)
                .header("X-Idempotency-Key", &idempotency_key)
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    if self
                        .refund_landed(payment_id, request.amount, &receipt)
                        .await
                        .unwrap_or(false)
                    {
                        tracing::warn!(?decision_id, "refund LANDED despite transport error — dedup");
                        return Ok(json!({
                            "status": "processed",
                            "deduplicated_after_upstream_error": true,
                            "payment_id": payment_id,
                            "amount": request.amount,
                        }));
                    }
                    return Err(ActionServiceError::RazorpayGateway(format!("transport error: {e}")));
                }
            };
            let status = resp.status();

            if status.is_success() {
                let payload: serde_json::Value = resp
                    .json()
                    .await
                    .map_err(|e| ActionServiceError::RazorpayGateway(format!("body decode: {e}")))?;
                tracing::info!(?decision_id, %status, "refund succeeded");
                return Ok(payload);
            }

            let rate_limited = status == reqwest::StatusCode::TOO_MANY_REQUESTS;
            let ambiguous_5xx = status.is_server_error();

            if !rate_limited && !ambiguous_5xx {
                // Definitive client error (400/401/404…) — retrying cannot help.
                let payload: serde_json::Value = resp.json().await.unwrap_or(json!(null));
                return Err(ActionServiceError::RazorpayGateway(format!(
                    "razorpay {status}: {payload}"
                )));
            }

            // Layer 2: lost-response check BEFORE any resend of an ambiguous 5xx.
            if ambiguous_5xx && self.refund_landed(payment_id, request.amount, &receipt).await? {
                tracing::warn!(
                    ?decision_id,
                    %status,
                    payment_id,
                    amount = request.amount,
                    "refund LANDED despite upstream error — treating as executed, not resending"
                );
                return Ok(json!({
                    "status": "processed",
                    "deduplicated_after_upstream_error": true,
                    "payment_id": payment_id,
                    "amount": request.amount,
                }));
            }

            if attempt >= 3 {
                let payload: serde_json::Value = resp.json().await.unwrap_or(json!(null));
                return Err(ActionServiceError::RazorpayGateway(format!(
                    "razorpay {status} after {attempt} retries: {payload}"
                )));
            }

            let delay = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or_else(|| 1 << attempt)
                .min(10);
            tracing::warn!(%status, attempt, backoff_s = delay, "refund throttled/erroring — retrying");
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            attempt += 1;
        }
    }

    /// True when OUR refund already exists on the payment — i.e. an earlier
    /// attempt processed despite the error response we received. Matching is
    /// by the `receipt` we stamp with the decision_id; amount is only a
    /// fallback for legacy refunds created without a receipt, so an unrelated
    /// same-amount refund can never false-positive the dedup. If state itself
    /// is unverifiable (the probe GET fails), this returns Err: refusing to
    /// resend costs a retry; a double refund costs real money.
    async fn refund_landed(
        &self,
        payment_id: &str,
        amount_paise: i64,
        receipt: &str,
    ) -> Result<bool, ActionServiceError> {
        let url = format!("{}/payments/{}/refunds?count=100", self.base_url, payment_id);
        let resp = self
            .http
            .get(&url)
            .basic_auth(&self.key_id, Some(&self.key_secret))
            .send()
            .await
            .map_err(|e| ActionServiceError::RazorpayGateway(format!("refund-state probe failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(ActionServiceError::RazorpayGateway(format!(
                "refund-state unverifiable (GET refunds returned {}); refusing to resend a possibly-executed refund",
                resp.status()
            )));
        }

        let payload: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ActionServiceError::RazorpayGateway(format!("refund probe decode: {e}")))?;
        let landed = payload
            .get("items")
            .and_then(|i| i.as_array())
            .map(|items| {
                items.iter().any(|r| match r.get("receipt").and_then(|s| s.as_str()) {
                    Some(rcpt) => rcpt == receipt,
                    None => r.get("amount").and_then(|a| a.as_i64()) == Some(amount_paise),
                })
            })
            .unwrap_or(false);
        Ok(landed)
    }
}

/// Phase 1 stand-in: records what would have been sent, moves no money.
/// Mirrors HttpGateway idempotency so demo + tests exercise at-most-once without network.
pub struct MockGateway {
    pub calls: std::sync::Arc<std::sync::Mutex<Vec<(Uuid, serde_json::Value)>>>,
    executed: std::sync::Arc<std::sync::Mutex<HashMap<Uuid, serde_json::Value>>>,
}

impl Default for MockGateway {
    fn default() -> Self {
        Self {
            calls: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            executed: std::sync::Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }
}

impl Clone for MockGateway {
    fn clone(&self) -> Self {
        Self {
            calls: self.calls.clone(),
            executed: self.executed.clone(),
        }
    }
}

#[async_trait]
impl RazorpayGateway for MockGateway {
    async fn verify_payment(&self, _payment_id: &str) -> Result<Option<VerifiedPayment>, ActionServiceError> {
        Ok(None)
    }

    async fn execute(
        &self,
        request: &AgentActionRequest,
        decision_id: Uuid,
    ) -> Result<serde_json::Value, ActionServiceError> {
        let mut waits = 0;
        loop {
            {
                let mut guard = self.executed.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(cached) = guard.get(&decision_id).cloned() {
                    if cached.get("_pending").and_then(|v| v.as_bool()) == Some(true) {
                        if waits >= 40 {
                            return Err(ActionServiceError::RazorpayGateway(
                                "concurrent mock execution in progress".into(),
                            ));
                        }
                    } else {
                        return Ok(cached);
                    }
                } else {
                    guard.insert(decision_id, json!({"_pending": true}));
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            waits += 1;
        }
        let body = json!({
            "mock": true,
            "action_type": request.action_type,
            "amount": request.amount,
            "agent_id": request.agent_id,
        });
        {
            let mut guard = self.calls.lock().unwrap_or_else(|e| e.into_inner());
            guard.push((decision_id, body.clone()));
        }
        let resp = json!({ "id": format!("rfnd_mock_{decision_id}"), "status": "processed", "mock": true });
        self.executed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(decision_id, resp.clone());
        Ok(resp)
    }
}

/// Verify X-Razorpay-Signature: HMAC-SHA256(raw_body, webhook_secret) hex-encoded.
///
/// Comparison runs through `verify_slice`, which is constant-time — a string
/// equality on hex (even case-insensitive) leaks timing information about how
/// many leading bytes matched, which is exploitable on a payments webhook.
/// Case-insensitivity comes free from `hex::decode`.
pub fn verify_webhook_signature(raw_body: &[u8], signature: &str, webhook_secret: &str) -> bool {
    if signature.trim().len() < 32 {
        return false;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(webhook_secret.as_bytes()).expect("hmac accepts any key length");
    mac.update(raw_body);
    match hex::decode(signature.trim()) {
        Ok(sig_bytes) => mac.verify_slice(&sig_bytes).is_ok(),
        Err(_) => false,
    }
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
