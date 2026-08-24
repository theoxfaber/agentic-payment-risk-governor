//! AI-assisted declared-intent understanding.
//!
//! Agents declare what an action is FOR ("refund for order #123"). This
//! crate turns that free text into structured claims — action type, amount,
//! order reference, urgency language — that the risk engine can CHECK against
//! the request's hard fields. A claim that contradicts the request is
//! evidence of deception or compromise; a missing claim is weak evidence of
//! nothing at all.
//!
//! SAFETY PRINCIPLE (the reason this crate exists as a side-channel, not the
//! decision-maker): extraction output feeds RISK FEATURES only. It can raise
//! a score; it can never lower one below policy boundaries, and the combiner
//! still demands low risk AND no contradictions before ALLOW. An LLM is a
//! witness here, never a judge.

use async_trait::async_trait;
use risk_governor_types::ActionType;
use serde::{Deserialize, Serialize};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Claims model
// ---------------------------------------------------------------------------

/// Structured claims extracted from declared intent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentClaims {
    /// Action type the text implies, if any ("refund", "payout", …).
    pub action_type_hint: Option<String>,
    /// Amount the text claims the action is for, in paise.
    pub amount_paise: Option<i64>,
    /// Order/reference id mentioned in the text ("#123" → "123").
    pub order_ref: Option<String>,
    /// Urgency/pressure language — classic social-engineering marker.
    pub urgency_flags: Vec<String>,
    /// True when the primary extractor (LLM) failed and a deterministic
    /// fallback answered instead. Degraded claims are still evidence — they
    /// just carry less weight downstream.
    #[serde(default)]
    pub degraded: bool,
}

impl IntentClaims {
    fn empty() -> Self {
        Self {
            action_type_hint: None,
            amount_paise: None,
            order_ref: None,
            urgency_flags: vec![],
            degraded: false,
        }
    }
}

#[async_trait]
pub trait IntentExtractor: Send + Sync {
    async fn extract(&self, declared_intent: &str, action_type: ActionType, amount: i64) -> IntentClaims;
}

// ---------------------------------------------------------------------------
// Heuristic extractor — deterministic, offline, always available
// ---------------------------------------------------------------------------

#[derive(Default, Clone, Copy)]
pub struct HeuristicExtractor;

const URGENCY_WORDS: &[&str] = &[
    "urgent",
    "urgently",
    "immediate",
    "immediately",
    "asap",
    "emergency",
    "right now",
    "bypass",
    "override",
];

impl HeuristicExtractor {
    fn parse(text: &str) -> IntentClaims {
        let mut claims = IntentClaims::empty();
        let lowered = text.to_lowercase();

        // Order ref: "#123", "#ORD-99".
        for token in lowered.split_whitespace() {
            if let Some(rest) = token.strip_prefix('#') {
                let cleaned = rest.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
                if !cleaned.is_empty() {
                    claims.order_ref = Some(cleaned.to_string());
                    break;
                }
            }
        }

        // Amount mentions: ₹1500 / rs. 1,500 / inr 1500 / 1500 rupees /
        // 2500 paise. Comma group separators tolerated.
        let tokens: Vec<&str> = lowered.split_whitespace().collect();
        for (i, tok) in tokens.iter().enumerate() {
            let bare = tok.trim_matches(|c: char| !(c.is_ascii_digit() || c == ',' || c == '.'));
            // currency prefix on same token (₹1500, rs.1500) or previous token (rs 1500)
            let prefixed = tok.contains('₹')
                || tokens[..i]
                    .last()
                    .map(|p| matches!(*p, "rs" | "rs." | "inr"))
                    .unwrap_or(false)
                || tok.starts_with("rs.") && bare.len() < tok.len();
            // currency suffix (1500 rupees)
            let suffixed = tokens
                .get(i + 1)
                .map(|n| *n == "rupees" || *n == "rupee")
                .unwrap_or(false);
            if !(prefixed || suffixed) {
                continue;
            }
            if let Some(v) = parse_number(bare) {
                let unit_is_paise = tokens.get(i + 1).map(|n| *n == "paise").unwrap_or(false);
                claims.amount_paise = Some(if unit_is_paise { v } else { v * 100 });
                break;
            }
        }
        // explicit paise mention without currency prefix ("refund 5000 paise")
        if claims.amount_paise.is_none() {
            for (i, tok) in tokens.iter().enumerate() {
                if tokens.get(i + 1).map(|n| *n == "paise").unwrap_or(false) {
                    if let Some(v) = parse_number(tok) {
                        claims.amount_paise = Some(v);
                        break;
                    }
                }
            }
        }

        // Action type hint.
        for (word, kind) in [
            ("refund", "refund"),
            ("payout", "payout"),
            ("payment link", "payment_link"),
            ("transfer", "transfer"),
            ("capture", "capture"),
            ("void", "void"),
        ] {
            if lowered.contains(word) {
                claims.action_type_hint = Some(kind.to_string());
                break;
            }
        }

        for w in URGENCY_WORDS {
            if lowered.contains(w) {
                claims.urgency_flags.push(w.to_string());
            }
        }

        claims
    }
}

fn parse_number(token: &str) -> Option<i64> {
    let cleaned: String = token.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    cleaned
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|v| v as i64)
}

#[async_trait]
impl IntentExtractor for HeuristicExtractor {
    async fn extract(&self, declared_intent: &str, _action_type: ActionType, _amount: i64) -> IntentClaims {
        Self::parse(declared_intent)
    }
}

// ---------------------------------------------------------------------------
// LLM extractor — OpenAI-compatible chat completions, heuristic fallback
// ---------------------------------------------------------------------------

pub struct LlmExtractor {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

const SYSTEM_PROMPT: &str = r##"You extract structured financial-action claims from an AI agent's declared intent for a payment-risk system. Reply with ONLY minified JSON, no prose, no code fences:
{"action_type_hint":"refund|payout|payment_link|transfer|capture|void|null","amount_paise":<integer paise or null>,"order_ref":<string or null>,"urgency_flags":[<strings>]}
Rules: amounts are converted to paise (₹100 → 10000). order_ref is an order/payment/invoice reference like "123" from "#123". urgency_flags capture pressure language ("urgent", "bypass"). If unsure, use null/[] — do not invent."##;

impl LlmExtractor {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(4))
                .build()
                .expect("reqwest client builds"),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
        }
    }

    /// From env: LLM_API_KEY required; LLM_BASE_URL (default OpenAI),
    /// LLM_MODEL (default gpt-4o-mini). None when no key configured.
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("LLM_API_KEY").ok()?;
        if api_key.trim().is_empty() {
            return None;
        }
        Some(Self::new(
            std::env::var("LLM_BASE_URL").unwrap_or_else(|_| "https://api.openai.com/v1".into()),
            api_key,
            std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
        ))
    }

    fn parse_completion(body: &serde_json::Value) -> Result<IntentClaims, String> {
        let content = body
            .pointer("/choices/0/message/content")
            .and_then(|c| c.as_str())
            .ok_or_else(|| "no completion content".to_string())?;
        // Tolerate ```json fences some models emit despite instructions.
        let stripped = content
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();
        let parsed: IntentClaims = serde_json::from_str(stripped).map_err(|e| format!("claims decode: {e}"))?;
        Ok(parsed)
    }

    pub async fn extract_llm(
        &self,
        declared_intent: &str,
        action_type: ActionType,
        amount: i64,
    ) -> Result<IntentClaims, String> {
        let user_msg =
            format!("action_type: {action_type:?}\namount_paise: {amount}\ndeclared_intent: {declared_intent}");
        let resp = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "model": self.model,
                "temperature": 0,
                "messages": [
                    {"role": "system", "content": SYSTEM_PROMPT},
                    {"role": "user", "content": user_msg},
                ],
            }))
            .send()
            .await
            .map_err(|e| format!("llm request: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("llm status {}", resp.status()));
        }
        let body: serde_json::Value = resp.json().await.map_err(|e| format!("llm decode: {e}"))?;
        Self::parse_completion(&body)
    }
}

#[async_trait]
impl IntentExtractor for LlmExtractor {
    async fn extract(&self, declared_intent: &str, action_type: ActionType, amount: i64) -> IntentClaims {
        match tokio::time::timeout(
            Duration::from_secs(5),
            self.extract_llm(declared_intent, action_type, amount),
        )
        .await
        {
            Ok(Ok(mut claims)) => {
                claims.degraded = false;
                claims
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "LLM intent extraction failed — deterministic fallback");
                let mut c = HeuristicExtractor::parse(declared_intent);
                c.degraded = true;
                c
            }
            Err(_) => {
                tracing::warn!("LLM intent extraction timed out — deterministic fallback");
                let mut c = HeuristicExtractor::parse(declared_intent);
                c.degraded = true;
                c
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_extracts_order_ref_and_amount() {
        let c = HeuristicExtractor::parse("Refund ₹1,500 for order #123 please");
        assert_eq!(c.order_ref.as_deref(), Some("123"));
        assert_eq!(c.amount_paise, Some(150_000));
        assert_eq!(c.action_type_hint.as_deref(), Some("refund"));
        assert!(c.urgency_flags.is_empty());
    }

    #[test]
    fn heuristic_reads_rs_prefix_and_rupees_suffix() {
        assert_eq!(HeuristicExtractor::parse("rs 2000 refund").amount_paise, Some(200_000));
        assert_eq!(
            HeuristicExtractor::parse("payout of 300 rupees").amount_paise,
            Some(30_000)
        );
    }

    #[test]
    fn heuristic_respects_explicit_paise() {
        assert_eq!(
            HeuristicExtractor::parse("send 2500 paise back").amount_paise,
            Some(2_500)
        );
    }

    #[test]
    fn heuristic_flags_urgency_language() {
        let c = HeuristicExtractor::parse("URGENT refund bypass override asap");
        assert_eq!(c.urgency_flags.len(), 4);
        assert!(!c.degraded);
    }

    #[test]
    fn heuristic_empty_on_plain_text() {
        let c = HeuristicExtractor::parse("monthly reconciliation batch");
        assert_eq!(c, IntentClaims::empty());
    }

    // --- LLM path against a local OpenAI-compatible mock -----------------

    async fn spawn_mock_llm(status: axum::http::StatusCode, content: &str) -> String {
        let content_owned = content.to_string();
        let app = axum::Router::new().route(
            "/chat/completions",
            axum::routing::post(move || {
                let content = content_owned.clone();
                async move {
                    (
                        status,
                        axum::Json(serde_json::json!({
                            "choices": [{"message": {"role": "assistant", "content": content}}]
                        })),
                    )
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    const GOOD_JSON: &str =
        r#"{"action_type_hint":"refund","amount_paise":150000,"order_ref":"123","urgency_flags":["urgent"]}"#;

    #[tokio::test]
    async fn llm_extractor_parses_structured_claims() {
        let base = spawn_mock_llm(axum::http::StatusCode::OK, GOOD_JSON).await;
        let ext = LlmExtractor::new(base, "key", "test-model");
        let claims = ext
            .extract(
                "urgent refund of rupees one five zero zero for #123",
                ActionType::Refund,
                999,
            )
            .await;
        assert!(!claims.degraded);
        assert_eq!(claims.amount_paise, Some(150_000));
        assert_eq!(claims.order_ref.as_deref(), Some("123"));
        assert_eq!(claims.action_type_hint.as_deref(), Some("refund"));
        assert_eq!(claims.urgency_flags, vec!["urgent".to_string()]);
    }

    #[tokio::test]
    async fn llm_failure_falls_back_to_heuristic_marked_degraded() {
        let base = spawn_mock_llm(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "").await;
        let ext = LlmExtractor::new(base, "key", "test-model");
        let claims = ext
            .extract("urgent refund ₹800 for order #77", ActionType::Refund, 80_000)
            .await;
        assert!(claims.degraded, "fallback must be marked degraded");
        assert_eq!(claims.amount_paise, Some(80_000));
        assert_eq!(claims.order_ref.as_deref(), Some("77"));
        assert!(!claims.urgency_flags.is_empty());
    }

    #[test]
    fn completion_parser_tolerates_code_fences_and_prose_rejection() {
        let fenced = format!("```json\n{GOOD_JSON}\n```");
        let ok = LlmExtractor::parse_completion(&serde_json::json!({
            "choices": [{"message": {"content": fenced}}]
        }));
        assert!(ok.is_ok());

        let bad = LlmExtractor::parse_completion(&serde_json::json!({
            "choices": [{"message": {"content": "I cannot answer that"}}]
        }));
        assert!(bad.is_err());
    }
}
