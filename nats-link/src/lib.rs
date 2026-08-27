//! The one-hop split (Phase 2, step 3): action-service → policy-engine over NATS.
//!
//! Client side implements `action_service::PolicyEngine` so `ActionService`
//! doesn't know or care that evaluation now happens in another process.
//!
//! Failure discipline: if the worker is down or slow, we fail SAFE — the
//! combiner routes "policy_engine_unavailable" to human Review, never
//! silent-allow and never a hard error.

use action_service::{ActionServiceError, GatheredEvidence};
use async_nats::Client;
use futures::StreamExt;
use policy_engine::PolicyEngine as InProcessPolicyEngine;
use risk_governor_correlation::{decode, scope_correlation, Envelope};
use risk_governor_types::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

pub const SUBJECT_POLICY_EVAL: &str = "policy.evaluate.requested";
pub const SUBJECT_EVIDENCE_GATHER: &str = "evidence.gather.requested";
pub const SUBJECT_EVIDENCE_RECORD: &str = "evidence.action.recorded";

const DEFAULT_EVAL_TIMEOUT: Duration = Duration::from_millis(1500);
const DEFAULT_GATHER_TIMEOUT: Duration = Duration::from_millis(1500);

/// Explicit marker consumed by the decision combiner → forces Review.
pub fn policy_unavailable_result(reason: &str) -> PolicyResult {
    PolicyResult {
        verdict: PolicyVerdict::Allow,
        matched_rules: vec![format!("policy_engine_unavailable:{reason}")],
        violated_thresholds: vec![],
        evaluated_at: now_utc(),
    }
}

// ---------------------------------------------------------------------------
// Client side (lives inside action-service's process)
// ---------------------------------------------------------------------------

pub struct NatsPolicyEngine {
    client: Client,
    timeout: Duration,
}

impl NatsPolicyEngine {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            timeout: DEFAULT_EVAL_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[async_trait::async_trait]
impl action_service::PolicyEngine for NatsPolicyEngine {
    async fn evaluate(
        &self,
        request: &AgentActionRequest,
        evidence: &Evidence,
    ) -> Result<PolicyResult, action_service::ActionServiceError> {
        let cid = risk_governor_correlation::current_correlation_id();
        let reply = format!("policy.evaluate.reply.{cid}");
        let mut sub = self
            .client
            .subscribe(reply.clone())
            .await
            .map_err(|e| action_service::ActionServiceError::PolicyEngine(e.to_string()))?;

        let job = PolicyEvaluateJob {
            request: request.clone(),
            evidence: evidence.clone(),
        };
        let bytes = Envelope::new(SUBJECT_POLICY_EVAL, job)
            .encode()
            .map_err(|e| action_service::ActionServiceError::PolicyEngine(e.to_string()))?;

        self.client
            .publish_with_reply(SUBJECT_POLICY_EVAL, reply, bytes.into())
            .await
            .map_err(|e| ActionServiceError::PolicyEngine(e.to_string()))?;

        match tokio::time::timeout(self.timeout, sub.next()).await {
            Ok(Some(msg)) => {
                // Server-side "no responders": empty message with a 503-style
                // status arrives when the worker's subscription hasn't
                // registered (or is down). Fail safe, don't decode garbage.
                if msg.status.is_some() {
                    let code = msg.status.map(|s| s.as_u16().to_string()).unwrap_or_default();
                    return Ok(policy_unavailable_result(&format!("no_responders_{code}")));
                }
                let env: Envelope<PolicyResult> = decode(&msg.payload)
                    .map_err(|e| action_service::ActionServiceError::PolicyEngine(e.to_string()))?;
                Ok(env.payload)
            }
            Ok(None) => Ok(policy_unavailable_result("worker_closed_inbox")),
            Err(_) => Ok(policy_unavailable_result("timeout")),
        }
    }
}

// ---------------------------------------------------------------------------
// Worker side (its own process)
// ---------------------------------------------------------------------------

/// Subscribe + serve loop for the policy worker. Returns Err if the
/// subscription cannot be established, so a worker binary can exit NON-ZERO
/// instead of sitting idle looking healthy.
pub async fn run_policy_worker(client: Client) -> anyhow::Result<()> {
    let mut sub = client
        .subscribe(SUBJECT_POLICY_EVAL)
        .await
        .map_err(|e| anyhow::anyhow!("policy worker subscribe failed: {e}"))?;
    info!(subject = SUBJECT_POLICY_EVAL, "policy-engine-worker listening");

    while let Some(msg) = sub.next().await {
        let client = client.clone();
        tokio::spawn(async move { handle_one(client, msg).await });
    }
    Ok(())
}

/// Spawned variant for tests/demos — kill by aborting the handle. The
/// JoinHandle carries the startup result so callers can detect failure.
pub fn spawn_policy_worker(client: Client) -> JoinHandle<anyhow::Result<()>> {
    tokio::spawn(run_policy_worker(client))
}

async fn handle_one(client: Client, msg: async_nats::Message) {
    let env: Envelope<PolicyEvaluateJob> = match decode(&msg.payload) {
        Ok(env) => env,
        Err(e) => {
            error!("policy worker: undecodable message: {e}");
            return;
        }
    };

    // Restore the caller's correlation context — log lines in THIS process
    // carry the same correlation_id as the originating HTTP request.
    scope_correlation(env.correlation_id, async move {
        let agent = env.payload.request.agent_id.clone();
        info!(%agent, "policy evaluation received");

        let result = match InProcessPolicyEngine::new()
            .evaluate(&env.payload.request, &env.payload.evidence)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                error!(%agent, "policy evaluation failed: {e}");
                policy_unavailable_result("worker_eval_error")
            }
        };
        info!(verdict = ?result.verdict, rules = ?result.matched_rules, "policy evaluation complete");

        if let Some(reply) = msg.reply {
            // Envelope::new picks up the scoped correlation id — same as caller's.
            match Envelope::new("policy.evaluate.result", result).encode() {
                Ok(bytes) => {
                    if let Err(e) = client.publish(reply, bytes.into()).await {
                        error!("policy worker: reply publish failed: {e}");
                    }
                }
                Err(e) => error!("policy worker: reply encode failed: {e}"),
            }
        }
    })
    .await;
}
// ---------------------------------------------------------------------------
// Evidence service: client + worker (Phase 2 step 4)
// ---------------------------------------------------------------------------

/// Transport-degraded evidence: benign defaults. The combiner forces Review
/// via the injected marker rule, so the content here only has to be valid,
/// not accurate.
fn fail_safe_evidence(request: &AgentActionRequest, reason: &str) -> GatheredEvidence {
    let evidence = Evidence {
        agent_history: AgentHistory {
            agent_id: request.agent_id.clone(),
            total_actions_30d: 0,
            total_volume_30d: 0,
            avg_amount: 0,
            max_amount: 0,
            std_amount: 0,
            refund_rate: 0.0,
            block_rate: 0.0,
            review_rate: 0.0,
            first_seen: now_utc(),
            last_action: now_utc(),
            action_type_distribution: Default::default(),
            anomaly_flags: vec![],
        },
        merchant_policy: MerchantPolicy {
            merchant_id: request.merchant_id.clone(),
            max_refund_amount: i64::MAX / 2,
            max_payout_amount: i64::MAX / 2,
            max_payment_link_amount: i64::MAX / 2,
            daily_refund_limit: i64::MAX / 2,
            daily_payout_limit: i64::MAX / 2,
            velocity_threshold_per_hour: u32::MAX,
            allowed_countries: vec![],
            blocked_countries: vec![],
            require_approval_above: i64::MAX / 2,
            custom_rules: vec![],
        },
        customer_history: None,
        recent_velocity: VelocityStats::default(),
        fetched_at: now_utc(),
    };
    GatheredEvidence {
        evidence,
        degraded_reason: Some(reason.to_string()),
    }
}

pub struct NatsEvidenceService {
    client: Client,
    timeout: Duration,
}

impl NatsEvidenceService {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            timeout: DEFAULT_GATHER_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[async_trait::async_trait]
impl action_service::EvidenceService for NatsEvidenceService {
    async fn gather(&self, request: &AgentActionRequest) -> Result<GatheredEvidence, ActionServiceError> {
        let cid = risk_governor_correlation::current_correlation_id();
        let reply = format!("evidence.gather.reply.{cid}");
        let mut sub = self
            .client
            .subscribe(reply.clone())
            .await
            .map_err(|e| ActionServiceError::EvidenceService(e.to_string()))?;

        let bytes = Envelope::new(SUBJECT_EVIDENCE_GATHER, request.clone())
            .encode()
            .map_err(|e| ActionServiceError::EvidenceService(e.to_string()))?;

        self.client
            .publish_with_reply(SUBJECT_EVIDENCE_GATHER, reply, bytes.into())
            .await
            .map_err(|e| ActionServiceError::EvidenceService(e.to_string()))?;

        match tokio::time::timeout(self.timeout, sub.next()).await {
            Ok(Some(msg)) => {
                if msg.status.is_some() {
                    // No responders — same NATS race/gotcha as policy hop.
                    let code = msg.status.map(|s| s.as_u16().to_string()).unwrap_or_default();
                    return Ok(fail_safe_evidence(request, &format!("no_responders_{code}")));
                }
                let env: Envelope<EvidenceOutcome> =
                    decode(&msg.payload).map_err(|e| ActionServiceError::EvidenceService(e.to_string()))?;
                match env.payload {
                    EvidenceOutcome::Ready(ev) => Ok(GatheredEvidence::fresh(ev)),
                    // Application-level: unknown merchant/agent → fail CLOSED
                    EvidenceOutcome::NotFound(msg) => Err(ActionServiceError::EvidenceService(msg)),
                }
            }
            Ok(None) => Ok(fail_safe_evidence(request, "worker_closed_inbox")),
            Err(_) => Ok(fail_safe_evidence(request, "timeout")),
        }
    }

    async fn record_action(&self, request: &AgentActionRequest) -> Result<(), ActionServiceError> {
        // Fire-and-forget by design: velocity feedback must never block or
        // fail a decision.
        match Envelope::new(SUBJECT_EVIDENCE_RECORD, request.clone()).encode() {
            Ok(bytes) => {
                if let Err(e) = self.client.publish(SUBJECT_EVIDENCE_RECORD, bytes.into()).await {
                    warn!("evidence record_action publish failed (best-effort): {e}");
                }
            }
            Err(e) => warn!("evidence record_action encode failed (best-effort): {e}"),
        }
        Ok(())
    }
}

/// Subscribe + serve loop for the evidence worker. Returns Err if either
/// subscription fails — the binary exits non-zero so an orchestrator can
/// see (and restart) it, instead of a silent zombie.
pub async fn run_evidence_worker<S: evidence_service::EvidenceStore + 'static>(
    client: Client,
    store: Arc<S>,
) -> anyhow::Result<()> {
    let mut gather_sub = client
        .subscribe(SUBJECT_EVIDENCE_GATHER)
        .await
        .map_err(|e| anyhow::anyhow!("evidence worker subscribe failed (gather): {e}"))?;
    let mut record_sub = client
        .subscribe(SUBJECT_EVIDENCE_RECORD)
        .await
        .map_err(|e| anyhow::anyhow!("evidence worker subscribe failed (record): {e}"))?;
    info!(
        gather = SUBJECT_EVIDENCE_GATHER,
        record = SUBJECT_EVIDENCE_RECORD,
        "evidence-worker listening"
    );

    loop {
        tokio::select! {
            Some(msg) = gather_sub.next() => {
                let client = client.clone();
                let store = store.clone();
                tokio::spawn(async move { handle_gather(client, store, msg).await });
            }
            Some(msg) = record_sub.next() => {
                let store = store.clone();
                tokio::spawn(async move { handle_record(store, msg).await });
            }
            else => break,
        }
    }
    Ok(())
}

/// Spawned variant for tests/demos — kill by aborting the handle.
pub fn spawn_evidence_worker<S: evidence_service::EvidenceStore + 'static>(
    client: Client,
    store: Arc<S>,
) -> JoinHandle<anyhow::Result<()>> {
    tokio::spawn(run_evidence_worker(client, store))
}

async fn handle_gather<S: evidence_service::EvidenceStore + 'static>(
    client: Client,
    store: Arc<S>,
    msg: async_nats::Message,
) {
    let env: Envelope<AgentActionRequest> = match decode(&msg.payload) {
        Ok(env) => env,
        Err(e) => {
            error!("evidence worker: undecodable gather request: {e}");
            return;
        }
    };

    scope_correlation(env.correlation_id, async move {
        let svc = evidence_service::EvidenceService::new(store);
        info!(agent = %env.payload.agent_id, "evidence gather received");

        let outcome = match svc.gather(&env.payload).await {
            Ok(ev) => {
                info!(merchant = %ev.merchant_policy.merchant_id, "evidence ready");
                EvidenceOutcome::Ready(ev)
            }
            Err(e) => {
                warn!("evidence gather not found: {e}");
                EvidenceOutcome::NotFound(e.to_string())
            }
        };

        if let Some(reply) = msg.reply {
            match Envelope::new("evidence.gather.result", outcome).encode() {
                Ok(bytes) => {
                    if let Err(e) = client.publish(reply, bytes.into()).await {
                        error!("evidence worker: reply publish failed: {e}");
                    }
                }
                Err(e) => error!("evidence worker: reply encode failed: {e}"),
            }
        }
    })
    .await;
}

async fn handle_record<S: evidence_service::EvidenceStore + 'static>(store: Arc<S>, msg: async_nats::Message) {
    let env: Envelope<AgentActionRequest> = match decode(&msg.payload) {
        Ok(env) => env,
        Err(e) => {
            warn!("evidence worker: undecodable record request: {e}");
            return;
        }
    };
    scope_correlation(env.correlation_id, async move {
        if let Err(e) = store.record_action(&env.payload).await {
            warn!("evidence worker: record_action failed: {e}");
        }
    })
    .await;
}
