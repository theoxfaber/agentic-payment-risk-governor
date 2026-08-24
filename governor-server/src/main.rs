//! Unified Risk Governor API server.
//!
//! One axum binary exposing the full pipeline as an HTTP API:
//!   POST /v1/actions              → submit an agent action, get ALLOW/REVIEW/BLOCK
//!   GET  /v1/decisions            → decision stream
//!   GET  /v1/decisions/{id}       → full replay (decision + complete audit trail)
//!   POST /v1/decisions/{id}/approve → human review queue resolution
//!   GET  /health                  → liveness
//!
//! Gateway selection: RAZORPAY_KEY_ID + RAZORPAY_KEY_SECRET set → live
//! test-mode HTTP calls; otherwise MockGateway (records intent, moves no
//! money). The decision path is identical either way.

use action_service::{ActionService, ActionServiceError, RazorpayGateway};
use audit_service::{AuditError, AuditService, AuditStore, InMemoryAuditStore};
use evidence_service::{EvidenceError, EvidenceService, EvidenceStore, InMemoryEvidenceStore};
use investigation_engine::{Baseline, CustomerBehavior, GraphInvestigator};
use pg_store::PgStore;
use razorpay_gateway::{HttpGateway, MockGateway};
use risk_governor_types::*;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Store backends — in-memory (dev/tests) or Postgres (production), selected
// at boot by DATABASE_URL. Identical trait surface, identical wire format.
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum EvidenceBackend {
    Mem(Arc<InMemoryEvidenceStore>),
    Pg(Arc<PgStore>),
}

#[async_trait::async_trait]
impl EvidenceStore for EvidenceBackend {
    async fn agent_history(&self, agent_id: &str) -> Result<Option<AgentHistory>, EvidenceError> {
        match self {
            Self::Mem(s) => s.agent_history(agent_id).await,
            Self::Pg(s) => s.agent_history(agent_id).await,
        }
    }
    async fn merchant_policy(&self, merchant_id: &str) -> Result<Option<MerchantPolicy>, EvidenceError> {
        match self {
            Self::Mem(s) => s.merchant_policy(merchant_id).await,
            Self::Pg(s) => s.merchant_policy(merchant_id).await,
        }
    }
    async fn customer_history(&self, customer_id: &str) -> Result<Option<CustomerHistory>, EvidenceError> {
        match self {
            Self::Mem(s) => s.customer_history(customer_id).await,
            Self::Pg(s) => s.customer_history(customer_id).await,
        }
    }
    async fn record_action(&self, request: &AgentActionRequest) -> Result<(), EvidenceError> {
        match self {
            Self::Mem(s) => s.record_action(request).await,
            Self::Pg(s) => s.record_action(request).await,
        }
    }
    async fn velocity(&self, agent_id: &str) -> Result<VelocityStats, EvidenceError> {
        match self {
            Self::Mem(s) => s.velocity(agent_id).await,
            Self::Pg(s) => s.velocity(agent_id).await,
        }
    }
}

enum AuditBackend {
    Mem(Arc<InMemoryAuditStore>),
    Pg(Arc<PgStore>),
}

impl Clone for AuditBackend {
    fn clone(&self) -> Self {
        match self {
            Self::Mem(s) => Self::Mem(s.clone()),
            Self::Pg(s) => Self::Pg(s.clone()),
        }
    }
}

#[async_trait::async_trait]
impl AuditStore for AuditBackend {
    async fn append(&self, record: risk_governor_types::AuditRecord) -> Result<(), AuditError> {
        match self {
            Self::Mem(s) => s.append(record).await,
            Self::Pg(s) => s.append(record).await,
        }
    }
    async fn by_decision(&self, decision_id: Uuid) -> Result<Vec<risk_governor_types::AuditRecord>, AuditError> {
        match self {
            Self::Mem(s) => s.by_decision(decision_id).await,
            Self::Pg(s) => s.by_decision(decision_id).await,
        }
    }
    async fn all(&self) -> Result<Vec<risk_governor_types::AuditRecord>, AuditError> {
        match self {
            Self::Mem(s) => s.all().await,
            Self::Pg(s) => s.all().await,
        }
    }
}

// ---------------------------------------------------------------------------
// Gateway selection
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Gateway {
    Mock(Arc<MockGateway>),
    Http(Arc<HttpGateway>),
}

#[async_trait::async_trait]
impl RazorpayGateway for Gateway {
    async fn execute(
        &self,
        request: &AgentActionRequest,
        decision_id: Uuid,
    ) -> Result<serde_json::Value, ActionServiceError> {
        match self {
            Gateway::Mock(g) => g.execute(request, decision_id).await,
            Gateway::Http(h) => h.execute(request, decision_id).await,
        }
    }
}

fn pick_gateway() -> Arc<Gateway> {
    match (std::env::var("RAZORPAY_KEY_ID"), std::env::var("RAZORPAY_KEY_SECRET")) {
        (Ok(id), Ok(secret)) if !id.is_empty() && !secret.is_empty() => {
            tracing::info!("live Razorpay TEST-MODE gateway enabled");
            Arc::new(Gateway::Http(Arc::new(HttpGateway::new(id, secret))))
        }
        _ => {
            tracing::info!("no Razorpay keys — using MockGateway (moves no money)");
            Arc::new(Gateway::Mock(Arc::new(MockGateway::default())))
        }
    }
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

type Svc = ActionService<
    policy_engine::PolicyEngine,
    risk_engine::RiskEngine,
    EvidenceService<EvidenceBackend>,
    AuditService<AuditBackend>,
    Gateway,
>;

struct AppState {
    svc: Arc<Svc>,
    audit: Arc<AuditService<AuditBackend>>,
    gateway: Arc<Gateway>,
    /// Every decision served, keyed by decision_id — the review queue reads
    /// from here, replay reads the immutable trail from the audit store.
    decisions: RwLock<HashMap<Uuid, Decision>>,
    metrics: Arc<Metrics>,
    /// Set when DATABASE_URL is configured: decisions are write-through
    /// persisted here and hydrated from it at boot.
    pg: Option<Arc<PgStore>>,
    /// Required on every /v1/* route. From GOVERNOR_API_KEY, or an ephemeral
    /// generated key printed at boot (local dev/demo) — never "no auth".
    api_key: String,
}

/// Counters exposed at /metrics in Prometheus text format.
#[derive(Default)]
struct Metrics {
    decisions_allow: std::sync::atomic::AtomicU64,
    decisions_review: std::sync::atomic::AtomicU64,
    decisions_block: std::sync::atomic::AtomicU64,
    gateway_executions: std::sync::atomic::AtomicU64,
}

impl Metrics {
    fn record(&self, outcome: DecisionOutcome) {
        use std::sync::atomic::Ordering::Relaxed;
        match outcome {
            DecisionOutcome::Allow => self.decisions_allow.fetch_add(1, Relaxed),
            DecisionOutcome::Review => self.decisions_review.fetch_add(1, Relaxed),
            DecisionOutcome::Block => self.decisions_block.fetch_add(1, Relaxed),
        };
    }

    fn prometheus(&self) -> String {
        use std::sync::atomic::Ordering::Relaxed;
        let load = |c: &std::sync::atomic::AtomicU64| c.load(Relaxed).to_string();
        format!(
            "# HELP risk_governor_decisions_total Decisions by outcome.\n\
             # TYPE risk_governor_decisions_total counter\n\
             risk_governor_decisions_total{{outcome=\"allow\"}} {}\n\
             risk_governor_decisions_total{{outcome=\"review\"}} {}\n\
             risk_governor_decisions_total{{outcome=\"block\"}} {}\n\
             # HELP risk_governor_gateway_executions_total Money-movement calls fired at the gateway (ALLOW decisions + approved reviews).\n\
             # TYPE risk_governor_gateway_executions_total counter\n\
             risk_governor_gateway_executions_total {}\n",
            load(&self.decisions_allow),
            load(&self.decisions_review),
            load(&self.decisions_block),
            load(&self.gateway_executions),
        )
    }
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

async fn health() -> &'static str {
    "ok"
}

/// Prometheus text exposition of decision counters.
async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        state.metrics.prometheus(),
    )
}

/// The dashboard IS the product demo — root serves it directly. It carries
/// the server's API key so its fetches pass the same auth as every client.
async fn dashboard_page(State(state): State<Arc<AppState>>) -> axum::response::Html<String> {
    axum::response::Html(dashboard::page(&state.api_key))
}

/// Wire format for submissions: caller supplies business fields, server owns
/// timestamps/correlation IDs.
#[derive(serde::Deserialize)]
struct SubmitAction {
    agent_id: String,
    merchant_id: String,
    action_type: ActionType,
    amount: i64,
    currency: Option<String>,
    declared_intent: String,
    #[serde(default)]
    context: serde_json::Value,
}

async fn submit_action(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SubmitAction>,
) -> Result<Json<Decision>, ApiError> {
    let mut context = body.context;
    if !context.is_object() {
        context = serde_json::json!({});
    }
    // The intelligence plane keys off context.customer_id. A missing ID gets a
    // synthetic one: it shares nothing in the graph → no ring hypothesis → no
    // added friction. Never fail a request just because investigation can't run.
    if context.get("customer_id").is_none() {
        context["customer_id"] = serde_json::Value::String(format!("cust_{}", body.agent_id));
    }

    let request = AgentActionRequest {
        agent_id: body.agent_id,
        merchant_id: body.merchant_id,
        action_type: body.action_type,
        amount: body.amount,
        currency: body.currency.unwrap_or_else(|| "INR".into()),
        declared_intent: body.declared_intent,
        context,
        timestamp: now_utc(),
        correlation_id: generate_correlation_id(),
    };

    action_service::validate_request(&request)?;

    let decision = state.svc.process_action(request).await?;
    state.metrics.record(decision.decision);
    // Every ALLOW fired one money-movement call inside process_action.
    if decision.decision == DecisionOutcome::Allow {
        state
            .metrics
            .gateway_executions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    state
        .decisions
        .write()
        .expect("decisions lock")
        .insert(decision.decision_id, decision.clone());
    if let Some(pg) = &state.pg {
        if let Err(e) = pg.upsert_decision(&decision).await {
            tracing::error!(decision_id = %decision.decision_id, "decision persist failed: {e}");
        }
    }
    Ok(Json(decision))
}

async fn list_decisions(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let map = state.decisions.read().expect("decisions lock");
    let decisions: Vec<&Decision> = map.values().collect();
    Json(serde_json::json!(decisions
        .iter()
        .map(|d| serde_json::json!({
            "decision_id": d.decision_id,
            "agent_id": d.action.agent_id,
            "action_type": d.action.action_type,
            "amount": d.action.amount,
            "decision": d.decision,
            "risk_score": d.risk_result.risk_score,
            "human_decision": d.human_review.as_ref().map(|h| h.decision),
            "created_at": d.created_at,
        }))
        .collect::<Vec<_>>()))
}

/// Replay: what the governor saw, every evaluation it ran, and why it decided.
async fn replay_decision(
    State(state): State<Arc<AppState>>,
    Path(decision_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let decision = state
        .decisions
        .read()
        .expect("decisions lock")
        .get(&decision_id)
        .cloned()
        .ok_or(ApiError::not_found(decision_id))?;
    let trail = state
        .audit
        .trail_for(decision_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(serde_json::json!({
        "decision": decision,
        "audit_trail": trail,
    })))
}

#[derive(serde::Deserialize)]
struct ApproveBody {
    approved: bool,
    reviewer_id: String,
    #[serde(default)]
    notes: Option<String>,
}

/// Resolve a REVIEW. An approval executes the held action against the
/// gateway; a rejection closes it as BLOCK-with-human-context. Either way the
/// human's identity lands in the immutable audit trail.
async fn approve_decision(
    State(state): State<Arc<AppState>>,
    Path(decision_id): Path<Uuid>,
    Json(body): Json<ApproveBody>,
) -> Result<Json<Decision>, ApiError> {
    if body.reviewer_id.trim().is_empty() {
        return Err(ApiError::bad_request("reviewer_id is required".into()));
    }

    let mut decision = state
        .decisions
        .read()
        .expect("decisions lock")
        .get(&decision_id)
        .cloned()
        .ok_or(ApiError::not_found(decision_id))?;

    if decision.human_review.is_some() {
        return Err(ApiError::bad_request(format!(
            "decision {decision_id} already reviewed"
        )));
    }
    if decision.decision != DecisionOutcome::Review {
        return Err(ApiError::bad_request(format!(
            "decision {decision_id} is {:?}, not review — nothing to resolve",
            decision.decision
        )));
    }

    let resolved_outcome = if body.approved {
        DecisionOutcome::Allow
    } else {
        DecisionOutcome::Block
    };

    decision.human_review = Some(HumanReview {
        reviewer_id: body.reviewer_id.clone(),
        decision: resolved_outcome,
        notes: body.notes.clone(),
        reviewed_at: now_utc(),
    });

    state
        .audit
        .record(
            AuditEventType::HumanReviewed,
            Some(decision_id),
            serde_json::to_value(&decision.human_review).map_err(|e| ApiError::internal(e.to_string()))?,
        )
        .await;

    if body.approved {
        state
            .metrics
            .gateway_executions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let response = state.gateway.execute(&decision.action, decision_id).await?;
        state
            .audit
            .record(
                AuditEventType::RazorpayCalled,
                Some(decision_id),
                serde_json::json!({
                    "via_human_approval": true,
                    "reviewer_id": body.reviewer_id,
                    "response": response,
                }),
            )
            .await;
    }

    state
        .decisions
        .write()
        .expect("decisions lock")
        .insert(decision_id, decision.clone());
    if let Some(pg) = &state.pg {
        if let Err(e) = pg.upsert_decision(&decision).await {
            tracing::error!(decision_id = %decision_id, "reviewed decision persist failed: {e}");
        }
    }
    Ok(Json(decision))
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct ApiError {
    status: axum::http::StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: String) -> Self {
        Self {
            status: axum::http::StatusCode::BAD_REQUEST,
            message,
        }
    }
    fn not_found(id: Uuid) -> Self {
        Self {
            status: axum::http::StatusCode::NOT_FOUND,
            message: format!("decision {id} not found"),
        }
    }
    fn internal(message: String) -> Self {
        Self {
            status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            message,
        }
    }
}

impl From<ActionServiceError> for ApiError {
    fn from(e: ActionServiceError) -> Self {
        ApiError::internal(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(serde_json::json!({ "error": self.message }))).into_response()
    }
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// A governor that executes money movement with NO authentication would be
/// the exact "valid credentials ≠ valid action" gap it exists to close. Every
/// /v1/* route requires the key; /health and /metrics stay open (liveness +
/// Prometheus scrape), and the dashboard page carries the server's own key so
/// it authenticates like any other client.
fn resolve_api_key() -> String {
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

async fn require_api_key(
    State(state): State<Arc<AppState>>,
    req: axum::extract::Request,
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

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

/// The full router, public + protected. Split out so tests exercise the exact
/// production route stack including the auth layer.
fn build_router(state: Arc<AppState>) -> Router {
    let protected = Router::new()
        .route("/v1/actions", post(submit_action))
        .route("/v1/decisions", get(list_decisions))
        .route("/v1/decisions/:id", get(replay_decision))
        .route("/v1/decisions/:id/approve", post(approve_decision))
        .layer(axum::middleware::from_fn_with_state(state.clone(), require_api_key));

    Router::new()
        .route("/", get(dashboard_page))
        .route("/dashboard", get(dashboard_page))
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .merge(protected)
        .with_state(state)
}

/// Default graph shipped with the server: a few isolated customers sharing
/// nothing. Unknown customer_ids land here → "no_cluster" → Unsupported
/// verdict → zero added friction for ordinary traffic.
fn default_graph_and_behaviors() -> (Arc<risk_graph::PropertyGraph>, HashMap<String, CustomerBehavior>) {
    let mut b = risk_graph::GraphBuilder::new();
    for c in ["cust_agent-trusted-01", "cust_agent-sketchy-99"] {
        b = b.entity(risk_graph::EntityKind::Customer, c);
    }
    let graph = Arc::new(b.build());
    (graph, HashMap::new())
}

async fn seed_demo_entities(store: &EvidenceBackend) -> Result<(), anyhow::Error> {
    let agents = [
        AgentHistory {
            agent_id: "agent-trusted-01".into(),
            total_actions_30d: 30,
            total_volume_30d: 1_500_000,
            avg_amount: 50_000,
            max_amount: 100_000,
            refund_rate: 0.05,
            block_rate: 0.02,
            review_rate: 0.03,
            first_seen: now_utc() - chrono::Duration::days(90),
            last_action: now_utc() - chrono::Duration::hours(2),
            action_type_distribution: Default::default(),
            anomaly_flags: vec![],
        },
        AgentHistory {
            agent_id: "agent-sketchy-99".into(),
            total_actions_30d: 300,
            total_volume_30d: 18_000_000,
            avg_amount: 60_000,
            max_amount: 120_000,
            refund_rate: 0.25,
            block_rate: 0.10,
            review_rate: 0.15,
            first_seen: now_utc() - chrono::Duration::days(3),
            last_action: now_utc() - chrono::Duration::minutes(10),
            action_type_distribution: Default::default(),
            anomaly_flags: vec!["rapid_fire".into()],
        },
    ];
    match store {
        EvidenceBackend::Mem(s) => {
            for a in agents {
                s.seed_agent(a).await;
            }
            s.seed_default_policy_if_missing("merchant-001").await?;
        }
        EvidenceBackend::Pg(s) => {
            let mut seed = serde_json::Map::new();
            seed.insert(
                "agents".into(),
                serde_json::to_value(&agents).expect("agents serialize"),
            );
            // Merchant policy mirrors InMemory defaults (seed_default_policy_if_missing).
            let policy = MerchantPolicy {
                merchant_id: "merchant-001".into(),
                max_refund_amount: 500_000,
                max_payout_amount: 1_000_000,
                max_payment_link_amount: 250_000,
                daily_refund_limit: 2_000_000,
                daily_payout_limit: 5_000_000,
                velocity_threshold_per_hour: 10,
                allowed_countries: vec![],
                blocked_countries: vec![],
                require_approval_above: 100_000,
                custom_rules: vec![],
            };
            seed.insert(
                "merchants".into(),
                serde_json::json!([serde_json::to_value(&policy).expect("policy serialize")]),
            );
            s.seed_from_json(&serde_json::Value::Object(seed).to_string()).await?;
        }
    }
    Ok(())
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
        None => EvidenceBackend::Mem(Arc::new(InMemoryEvidenceStore::new())),
    };
    seed_demo_entities(&evidence_backend).await?;

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
        None => AuditBackend::Mem(Arc::new(InMemoryAuditStore::new())),
    };

    // Hydrate prior decisions so replay/review survive restarts.
    let decisions: HashMap<Uuid, Decision> = match &pg {
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

    let gateway = pick_gateway();

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
    let investigator = GraphInvestigator::new(graph, behaviors, HashMap::new(), Baseline::default());

    let svc = Arc::new(
        ActionService::new(
            Arc::new(policy_engine::PolicyEngine::new()),
            Arc::new(risk),
            Arc::new(EvidenceService::new(Arc::new(evidence_backend))),
            Arc::new(AuditService::new(Arc::new(audit_backend.clone()))),
            gateway.clone(),
        )
        .with_investigator(investigator.into_trait()),
    );

    let state = Arc::new(AppState {
        svc,
        audit: Arc::new(AuditService::new(Arc::new(audit_backend))),
        gateway,
        decisions: RwLock::new(decisions),
        metrics: Arc::new(Metrics::default()),
        pg,
        api_key: resolve_api_key(),
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
    use axum::body::Body;
    use axum::extract::State;
    use tower::ServiceExt;

    const TEST_KEY: &str = "rgov_test_key";

    async fn test_state() -> Arc<AppState> {
        let evidence_store = Arc::new(InMemoryEvidenceStore::new());
        seed_demo_entities(&EvidenceBackend::Mem(evidence_store.clone()))
            .await
            .unwrap();
        let audit_store = Arc::new(InMemoryAuditStore::new());
        let gateway = Arc::new(Gateway::Mock(Arc::new(MockGateway::default())));
        let (graph, behaviors) = default_graph_and_behaviors();
        let investigator = GraphInvestigator::new(graph, behaviors, HashMap::new(), Baseline::default());
        let svc = Arc::new(
            ActionService::new(
                Arc::new(policy_engine::PolicyEngine::new()),
                Arc::new(risk_engine::RiskEngine::default()),
                Arc::new(EvidenceService::new(Arc::new(EvidenceBackend::Mem(evidence_store)))),
                Arc::new(AuditService::new(Arc::new(AuditBackend::Mem(audit_store.clone())))),
                gateway.clone(),
            )
            .with_investigator(investigator.into_trait()),
        );
        Arc::new(AppState {
            svc,
            audit: Arc::new(AuditService::new(Arc::new(AuditBackend::Mem(audit_store)))),
            gateway,
            decisions: RwLock::new(HashMap::new()),
            metrics: Arc::new(Metrics::default()),
            pg: None,
            api_key: TEST_KEY.into(),
        })
    }

    fn submit_body(agent: &str, amount: i64) -> Json<SubmitAction> {
        Json(SubmitAction {
            agent_id: agent.into(),
            merchant_id: "merchant-001".into(),
            action_type: ActionType::Refund,
            amount,
            currency: Some("INR".into()),
            declared_intent: "refund for order #1".into(),
            context: serde_json::json!({}),
        })
    }

    #[tokio::test]
    async fn metrics_counters_track_decision_outcomes() {
        let state = test_state().await;
        // trusted agent, small amount → allow
        let _ = submit_action(State(state.clone()), submit_body("agent-trusted-01", 50_000))
            .await
            .unwrap();
        // trusted agent above approval threshold → review
        let _ = submit_action(State(state.clone()), submit_body("agent-trusted-01", 150_000))
            .await
            .unwrap();
        // over hard cap → block
        let _ = submit_action(State(state.clone()), submit_body("agent-trusted-01", 600_000))
            .await
            .unwrap();

        let body = state.metrics.prometheus();
        assert!(body.contains("risk_governor_decisions_total{outcome=\"allow\"} 1"));
        assert!(body.contains("risk_governor_decisions_total{outcome=\"review\"} 1"));
        assert!(body.contains("risk_governor_decisions_total{outcome=\"block\"} 1"));
        // ALLOW fired one gateway execution; the review was not approved
        assert!(body.contains("risk_governor_gateway_executions_total 1"));
    }

    #[tokio::test]
    async fn approval_execution_increments_gateway_counter() {
        let state = test_state().await;
        let decision = submit_action(State(state.clone()), submit_body("agent-trusted-01", 150_000))
            .await
            .unwrap();
        assert_eq!(decision.decision, DecisionOutcome::Review);

        let _ = approve_decision(
            State(state.clone()),
            Path(decision.decision_id),
            Json(ApproveBody {
                approved: true,
                reviewer_id: "analyst-test".into(),
                notes: None,
            }),
        )
        .await
        .unwrap();

        assert!(state
            .metrics
            .prometheus()
            .contains("risk_governor_gateway_executions_total 1"));
    }

    #[tokio::test]
    async fn health_and_metrics_handlers_respond() {
        let state = test_state().await;
        assert_eq!(health().await, "ok");
        let resp = metrics(State(state)).await.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    // --- auth on /v1/* (full production router incl. middleware) ---------

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

    #[test]
    fn constant_time_eq_matches_only_equal_inputs() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }
}
