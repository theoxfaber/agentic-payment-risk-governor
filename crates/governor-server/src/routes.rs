//! HTTP route handlers for the decision API, replay, review queue, dashboard,
//! health and Prometheus metrics.

use crate::state::AppState;
use action_service::{ActionServiceError, RazorpayGateway as _};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;
use risk_governor_types::*;
use serde::Deserialize;
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
    const INDEX: &str = include_str!("../../../dashboard-v2/dist/index.html");
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
    /// Optional client idempotency key (also accepted via Idempotency-Key /
    /// X-Idempotency-Key header). Retries with the same key return the
    /// original decision instead of moving money twice.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

pub(crate) async fn submit_action(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SubmitAction>,
) -> Result<Json<Decision>, ApiError> {
    let mut context = body.context;
    if !context.is_object() {
        context = serde_json::json!({});
    }

    // Idempotency fingerprint over CLIENT-supplied fields only — computed
    // BEFORE the synthetic customer_id below (fresh UUID per request) so a
    // retried POST hashes identically. Same key + different fingerprint is a
    // client bug → 422, never a silent replay of the wrong action.
    let idem_fingerprint = AgentActionRequest {
        agent_id: body.agent_id.clone(),
        merchant_id: body.merchant_id.clone(),
        action_type: body.action_type,
        amount: body.amount,
        currency: body.currency.clone().unwrap_or_else(|| "INR".into()),
        declared_intent: body.declared_intent.clone(),
        context: context.clone(),
        timestamp: now_utc(),
        correlation_id: Uuid::nil(),
    }
    .input_hash();

    // Synthetic customer_id per-request UUID to avoid spurious clustering (cust_{agent_id} would link all requests from same agent via device).
    if context.get("customer_id").is_none() {
        context["customer_id"] = serde_json::Value::String(format!("cust_{}", Uuid::new_v4()));
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

    // Request-level idempotency with pending-claim: header wins, body key is
    // the fallback. The claim (Pending) is inserted atomically under one lock
    // hold BEFORE process_action runs, so two concurrent POSTs with the same
    // key cannot both mint a decision_id and double-execute. Duplicates wait
    // for Ready (up to 60s) and receive the original decision.
    let idem_key = headers
        .get("idempotency-key")
        .or_else(|| headers.get("x-idempotency-key"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            body.idempotency_key
                .clone()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        });
    if let Some(ref key) = idem_key {
        let mut waits = 0u32;
        loop {
            // Atomic check-and-claim under a single lock hold.
            let slot = {
                let mut g = state.idempotency.lock().await;
                match g.get(key).cloned() {
                    None => {
                        g.insert(key.clone(), crate::state::IdemSlot::Pending);
                        None
                    }
                    some => some,
                }
            };
            match slot {
                // We own the claim — fall through to process_action.
                None => break,
                Some(crate::state::IdemSlot::Ready {
                    decision_id,
                    input_hash,
                }) => {
                    if input_hash != idem_fingerprint {
                        return Err(ApiError::unprocessable(
                            "Idempotency-Key already used for a different action".to_string(),
                        ));
                    }
                    if let Some(cached) = state.decisions.write().await.get(&decision_id).cloned() {
                        return Ok(Json(cached));
                    }
                    // Decision evicted from the LRU: drop the stale mapping and reclaim.
                    let mut g = state.idempotency.lock().await;
                    if matches!(g.get(key), Some(crate::state::IdemSlot::Ready { .. })) {
                        g.remove(key);
                    }
                }
                Some(crate::state::IdemSlot::Pending) => {
                    if waits >= 1200 {
                        return Err(ApiError::conflict(
                            "duplicate request with this Idempotency-Key is still processing — retry".to_string(),
                        ));
                    }
                    waits += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
    }

    let start = std::time::Instant::now();
    let decision = match state.svc.process_action(request.clone()).await {
        Ok(d) => d,
        Err(e) => {
            // Release a claim we own so a retry can proceed (a failed decision
            // never executed the gateway, so there is nothing to replay).
            if let Some(ref key) = idem_key {
                let mut g = state.idempotency.lock().await;
                if matches!(g.get(key), Some(crate::state::IdemSlot::Pending)) {
                    g.remove(key);
                }
            }
            return Err(e.into());
        }
    };
    state.metrics.record_latency_ms(start.elapsed().as_secs_f64() * 1000.0);
    if let Some(insight) = &decision.learned_insight {
        state.metrics.record_learned(insight.p_hat, &insight.band);
    }
    state.metrics.record(decision.decision);
    state.metrics.record_score(decision.risk_result.risk_score);
    if decision.decision == DecisionOutcome::Allow {
        state.metrics.count_execution();
    }
    state
        .decisions
        .write()
        .await
        .put(decision.decision_id, decision.clone());
    if let Some(ref key) = idem_key {
        state.idempotency.lock().await.insert(
            key.clone(),
            crate::state::IdemSlot::Ready {
                decision_id: decision.decision_id,
                input_hash: idem_fingerprint,
            },
        );
    }
    if let Some(pg) = &state.pg {
        if let Err(e) = pg.upsert_decision(&decision).await {
            tracing::error!(decision_id = %decision.decision_id, "decision persist failed: {e}");
        }
    }
    Ok(Json(decision))
}

pub(crate) async fn list_decisions(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let map = state.decisions.read().await;
    let decisions: Vec<&Decision> = map.iter().map(|(_, d)| d).collect();
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

pub(crate) async fn razorpay_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    let secret = state
        .webhook_secret
        .as_deref()
        .ok_or_else(|| ApiError::internal("webhook not configured: set WEBHOOK_SECRET".into()))?;
    let sig = headers
        .get("x-razorpay-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("missing X-Razorpay-Signature".into()))?;
    if !razorpay_gateway::verify_webhook_signature(&body, sig, secret) {
        return Err(ApiError::unauthorized("invalid webhook signature".into()));
    }
    let payload: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| ApiError::bad_request(format!("invalid json: {e}")))?;
    state
        .audit
        .record(AuditEventType::WebhookReceived, None, payload.clone())
        .await;
    Ok(Json(
        serde_json::json!({"verified": true, "received": payload.get("event")}),
    ))
}

/// Replay: what the governor saw, every evaluation it ran, and why it decided.
/// The per-decision trail is verified with the hash chain; a tampered trail returns 500.
pub(crate) async fn replay_decision(
    State(state): State<Arc<AppState>>,
    Path(decision_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, ApiError> {
    // LRU get() promotes recency and needs &mut — write lock required even for reads.
    let decision = state
        .decisions
        .write()
        .await
        .get(&decision_id)
        .cloned()
        .ok_or(ApiError::not_found(decision_id))?;
    let trail = state
        .audit
        .trail_for(decision_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let chain_verified = audit_service::AuditService::<crate::backends::AuditBackend>::verify_records(&trail).is_ok();
    if !chain_verified && !trail.is_empty() {
        let err = audit_service::AuditService::<crate::backends::AuditBackend>::verify_records(&trail).unwrap_err();
        return Err(ApiError::internal(format!(
            "audit chain tampered for {decision_id}: {err}"
        )));
    }
    let anchor = state.anchor_key.as_deref().and_then(|k| {
        trail.last().map(|h| {
            serde_json::json!({
                "head_hash": h.current_hash,
                "hmac_sha256": audit_service::anchor_signature(&h.current_hash, k),
                "anchored": true,
            })
        })
    });
    Ok(Json(serde_json::json!({
        "decision": decision,
        "audit_trail": trail,
        "audit_verified": chain_verified,
        "audit_anchor": anchor,
    })))
}

pub(crate) async fn verify_audit_chain(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let records = state.audit.all_records().await.unwrap_or_default();
    let verified = audit_service::AuditService::<crate::backends::AuditBackend>::verify_chain(&records);
    let head = records.last().map(|r| r.current_hash.clone());
    let anchor = match (&state.anchor_key, &head) {
        (Some(k), Some(h)) => Some(serde_json::json!({
            "head_hash": h,
            "hmac_sha256": audit_service::anchor_signature(h, k),
            "anchored": true,
        })),
        _ => head
            .as_ref()
            .map(|h| serde_json::json!({ "head_hash": h, "anchored": false })),
    };
    match verified {
        Ok(()) => Json(serde_json::json!({
            "verified": true,
            "records": records.len(),
            "head": head,
            "anchor": anchor,
        })),
        Err(e) => Json(serde_json::json!({
            "verified": false,
            "records": records.len(),
            "head": head,
            "anchor": anchor,
            "error": e,
        })),
    }
}

pub(crate) async fn audit_anchor(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let records = state.audit.all_records().await.unwrap_or_default();
    let head = records.last().map(|r| r.current_hash.clone());
    match (&state.anchor_key, head) {
        (Some(k), Some(h)) => Json(serde_json::json!({
            "head_hash": h,
            "hmac_sha256": audit_service::anchor_signature(&h, k),
            "anchored": true,
            "records": records.len(),
            "note": "HMAC-SHA256 of chain head with AUDIT_SIGNING_KEY (out-of-process). Publish this anchor externally; a full-chain rewrite without the key cannot forge it."
        })),
        (None, Some(h)) => Json(serde_json::json!({
            "head_hash": h,
            "anchored": false,
            "records": records.len(),
            "note": "Chain is hash-linked but not HMAC-anchored. Set AUDIT_SIGNING_KEY to enable external anchor; also REVOKE UPDATE/DELETE on audit_records in Postgres for append-only storage."
        })),
        (_, None) => Json(serde_json::json!({ "head_hash": null, "anchored": false, "records": 0 })),
    }
}

#[derive(Deserialize)]
pub(crate) struct RealParams {
    pub count: Option<usize>,
}

pub(crate) async fn real_analysis(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RealParams>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let count = params.count.unwrap_or(20).clamp(1, 100);
    let mut source = "razorpay_test_payments";
    let raw = state
        .gateway
        .fetch_real_payments(count)
        .await
        .unwrap_or_else(|_| serde_json::json!({"items":[]}));
    let mut items = raw.get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    if items.is_empty() {
        if let Ok(orders_raw) = state.gateway.fetch_real_orders(count).await {
            if let Some(order_items) = orders_raw.get("items").and_then(|v| v.as_array()) {
                if !order_items.is_empty() {
                    items = order_items.clone();
                    source = "razorpay_test_orders";
                }
            }
        }
    }
    if items.is_empty() {
        return Ok(Json(serde_json::json!({
            "source": source,
            "count": 0,
            "total_paise": 0,
            "avg_paise": 0,
            "payments": [],
            "note": "No live test payments/orders yet — create an order first (smoke already creates order_TWOGRNeAdwc689). Synthetic dataset-gen remains for offline held-out."
        })));
    }
    let total: i64 = items
        .iter()
        .filter_map(|p| p.get("amount").and_then(|v| v.as_i64()))
        .sum();
    let avg = if items.is_empty() {
        0
    } else {
        total / items.len() as i64
    };
    let analysis: Vec<serde_json::Value> = items
        .iter()
        .map(|p| {
            let amt = p.get("amount").and_then(|v| v.as_i64()).unwrap_or(0);
            let status = p.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
            let risk = if amt > avg * 3 {
                "high"
            } else if amt > avg * 2 {
                "medium"
            } else {
                "low"
            };
            serde_json::json!({
                "id": p.get("id"),
                "amount_paise": amt,
                "amount_inr": amt as f64 / 100.0,
                "status": status,
                "risk_flag": risk,
                "raw": p
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "source": source,
        "count": items.len(),
        "total_paise": total,
        "avg_paise": avg,
        "payments": analysis,
        "note": "Real Razorpay test-mode data — replaces synthetic dataset-gen for live analysis. Requires RAZORPAY_KEY_ID/SECRET."
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
    if body.reviewer_id.len() > 128 {
        return Err(ApiError::bad_request("reviewer_id too long (max 128)".into()));
    }

    // Atomic claim: remove under the write lock. A concurrent approver now
    // sees None (404) instead of a stale unreviewed copy — no check-then-act
    // race across the gateway await.
    let mut decision = {
        let mut map = state.decisions.write().await;
        map.pop(&decision_id).ok_or(ApiError::not_found(decision_id))?
    };

    macro_rules! restore_and {
        ($err:expr) => {{
            let err = $err;
            state.decisions.write().await.put(decision_id, decision);
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
                // Sync the unresolved state back to Postgres before restoring
                // to the in-memory map, preventing memory/DB desync.
                if let Some(pg) = &state.pg {
                    if let Err(pe) = pg.upsert_decision(&decision).await {
                        tracing::error!(decision_id = %decision_id, "failed to persist unresolved decision: {pe}");
                    }
                }
                restore_and!(ApiError::internal(format!("gateway execution failed: {e}")));
            }
        }
    }

    state.decisions.write().await.put(decision_id, decision.clone());
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
    pub(crate) fn unauthorized(message: String) -> Self {
        Self {
            status: axum::http::StatusCode::UNAUTHORIZED,
            message,
        }
    }
    pub(crate) fn conflict(message: String) -> Self {
        Self {
            status: axum::http::StatusCode::CONFLICT,
            message,
        }
    }
    pub(crate) fn unprocessable(message: String) -> Self {
        Self {
            status: axum::http::StatusCode::UNPROCESSABLE_ENTITY,
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

    async fn submit(state: Arc<AppState>, agent: &str, amount: i64) -> Decision {
        let (hdrs, body) = submit_body(agent, amount);
        submit_action(State(state), hdrs, body).await.unwrap().0
    }

    fn keyed_body(agent: &str, amount: i64, key: &str) -> (HeaderMap, Json<SubmitAction>) {
        let (_, body) = submit_body(agent, amount);
        let mut hdrs = HeaderMap::new();
        hdrs.insert("idempotency-key", key.parse().unwrap());
        let body = body.0;
        (
            hdrs,
            Json(SubmitAction {
                idempotency_key: None,
                ..body
            }),
        )
    }

    #[tokio::test]
    async fn concurrent_same_key_processes_exactly_once() {
        // THE request-level race: N concurrent POSTs with one Idempotency-Key
        // must yield ONE decision_id (single process_action, single gateway
        // execution), never N decisions.
        let state = test_state().await;
        let mut handles = Vec::new();
        for _ in 0..8 {
            let st = state.clone();
            handles.push(tokio::spawn(async move {
                let (hdrs, body) = keyed_body("agent-trusted-01", 50_000, "key-race-1");
                submit_action(State(st), hdrs, body).await.unwrap().0
            }));
        }
        let mut ids = std::collections::HashSet::new();
        for h in handles {
            ids.insert(h.await.unwrap().decision_id);
        }
        assert_eq!(ids.len(), 1, "same key must map to one decision, got {ids:?}");
        assert_eq!(
            state
                .metrics
                .prometheus()
                .matches("risk_governor_gateway_executions_total 1")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn same_key_replay_returns_original_decision() {
        let state = test_state().await;
        let (h1, b1) = keyed_body("agent-trusted-01", 50_000, "key-replay-1");
        let first = submit_action(State(state.clone()), h1, b1).await.unwrap().0;
        let (h2, b2) = keyed_body("agent-trusted-01", 50_000, "key-replay-1");
        let second = submit_action(State(state.clone()), h2, b2).await.unwrap().0;
        assert_eq!(first.decision_id, second.decision_id);
    }

    #[tokio::test]
    async fn key_reuse_with_different_body_is_rejected() {
        let state = test_state().await;
        let (h1, b1) = keyed_body("agent-trusted-01", 50_000, "key-reuse-1");
        let _ = submit_action(State(state.clone()), h1, b1).await.unwrap();
        let (h2, b2) = keyed_body("agent-trusted-01", 60_000, "key-reuse-1");
        let err = submit_action(State(state.clone()), h2, b2).await.unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn metrics_counters_track_decision_outcomes() {
        let state = test_state().await;
        // trusted agent, small amount → allow
        let _ = submit(state.clone(), "agent-trusted-01", 50_000).await;
        // trusted agent above approval threshold → review
        let _ = submit(state.clone(), "agent-trusted-01", 150_000).await;
        // over hard cap → block
        let _ = submit(state.clone(), "agent-trusted-01", 600_000).await;

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
        let decision = submit(state.clone(), "agent-trusted-01", 150_000).await;
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
        let decision = submit(state.clone(), "agent-trusted-01", 150_000).await;

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
        let decision = submit(state.clone(), "agent-trusted-01", 50_000).await;

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
        let (hdrs, body) = submit_body("agent-trusted-01", -1);
        let result = submit_action(State(state), hdrs, body).await;
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
        let decision = submit(state.clone(), "agent-trusted-01", 150_000).await;
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
            .write()
            .await
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
