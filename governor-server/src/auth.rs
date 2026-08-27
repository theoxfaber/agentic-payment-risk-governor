//! API-key authentication for every `/v1/*` route.

use axum::extract::{Request, State};
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;
use uuid::Uuid;

use crate::state::AppState;
use subtle::ConstantTimeEq;

/// A governor that executes money movement with NO authentication would be
/// the exact "valid credentials ≠ valid action" gap it exists to close. Every
/// /v1/* route requires the key; /health and /metrics stay open (liveness +
/// Prometheus scrape). The dashboard is unauthenticated and carries no secret.
pub(crate) fn resolve_api_key() -> String {
    let from_env = std::env::var("GOVERNOR_API_KEY").ok().filter(|k| !k.trim().is_empty());
    let key = api_key_from(from_env.clone());
    if from_env.is_none() {
        tracing::warn!("GOVERNOR_API_KEY not set — generated EPHEMERAL key for this run (redacted). Set GOVERNOR_API_KEY to pin a stable key across restarts");
    }
    key
}

/// Pure decision logic — testable without touching process-global env state
/// (env mutation is unsound under Rust's multithreaded test runners).
fn api_key_from(env_value: Option<String>) -> String {
    match env_value {
        Some(k) if !k.trim().is_empty() => k,
        _ => format!("rgov_{}", Uuid::new_v4()),
    }
}

/// Constant-time comparison via `subtle`. Avoids early return on length
/// mismatch which leaks key length through timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).unwrap_u8() == 1
}

fn authorized(headers: &axum::http::HeaderMap, expected: &str) -> bool {
    let from_api_key = headers.get("x-api-key").and_then(|v| v.to_str().ok());
    let from_bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));
    match from_api_key.or(from_bearer) {
        Some(provided) => constant_time_eq(provided.as_bytes(), expected.as_bytes()),
        None => false,
    }
}

pub(crate) async fn require_api_key(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if !authorized(req.headers(), &state.api_key) {
        return (
            axum::http::StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "missing or invalid API key — send X-API-Key (or Authorization: Bearer)"
            })),
        )
            .into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_only_equal_inputs() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn env_key_wins_and_ephemeral_is_prefixed() {
        // Pure-function path: no process-global env mutation (unsound under
        // multithreaded test runners).
        assert_eq!(api_key_from(Some("pinned-key".into())), "pinned-key");
        assert!(
            api_key_from(Some("   ".into())).starts_with("rgov_"),
            "blank key must fall back to ephemeral"
        );
        assert!(api_key_from(None).starts_with("rgov_"));
        assert_ne!(
            api_key_from(None),
            api_key_from(None),
            "ephemeral keys must be unique per call"
        );
    }
}
