//! Unified Risk Governor API server.
//!
//! One axum binary exposing the full pipeline as an HTTP API:
//!   POST /v1/actions              → submit an agent action, get ALLOW/REVIEW/BLOCK
//!   GET  /v1/decisions            → decision stream
//!   GET  /v1/decisions/{id}       → full replay (decision + complete audit trail)
//!   POST /v1/decisions/{id}/approve → human review queue resolution
//!   GET  /health                  → liveness
//!
//! Module map:
//!   auth      — API-key middleware for every /v1/* route
//!   backends  — in-memory vs Postgres / mock vs live gateway selection
//!   bootstrap — demo graph + seeded entities + shared test support
//!   routes    — HTTP handlers (submit, list, replay, approve, metrics)
//!   state     — AppState + Prometheus counters
//!
//! Gateway selection: RAZORPAY_KEY_ID + RAZORPAY_KEY_SECRET set → live
//! test-mode HTTP calls; otherwise MockGateway (records intent, moves no
//! money). The decision path is identical either way.

mod auth;
mod backends;
mod bootstrap;
mod learned;
mod routes;
mod state;

use auth::resolve_api_key;
use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use backends::{AuditBackend, EvidenceBackend};
use bootstrap::{default_graph_and_behaviors, seed_demo_entities};
use investigation_engine::{Baseline, GraphInvestigator};
use pg_store::PgStore;
use risk_governor_types::*;
use state::{AppState, Metrics};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// The full router: public routes (dashboard/health/metrics) plus the
/// /v1/* decision surface behind the auth middleware. Split out so tests
/// exercise the exact production stack.
fn build_router(state: Arc<AppState>) -> Router {
    let protected = Router::new()
        .route("/v1/actions", post(routes::submit_action))
        .route("/v1/decisions", get(routes::list_decisions))
        .route("/v1/decisions/:id", get(routes::replay_decision))
        .route("/v1/decisions/:id/approve", post(routes::approve_decision))
        .route("/v1/audit/verify", get(routes::verify_audit_chain))
        .route("/v1/audit/anchor", get(routes::audit_anchor))
        .route("/v1/real/analysis", get(routes::real_analysis))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ));

    let assets =
        tower_http::services::ServeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../dashboard-v2/dist/assets"));
    Router::new()
        .route("/", get(routes::dashboard_page))
        .route("/dashboard", get(routes::dashboard_page))
        .route("/health", get(routes::health))
        .route("/metrics", get(routes::metrics))
        .route("/webhooks/razorpay", post(routes::razorpay_webhook))
        .nest_service("/assets", assets)
        .merge(protected)
        .layer(DefaultBodyLimit::max(64 * 1024))
        .with_state(state)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Structured JSON logs when RUST_LOG_FORMAT=json (production collectors);
    // human-readable single-line otherwise (local dev).
    let format_json = std::env::var("RUST_LOG_FORMAT").as_deref() == Ok("json");
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower=warn".into());
    if format_json {
        tracing_subscriber::fmt().json().with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    // Persistence backend: DATABASE_URL → Postgres (survives restarts);
    // unset → in-memory (dev/tests). Same pipeline either way.
    let pg = match std::env::var("DATABASE_URL") {
        Ok(url) if !url.is_empty() => {
            let store = Arc::new(PgStore::connect(&url).await?);
            tracing::info!("persistence: Postgres backend enabled");
            Some(store)
        }
        _ => {
            tracing::info!("persistence: in-memory backend (DATABASE_URL not set)");
            None
        }
    };

    let evidence_backend = match &pg {
        Some(s) => EvidenceBackend::Pg(s.clone()),
        None => EvidenceBackend::Mem(Arc::new(evidence_service::InMemoryEvidenceStore::new())),
    };
    // Demo seeding policy:
    //   - in-memory backend → seeded (local dev / demo ergonomics)
    //   - Postgres backend  → ONLY when SEED_DEMO=true|1. A production DB
    //     must not silently inherit demo agents and a default merchant
    //     policy with hardcoded limits.
    let seed_demo = match (&pg, std::env::var("SEED_DEMO").as_deref()) {
        (None, _) => true,
        (Some(_), Ok(v)) => matches!(v.trim(), "true" | "1"),
        (Some(_), Err(_)) => false,
    };
    if seed_demo {
        seed_demo_entities(&evidence_backend).await?;
    } else {
        tracing::info!("demo entity seeding skipped (Postgres backend, SEED_DEMO not set)");
    }

    // Optional extra reference entities from a seed file (same shape as the
    // evidence-worker's EVIDENCE_SEED) — only meaningful on the PG backend.
    if let (Some(s), Ok(path)) = (&pg, std::env::var("EVIDENCE_SEED")) {
        if !path.is_empty() {
            let raw = std::fs::read_to_string(&path)?;
            s.seed_from_json(&raw).await?;
        }
    }

    let audit_backend = match &pg {
        Some(s) => AuditBackend::Pg(s.clone()),
        None => AuditBackend::Mem(Arc::new(audit_service::InMemoryAuditStore::new())),
    };

    // Hydrate prior decisions so replay/review survive restarts.
    let decisions: HashMap<uuid::Uuid, Decision> = match &pg {
        Some(s) => s
            .all_decisions()
            .await?
            .into_iter()
            .map(|d| (d.decision_id, d))
            .collect(),
        None => HashMap::new(),
    };
    if !decisions.is_empty() {
        tracing::info!(count = decisions.len(), "hydrated prior decisions from Postgres");
    }

    let gateway = backends::pick_gateway();

    // Intent extraction: LLM-backed when LLM_API_KEY is configured (claims
    // are evidence only — see intent-engine); deterministic heuristic
    // otherwise. Either way the pipeline is identical.
    let risk = match intent_engine::LlmExtractor::from_env() {
        Some(llm) => {
            tracing::info!("intent extraction: LLM-backed (evidence-only, never the decision-maker)");
            risk_engine::RiskEngine::new("1.2.0-intent-llm".into()).with_intent_extractor(Arc::new(llm))
        }
        None => {
            tracing::info!("intent extraction: deterministic heuristic (set LLM_API_KEY for LLM-backed claims)");
            risk_engine::RiskEngine::new("1.2.0-intent-heuristic".into())
                .with_intent_extractor(Arc::new(intent_engine::HeuristicExtractor))
        }
    };

    let (graph, behaviors) = default_graph_and_behaviors();
    let investigator = GraphInvestigator::new(graph.clone(), behaviors.clone(), HashMap::new(), Baseline::default());

    let svc = Arc::new(
        action_service::ActionService::new(
            Arc::new(policy_engine::PolicyEngine::new()),
            Arc::new(risk),
            Arc::new(evidence_service::EvidenceService::new(Arc::new(evidence_backend))),
            Arc::new(audit_service::AuditService::new(Arc::new(audit_backend.clone()))),
            gateway.clone(),
        )
        .with_investigator(investigator.into_trait())
        .with_learned_scorer(Arc::new(action_service::learned::DefaultLearnedScorer::from_embedded())),
    );

    let anchor_key = std::env::var("AUDIT_SIGNING_KEY")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.into_bytes());
    if anchor_key.is_some() {
        tracing::info!("audit anchor: HMAC signing enabled (AUDIT_SIGNING_KEY set)");
    } else {
        tracing::info!("audit anchor: no AUDIT_SIGNING_KEY — chain is tamper-evident but not externally anchored; set AUDIT_SIGNING_KEY for HMAC-anchored verification");
    }
    let webhook_secret = std::env::var("WEBHOOK_SECRET").ok().filter(|s| !s.is_empty());
    if webhook_secret.is_some() {
        tracing::info!("webhook: Razorpay webhook verification enabled");
    } else {
        tracing::info!("webhook: no WEBHOOK_SECRET — /webhooks/razorpay will reject");
    }

    let state = Arc::new(AppState {
        svc,
        audit: Arc::new(audit_service::AuditService::new(Arc::new(audit_backend))),
        gateway,
        decisions: RwLock::new(decisions),
        metrics: Arc::new(Metrics::default()),
        pg,
        api_key: resolve_api_key(),
        anchor_key,
        webhook_secret,
        graph,
        behaviors,
    });

    let app = build_router(state);

    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
    let host = std::env::var("BIND_HOST").unwrap_or_else(|_| "127.0.0.1".into());
    let addr: std::net::SocketAddr = format!("{host}:{port}")
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid BIND_HOST {host}: {e}"))?;
    tracing::info!("risk governor listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// SIGTERM (containers/orchestrators) + ctrl_c (local) → clean drain.
async fn shutdown_signal() {
    let mut sigterm =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
    }
    tracing::info!("shutdown signal received — draining");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::test_support::{test_state, TEST_KEY};
    use axum::body::Body;
    use tower::ServiceExt;

    async fn request(app: axum::Router, method: &str, uri: &str, api_key: Option<&str>) -> axum::response::Response {
        let mut builder = axum::http::Request::builder().method(method).uri(uri);
        if let Some(key) = api_key {
            builder = builder.header("x-api-key", key);
        }
        app.oneshot(builder.body(Body::empty()).unwrap()).await.unwrap()
    }

    #[tokio::test]
    async fn v1_routes_reject_missing_and_wrong_keys() {
        let app = build_router(test_state().await);

        for uri in ["/v1/decisions"] {
            let resp = request(app.clone(), "GET", uri, None).await;
            assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED, "{uri} without key");
            let resp = request(app.clone(), "GET", uri, Some("wrong-key")).await;
            assert_eq!(resp.status(), axum::http::StatusCode::UNAUTHORIZED, "{uri} wrong key");
        }

        let body = serde_json::json!({
            "agent_id": "agent-trusted-01", "merchant_id": "merchant-001",
            "action_type": "refund", "amount": 50_000,
            "declared_intent": "refund order #1"
        });
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/actions")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::UNAUTHORIZED,
            "/v1/actions without key"
        );
    }

    #[tokio::test]
    async fn valid_key_passes_and_bearer_works() {
        let app = build_router(test_state().await);
        let resp = request(app.clone(), "GET", "/v1/decisions", Some(TEST_KEY)).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);

        let req = axum::http::Request::builder()
            .uri("/v1/decisions")
            .header("authorization", format!("Bearer {TEST_KEY}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::OK,
            "Authorization: Bearer accepted"
        );
    }

    #[tokio::test]
    async fn health_and_metrics_stay_open() {
        let app = build_router(test_state().await);
        let resp = request(app.clone(), "GET", "/health", None).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let resp = request(app, "GET", "/metrics", None).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }
}
