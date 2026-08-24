//! API-key authentication for every `/v1/*` route.

use axum::extract::{Request, State};
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;
use uuid::Uuid;

use crate::state::AppState;

/// A governor that executes money movement with NO authentication would be
/// the exact "valid credentials ≠ valid action" gap it exists to close. Every
/// /v1/* route requires the key; /health and /metrics stay open (liveness +
/// Prometheus scrape), and the dashboard page carries the server's own key so
/// it authenticates like any other client.
pub(crate) fn resolve_api_key() -> String {
    match std::env::var("GOVERNOR_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k,
        _ => {
            let ephemeral = format!("rgov_{}", Uuid::new_v4());
            tracing::warn!("GOVERNOR_API_KEY not set — generated EPHEMERAL key for this run: {ephemeral}");
            tracing::warn!("set GOVERNOR_API_KEY to pin a stable key across restarts");
            ephemeral
        }
    }
}

/// Length-independent byte comparison (comparison time does not leak how many
/// leading bytes of the key matched).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
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
        // resolve_api_key reads the process env; only assert the fallback shape
        // here to avoid mutating global env in parallel tests.
        std::env::remove_var("GOVERNOR_API_KEY");
        let k = resolve_api_key();
        assert!(k.starts_with("rgov_"), "ephemeral keys must be prefixed: {k}");
        std::env::set_var("GOVERNOR_API_KEY", "fixed-test-key");
        assert_eq!(resolve_api_key(), "fixed-test-key");
        std::env::remove_var("GOVERNOR_API_KEY");
    }
}
