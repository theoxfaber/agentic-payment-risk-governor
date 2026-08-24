//! Shared application state and Prometheus decision counters.

use action_service::ActionService;
use audit_service::AuditService;
use evidence_service::EvidenceService;
use risk_governor_types::*;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

use crate::backends::{AuditBackend, EvidenceBackend, Gateway};

pub(crate) type Svc = ActionService<
    policy_engine::PolicyEngine,
    risk_engine::RiskEngine,
    EvidenceService<EvidenceBackend>,
    AuditService<AuditBackend>,
    Gateway,
>;

pub(crate) struct AppState {
    pub svc: Arc<Svc>,
    pub audit: Arc<AuditService<AuditBackend>>,
    pub gateway: Arc<Gateway>,
    /// Every decision served, keyed by decision_id — the review queue reads
    /// from here, replay reads the immutable trail from the audit store.
    pub decisions: RwLock<HashMap<Uuid, Decision>>,
    pub metrics: Arc<Metrics>,
    /// Set when DATABASE_URL is configured: decisions are write-through
    /// persisted here and hydrated from it at boot.
    pub pg: Option<Arc<pg_store::PgStore>>,
    /// Required on every /v1/* route. From GOVERNOR_API_KEY, or an ephemeral
    /// generated key printed at boot (local dev/demo) — never "no auth".
    pub api_key: String,
}

/// Counters exposed at /metrics in Prometheus text format.
#[derive(Default)]
pub(crate) struct Metrics {
    decisions_allow: std::sync::atomic::AtomicU64,
    decisions_review: std::sync::atomic::AtomicU64,
    decisions_block: std::sync::atomic::AtomicU64,
    gateway_executions: std::sync::atomic::AtomicU64,
}

impl Metrics {
    pub(crate) fn record(&self, outcome: DecisionOutcome) {
        use std::sync::atomic::Ordering::Relaxed;
        match outcome {
            DecisionOutcome::Allow => self.decisions_allow.fetch_add(1, Relaxed),
            DecisionOutcome::Review => self.decisions_review.fetch_add(1, Relaxed),
            DecisionOutcome::Block => self.decisions_block.fetch_add(1, Relaxed),
        };
    }

    pub(crate) fn count_execution(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        self.gateway_executions.fetch_add(1, Relaxed);
    }

    pub(crate) fn prometheus(&self) -> String {
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
