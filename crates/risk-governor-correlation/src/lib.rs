//! Correlation ID threading: one crate, three carriers, single source of truth.
//!
//! 1. HTTP: `CorrelationLayer` reads or generates the ID per request, exposes it
//!    to handlers via `RequestCorrelation` extension, echoes it in the response,
//!    and wraps the whole request future in a tracing span carrying
//!    `correlation_id` so every log line inside is linked.
//! 2. Task-local: `current_correlation_id()` — programmatic access anywhere
//!    inside that scope (used by `Envelope::new`).
//! 3. Bus: `Envelope<T>` stamps every published NATS message; consumers call
//!    `scope_correlation(env.correlation_id, ...)` to restore identical context.
//!
//! Set together at the edges (HTTP entry, bus receive), never invented mid-pipeline.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Request, Response};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::task::{Context, Poll};
use thiserror::Error;
use tokio::task_local;
use tower::{Layer, Service};
use tracing::Instrument;
use uuid::Uuid;

pub const CORRELATION_HEADER: &str = "x-correlation-id";

static CORRELATION_HEADER_NAME: HeaderName = HeaderName::from_static("x-correlation-id");

task_local! {
    static CORRELATION_ID: Uuid;
}

/// The correlation ID of the current HTTP request / bus message.
/// Falls back to a fresh v4 when called outside any scoped context
/// (e.g. startup jobs) so callers never have to handle None.
pub fn current_correlation_id() -> Uuid {
    CORRELATION_ID.try_with(|id| *id).unwrap_or_else(|_| Uuid::new_v4())
}

/// Run `fut` with both carriers set from one source: task-local + tracing span.
pub async fn scope_correlation<F>(correlation_id: Uuid, fut: F) -> F::Output
where
    F: std::future::Future,
{
    let span = tracing::info_span!("correlated", correlation_id = %correlation_id);
    CORRELATION_ID.scope(correlation_id, fut.instrument(span)).await
}

/// Typed extension so handlers can grab the ID without parsing headers:
/// `Extension<RequestCorrelation>(RequestCorrelation(id))`.
#[derive(Debug, Clone, Copy)]
pub struct RequestCorrelation(pub Uuid);

fn extract_or_generate(headers: &HeaderMap) -> Uuid {
    headers
        .get(&CORRELATION_HEADER_NAME)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .unwrap_or_else(Uuid::new_v4)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CorrelationLayer;

impl<S> Layer<S> for CorrelationLayer {
    type Service = CorrelationMiddleware<S>;
    fn layer(&self, inner: S) -> Self::Service {
        CorrelationMiddleware { inner }
    }
}

#[derive(Debug, Clone)]
pub struct CorrelationMiddleware<S> {
    inner: S,
}

impl<S> Service<Request<Body>> for CorrelationMiddleware<S>
where
    S: Service<Request<Body>, Response = Response<Body>, Error = Infallible> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let cid = extract_or_generate(req.headers());
        req.extensions_mut().insert(RequestCorrelation(cid));
        let response_value = HeaderValue::from_str(&cid.to_string()).expect("uuid is header-safe");

        // Ownership swap so the future is 'static without borrowing self.
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);

        Box::pin(async move {
            let mut res = scope_correlation(cid, inner.call(req)).await?;
            res.headers_mut()
                .insert(CORRELATION_HEADER_NAME.clone(), response_value);
            Ok(res)
        })
    }
}

/// Wire format for every NATS message. Bumping SCHEMA_VERSION invalidates
/// old messages instead of silently mis-decoding them mid-development.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub schema_version: u32,
    pub correlation_id: Uuid,
    /// Present on everything downstream of the decision combiner.
    pub decision_id: Option<Uuid>,
    pub subject: String,
    pub emitted_at: DateTime<Utc>,
    pub payload: T,
}

impl<T> Envelope<T> {
    pub fn new(subject: impl Into<String>, payload: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            correlation_id: current_correlation_id(),
            decision_id: None,
            subject: subject.into(),
            emitted_at: Utc::now(),
            payload,
        }
    }

    pub fn with_decision_id(mut self, decision_id: Uuid) -> Self {
        self.decision_id = Some(decision_id);
        self
    }

    pub fn encode(&self) -> Result<Vec<u8>, serde_json::Error>
    where
        T: Serialize,
    {
        serde_json::to_vec(self)
    }
}

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("json decode failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schema version {found}, expected {expected} — rebuild producers/consumers together")]
    SchemaMismatch { found: u32, expected: u32 },
}

#[derive(Debug, Deserialize)]
struct VersionProbe {
    schema_version: u32,
}

/// Decode and validate schema version. Returns the full envelope; caller
/// passes `envelope.payload` to its handler inside `scope_correlation`.
pub fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<Envelope<T>, DecodeError> {
    let probe: VersionProbe = serde_json::from_slice(bytes)?;
    if probe.schema_version != SCHEMA_VERSION {
        return Err(DecodeError::SchemaMismatch {
            found: probe.schema_version,
            expected: SCHEMA_VERSION,
        });
    }
    Ok(serde_json::from_slice(bytes)?)
}

/// JSON structured logs with span fields (incl. correlation_id) on every line.
pub fn init_tracing(default_filter: &str) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().json().flatten_event(true))
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn envelope_round_trip_preserves_ids() {
        let cid = Uuid::new_v4();
        let did = Uuid::new_v4();

        let env = scope_correlation(cid, async {
            Envelope::new("agent.action.requested", vec!["paise".to_string()]).with_decision_id(did)
        })
        .await;

        assert_eq!(env.correlation_id, cid, "must inherit from scope, not invent");
        let bytes = env.encode().unwrap();
        let back: Envelope<Vec<String>> = decode(&bytes).unwrap();
        assert_eq!(back.correlation_id, cid);
        assert_eq!(back.decision_id, Some(did));
        assert_eq!(back.subject, "agent.action.requested");
        assert_eq!(back.schema_version, 1);
    }

    #[test]
    fn decode_rejects_wrong_schema_version() {
        let bad = br#"{"schema_version": 999, "correlation_id": "00000000-0000-0000-0000-000000000000"}"#;
        let err = decode::<serde_json::Value>(bad).unwrap_err();
        assert!(matches!(err, DecodeError::SchemaMismatch { found: 999, .. }));
    }

    #[tokio::test]
    async fn outside_any_scope_falls_back_to_fresh_id() {
        // No panic, returns parseable uuid distinct across calls
        let a = current_correlation_id();
        let b = current_correlation_id();
        assert_ne!(a, b);
    }
}
