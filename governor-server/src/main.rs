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
use audit_service::{AuditService, InMemoryAuditStore};
use evidence_service::{EvidenceService, InMemoryEvidenceStore};
use investigation_engine::{Baseline, CustomerBehavior, GraphInvestigator};
use razorpay_gateway::{HttpGateway, MockGateway};
use risk_governor_types::*;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

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
    EvidenceService<InMemoryEvidenceStore>,
    AuditService<InMemoryAuditStore>,
    Gateway,
>;

struct AppState {
    svc: Arc<Svc>,
    audit: Arc<AuditService<InMemoryAuditStore>>,
    gateway: Arc<Gateway>,
    /// Every decision served, keyed by decision_id — the review queue reads
    /// from here, replay reads the immutable trail from the audit store.
    decisions: RwLock<HashMap<Uuid, Decision>>,
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

async fn health() -> &'static str {
    "ok"
}

/// The dashboard IS the product demo — root serves it directly.
async fn dashboard_page() -> axum::response::Html<&'static str> {
    axum::response::Html(dashboard::page())
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
    state
        .decisions
        .write()
        .expect("decisions lock")
        .insert(decision.decision_id, decision.clone());
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
    Ok(Json(decision))
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

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
// Wiring
// ---------------------------------------------------------------------------

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

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

async fn seed_demo_entities(store: &InMemoryEvidenceStore) -> Result<(), anyhow::Error> {
    store
        .seed_agent(AgentHistory {
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
        })
        .await;
    store
        .seed_agent(AgentHistory {
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
        })
        .await;
    store.seed_default_policy_if_missing("merchant-001").await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower=warn".into()),
        )
        .init();

    let evidence_store = Arc::new(InMemoryEvidenceStore::new());
    seed_demo_entities(&evidence_store).await?;
    let audit_store = Arc::new(InMemoryAuditStore::new());
    let gateway = pick_gateway();

    let (graph, behaviors) = default_graph_and_behaviors();
    let investigator = GraphInvestigator::new(graph, behaviors, HashMap::new(), Baseline::default());

    let svc = Arc::new(
        ActionService::new(
            Arc::new(policy_engine::PolicyEngine::new()),
            Arc::new(risk_engine::RiskEngine::default()),
            Arc::new(EvidenceService::new(evidence_store.clone())),
            Arc::new(AuditService::new(audit_store.clone())),
            gateway.clone(),
        )
        .with_investigator(investigator.into_trait()),
    );

    let state = Arc::new(AppState {
        svc,
        audit: Arc::new(AuditService::new(audit_store)),
        gateway,
        decisions: RwLock::new(HashMap::new()),
    });

    let app = Router::new()
        .route("/", get(dashboard_page))
        .route("/dashboard", get(dashboard_page))
        .route("/health", get(health))
        .route("/v1/actions", post(submit_action))
        .route("/v1/decisions", get(list_decisions))
        .route("/v1/decisions/:id", get(replay_decision))
        .route("/v1/decisions/:id/approve", post(approve_decision))
        .with_state(state);

    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8080);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!("risk governor listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
