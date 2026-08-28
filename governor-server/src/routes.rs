//! HTTP route handlers for the decision API, replay, review queue, dashboard,
//! health and Prometheus metrics.

use crate::state::AppState;
use action_service::{ActionServiceError, RazorpayGateway as _};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use risk_governor_types::*;
use std::sync::Arc;
use uuid::Uuid;

/// Liveness probe.
pub(crate) async fn health() -> &'static str {
    "ok"
}

/// Prometheus text exposition of decision counters.
pub(crate) async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        state.metrics.prometheus(),
    )
}

/// Production triage console — Vite + React build from dashboard-v2/dist.
/// Served as static files; unauthenticated and carries no secret.
pub(crate) async fn dashboard_page() -> axum::response::Html<String> {
    const INDEX: &str = include_str!("../../dashboard-v2/dist/index.html");
    axum::response::Html(INDEX.to_string())
}

/// Wire format for submissions: caller supplies business fields, server owns
/// timestamps/correlation IDs.
#[derive(serde::Deserialize)]
pub(crate) struct SubmitAction {
    pub agent_id: String,
    pub merchant_id: String,
    pub action_type: ActionType,
    pub amount: i64,
    pub currency: Option<String>,
    pub declared_intent: String,
    #[serde(default)]
    pub context: serde_json::Value,
}

pub(crate) async fn submit_action(
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

    // Client-supplied business fields: a violation is the CALLER's error →
    // 400, never a 500.
    action_service::validate_request(&request).map_err(|e| ApiError::bad_request(e.to_string()))?;

    let mut decision = state.svc.process_action(request).await?;
    {
        let scorer = crate::learned::LearnedScorer::from_embedded();
        let insight = scorer.score(&decision.evidence_snapshot, &decision.action);
        decision.learned_insight = Some(insight);
    }
    state.metrics.record(decision.decision);
    // Drift monitoring: every scored action feeds the risk-score histogram
    // (PSI at /metrics vs SCORE_REFERENCE_JSON when configured).
    state.metrics.record_score(decision.risk_result.risk_score);
    // Every ALLOW fired one money-movement call inside process_action.
    if decision.decision == DecisionOutcome::Allow {
        state.metrics.count_execution();
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

pub(crate) async fn list_decisions(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
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
            "learned_p_hat": d.learned_insight.as_ref().map(|l| l.p_hat),
            "learned_band": d.learned_insight.as_ref().map(|l| &l.band),
            "learned_version": d.learned_insight.as_ref().map(|l| &l.model_version),
            "human_decision": d.human_review.as_ref().map(|h| h.decision),
            "created_at": d.created_at,
        }))
        .collect::<Vec<_>>()))
}

/// Replay: what the governor saw, every evaluation it ran, and why it decided.
pub(crate) async fn replay_decision(
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
pub(crate) struct ApproveBody {
    pub approved: bool,
    pub reviewer_id: String,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Resolve a REVIEW. An approval executes the held action against the
/// gateway; a rejection closes it as BLOCK-with-human-context. Either way the
/// human's identity lands in the immutable audit trail.
///
/// Concurrency: the decision is CLAIMED (removed from the map) under the
/// write lock before any await point, so two simultaneous approvers can never
/// both pass the already-reviewed guard and double-execute the payment.
/// The claim is restored if resolution cannot proceed.
pub(crate) async fn approve_decision(
    State(state): State<Arc<AppState>>,
    Path(decision_id): Path<Uuid>,
    Json(body): Json<ApproveBody>,
) -> Result<Json<Decision>, ApiError> {
    if body.reviewer_id.trim().is_empty() {
        return Err(ApiError::bad_request("reviewer_id is required".into()));
    }

    // Atomic claim: remove under the write lock. A concurrent approver now
    // sees None (404) instead of a stale unreviewed copy — no check-then-act
    // race across the gateway await.
    let mut decision = {
        let mut map = state.decisions.write().expect("decisions lock");
        map.remove(&decision_id).ok_or(ApiError::not_found(decision_id))?
    };

    // Restore-on-reject helper: the claim must go back or the queue loses it.
    // The error value is built FIRST — it may read from the claim.
    macro_rules! restore_and {
        ($err:expr) => {{
            let err = $err;
            state
                .decisions
                .write()
                .expect("decisions lock")
                .insert(decision_id, decision);
            return Err(err);
        }};
    }

    if decision.human_review.is_some() {
        restore_and!(ApiError::bad_request(format!(
            "decision {decision_id} already reviewed"
        )));
    }
    if decision.decision != DecisionOutcome::Review {
        restore_and!(ApiError::bad_request(format!(
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
        match state.gateway.execute(&decision.action, decision_id).await {
            Ok(response) => {
                state.metrics.count_execution();
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
            Err(e) => {
                // Money did NOT move (gateway errored). Restore the decision
                // UNRESOLVED so a reviewer can retry, and record the failed
                // attempt so replay shows why.
                decision.human_review = None;
                state
                    .audit
                    .record(
                        AuditEventType::RazorpayCalled,
                        Some(decision_id),
                        serde_json::json!({
                            "via_human_approval": true,
                            "reviewer_id": body.reviewer_id,
                            "status": "execution_failed",
                            "error": e.to_string(),
                        }),
                    )
                    .await;
                restore_and!(ApiError::internal(format!("gateway execution failed: {e}")));
            }
        }
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
pub(crate) struct ApiError {
    pub status: axum::http::StatusCode,
    pub message: String,
}

impl ApiError {
    pub(crate) fn bad_request(message: String) -> Self {
        Self {
            status: axum::http::StatusCode::BAD_REQUEST,
            message,
        }
    }
    pub(crate) fn not_found(id: Uuid) -> Self {
        Self {
            status: axum::http::StatusCode::NOT_FOUND,
            message: format!("decision {id} not found"),
        }
    }
    pub(crate) fn internal(message: String) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bootstrap::test_support::{submit_body, test_state};

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
    async fn double_approval_is_rejected() {
        let state = test_state().await;
        let decision = submit_action(State(state.clone()), submit_body("agent-trusted-01", 150_000))
            .await
            .unwrap();

        let first = approve_decision(
            State(state.clone()),
            Path(decision.decision_id),
            Json(ApproveBody {
                approved: true,
                reviewer_id: "analyst-1".into(),
                notes: None,
            }),
        )
        .await
        .unwrap();
        // The outcome field stays REVIEW; the human's resolution lands in
        // human_review — that's what replay shows.
        assert_eq!(
            first.human_review.as_ref().map(|h| h.decision),
            Some(DecisionOutcome::Allow)
        );

        let second = approve_decision(
            State(state),
            Path(decision.decision_id),
            Json(ApproveBody {
                approved: false,
                reviewer_id: "analyst-2".into(),
                notes: None,
            }),
        )
        .await;
        assert!(
            matches!(second, Err(ref e) if e.status == axum::http::StatusCode::BAD_REQUEST),
            "second resolution must be rejected"
        );
    }

    #[tokio::test]
    async fn replay_returns_decision_with_full_trail() {
        let state = test_state().await;
        let decision = submit_action(State(state.clone()), submit_body("agent-trusted-01", 50_000))
            .await
            .unwrap();

        let payload = replay_decision(State(state), Path(decision.decision_id)).await.unwrap();
        let json: serde_json::Value = payload.0;
        assert_eq!(json["decision"]["decision_id"], decision.decision_id.to_string());
        let trail = json["audit_trail"].as_array().expect("trail present");
        let events: Vec<&str> = trail.iter().filter_map(|e| e["event_type"].as_str()).collect();
        assert!(events.contains(&"action_requested"));
        assert!(events.contains(&"decision_made"));
        assert!(
            events.contains(&"razorpay_called"),
            "ALLOW must record the gateway call"
        );
    }

    #[tokio::test]
    async fn replay_unknown_decision_is_404() {
        let state = test_state().await;
        let err = replay_decision(State(state), Path(Uuid::new_v4())).await.unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn submit_rejects_invalid_business_fields() {
        let state = test_state().await;
        // negative amount fails validate_request before any pipeline runs —
        // a caller mistake must surface as 400, not a server error.
        let result = submit_action(State(state), submit_body("agent-trusted-01", -1)).await;
        assert!(
            matches!(result, Err(ref e) if e.status == axum::http::StatusCode::BAD_REQUEST),
            "validation failure maps to 400"
        );
    }

    /// THE race guard: two concurrent approvals of the same decision. The
    /// claim-under-lock protocol means exactly ONE may execute the payment;
    /// the other gets 404 (claim taken) or 400 (already resolved) — never a
    /// second gateway call.
    #[tokio::test]
    async fn concurrent_approvals_execute_exactly_once() {
        let state = test_state().await;
        let decision = submit_action(State(state.clone()), submit_body("agent-trusted-01", 150_000))
            .await
            .unwrap();
        assert_eq!(decision.decision, DecisionOutcome::Review);

        let mut handles = Vec::new();
        for i in 0..8 {
            let st = state.clone();
            let did = decision.decision_id;
            handles.push(tokio::spawn(async move {
                approve_decision(
                    State(st),
                    Path(did),
                    Json(ApproveBody {
                        approved: true,
                        reviewer_id: format!("analyst-{i}"),
                        notes: None,
                    }),
                )
                .await
            }));
        }

        let mut successes = 0usize;
        for h in handles {
            if h.await.unwrap().is_ok() {
                successes += 1;
            }
        }
        assert_eq!(successes, 1, "exactly one concurrent approver may win");

        // The gateway fired ONCE, not once per approver.
        let body = state.metrics.prometheus();
        assert!(
            body.contains("risk_governor_gateway_executions_total 1"),
            "double execution detected:\n{body}"
        );

        // Resolved decision is back in the map with the winner's review.
        let resolved = state
            .decisions
            .read()
            .expect("decisions lock")
            .get(&decision.decision_id)
            .cloned()
            .expect("resolved decision restored");
        assert_eq!(resolved.human_review.map(|h| h.decision), Some(DecisionOutcome::Allow));
    }

    #[tokio::test]
    async fn health_and_metrics_handlers_respond() {
        let state = test_state().await;
        assert_eq!(health().await, "ok");
        let resp = metrics(State(state)).await.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }
}
