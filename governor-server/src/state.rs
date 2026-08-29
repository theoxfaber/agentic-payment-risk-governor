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

#[allow(dead_code)]
pub(crate) struct AppState {
    pub svc: Arc<Svc>,
    pub audit: Arc<AuditService<AuditBackend>>,
    pub gateway: Arc<Gateway>,
    pub decisions: RwLock<HashMap<Uuid, Decision>>,
    pub metrics: Arc<Metrics>,
    pub pg: Option<Arc<pg_store::PgStore>>,
    pub api_key: String,
    pub anchor_key: Option<Vec<u8>>,
    pub webhook_secret: Option<String>,
    pub graph: Arc<risk_graph::PropertyGraph>,
    pub behaviors: HashMap<String, investigation_engine::CustomerBehavior>,
}

/// Counters exposed at /metrics in Prometheus text format.
#[derive(Default)]
pub(crate) struct Metrics {
    decisions_allow: std::sync::atomic::AtomicU64,
    decisions_review: std::sync::atomic::AtomicU64,
    decisions_block: std::sync::atomic::AtomicU64,
    gateway_executions: std::sync::atomic::AtomicU64,
    score_buckets: [std::sync::atomic::AtomicU64; 5],
    learned_buckets: [std::sync::atomic::AtomicU64; 5],
    learned_reviews: std::sync::atomic::AtomicU64,
    learned_blocks: std::sync::atomic::AtomicU64,
}

/// Reference proportions for the score buckets, loaded once from
/// SCORE_REFERENCE_JSON (e.g. produced by `cargo run -p eval-harness`).
/// Unset → PSI is not exported rather than computed against a guess.
fn score_reference() -> &'static Option<[f64; 5]> {
    static REF: std::sync::OnceLock<Option<[f64; 5]>> = std::sync::OnceLock::new();
    REF.get_or_init(|| {
        let raw = std::env::var("SCORE_REFERENCE_JSON").ok()?;
        let v: Vec<f64> = serde_json::from_str(&raw).ok()?;
        if v.len() == 5 {
            Some([v[0], v[1], v[2], v[3], v[4]])
        } else {
            tracing::warn!("SCORE_REFERENCE_JSON must be an array of 5 proportions — PSI disabled");
            None
        }
    })
}

/// Population Stability Index between current bucket proportions `current`
/// and a reference distribution. Standard drift statistic; χ²-calibrated
/// thresholds belong to the alerting layer, not this metric.
pub(crate) fn psi(current: &[f64; 5], reference: &[f64; 5]) -> f64 {
    const EPS: f64 = 1e-6;
    current
        .iter()
        .zip(reference)
        .map(|(&c, &r)| {
            let c = c.max(EPS);
            let r = r.max(EPS);
            (c - r) * (c / r).ln()
        })
        .sum()
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

    /// Bucket a risk score into the fixed histogram.
    pub(crate) fn record_score(&self, score: f64) {
        use std::sync::atomic::Ordering::Relaxed;
        let idx = (((score.clamp(0.0, 1.0)) * 5.0).floor() as usize).min(4);
        self.score_buckets[idx].fetch_add(1, Relaxed);
    }

    pub(crate) fn record_learned(&self, p_hat: f64, band: &str) {
        use std::sync::atomic::Ordering::Relaxed;
        let idx = (((p_hat.clamp(0.0, 1.0)) * 5.0).floor() as usize).min(4);
        self.learned_buckets[idx].fetch_add(1, Relaxed);
        match band {
            "review" => self.learned_reviews.fetch_add(1, Relaxed),
            "block" => self.learned_blocks.fetch_add(1, Relaxed),
            _ => 0,
        };
    }

    fn bucket_proportions(&self) -> Option<[f64; 5]> {
        use std::sync::atomic::Ordering::Relaxed;
        let counts: Vec<u64> = self.score_buckets.iter().map(|b| b.load(Relaxed)).collect();
        let total: u64 = counts.iter().sum();
        if total == 0 {
            return None;
        }
        Some([
            counts[0] as f64 / total as f64,
            counts[1] as f64 / total as f64,
            counts[2] as f64 / total as f64,
            counts[3] as f64 / total as f64,
            counts[4] as f64 / total as f64,
        ])
    }

    /// Current PSI vs the configured reference distribution, if one exists
    /// and any scores have been observed.
    pub(crate) fn current_psi(&self) -> Option<f64> {
        let reference = score_reference().as_ref()?;
        self.bucket_proportions().map(|cur| psi(&cur, reference))
    }

    pub(crate) fn prometheus(&self) -> String {
        use std::sync::atomic::Ordering::Relaxed;
        let load = |c: &std::sync::atomic::AtomicU64| c.load(Relaxed).to_string();
        let mut out = format!(
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
        );

        if let Some(props) = self.bucket_proportions() {
            let edges = ["0.2", "0.4", "0.6", "0.8", "1.0"];
            out.push_str(
                "# HELP risk_governor_risk_score_bucket Decisions by risk-score bucket.\n\
                 # TYPE risk_governor_risk_score_bucket counter\n",
            );
            for (edge, p) in edges.iter().zip(props.iter()) {
                out.push_str(&format!("risk_governor_risk_score_bucket{{le=\"{edge}\"}} {:.6}\n", p));
            }
            if let Some(psi_value) = self.current_psi() {
                out.push_str(
                    "# HELP risk_governor_score_psi Population Stability Index of the risk-score distribution vs SCORE_REFERENCE_JSON.\n\
                     # TYPE risk_governor_score_psi gauge\n",
                );
                out.push_str(&format!("risk_governor_score_psi {psi_value:.6}\n"));
            }
        }
        {
            use std::sync::atomic::Ordering::Relaxed;
            let counts: Vec<u64> = self.learned_buckets.iter().map(|b| b.load(Relaxed)).collect();
            let total: u64 = counts.iter().sum();
            if total > 0 {
                out.push_str(
                    "# HELP risk_governor_learned_p_hat_bucket Calibrated p_hat histogram.\n\
                     # TYPE risk_governor_learned_p_hat_bucket counter\n",
                );
                let edges = ["0.2", "0.4", "0.6", "0.8", "1.0"];
                for (edge, c) in edges.iter().zip(counts.iter()) {
                    out.push_str(&format!(
                        "risk_governor_learned_p_hat_bucket{{le=\"{edge}\"}} {:.6}\n",
                        *c as f64 / total as f64
                    ));
                }
                out.push_str(
                    "# HELP risk_governor_learned_band_total Decisions by learned band.\n\
                     # TYPE risk_governor_learned_band_total counter\n",
                );
                out.push_str(&format!(
                    "risk_governor_learned_band_total{{band=\"review\"}} {}\n",
                    self.learned_reviews.load(Relaxed)
                ));
                out.push_str(&format!(
                    "risk_governor_learned_band_total{{band=\"block\"}} {}\n",
                    self.learned_blocks.load(Relaxed)
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn score_buckets_map_correctly() {
        let m = Metrics::default();
        for &s in &[0.0f64, 0.19, 0.2, 0.55, 0.99] {
            m.record_score(s);
        }
        let props = m.bucket_proportions().unwrap();
        assert_eq!(props, [0.4, 0.2, 0.2, 0.0, 0.2]);
    }

    #[test]
    fn psi_zero_for_identical_and_positive_for_drift() {
        let r = [0.5, 0.3, 0.1, 0.07, 0.03];
        assert!(psi(&r, &r).abs() < 1e-9);
        let drifted = [0.1, 0.2, 0.3, 0.25, 0.15];
        assert!(psi(&drifted, &r) > 0.5, "big shift must produce large PSI");
    }

    #[test]
    fn prometheus_exports_buckets_without_reference() {
        // No SCORE_REFERENCE_JSON in test env → histogram yes, PSI no.
        let m = Metrics::default();
        m.record_score(0.7);
        let body = m.prometheus();
        assert!(body.contains("risk_governor_risk_score_bucket"));
        assert!(
            !body.contains("risk_governor_score_psi"),
            "PSI needs an explicit reference"
        );
    }
}
