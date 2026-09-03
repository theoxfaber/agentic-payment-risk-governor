use risk_governor_types::{AgentActionRequest, Evidence, InvestigationSummary, LearnedInsight};
use std::collections::{BTreeMap, HashMap};

const ARTIFACT: &str = include_str!("../../eval-harness/artifacts/lr_model.json");

#[derive(serde::Deserialize)]
struct Artifact {
    model: Model,
    thresholds: Thresholds,
}
#[derive(serde::Deserialize)]
struct Model {
    feature_names: Vec<String>,
    means: Vec<f64>,
    stds: Vec<f64>,
    weights: Vec<f64>,
    bias: f64,
    version: String,
}
#[derive(serde::Deserialize)]
#[allow(dead_code)]
struct Thresholds {
    tau_clear: f64,
    tau_block: f64,
    alpha_leak: f64,
    alpha_friction: f64,
}

pub trait LearnedScorer: Send + Sync {
    fn score_with_investigation(
        &self,
        evidence: &Evidence,
        request: &AgentActionRequest,
        investigation: Option<&InvestigationSummary>,
    ) -> LearnedInsight;
}

/// Serving feature ORDER — must match the artifact's feature_names positionally
/// (predict() standardizes by index). feature[3] is the train/serve analog:
/// train (eval-harness) feeds distinct_products/30, serving feeds
/// unique_customers_24h/30 — same "breadth" position and /30 scale, documented
/// in eval-harness/src/learned.rs. from_embedded asserts the artifact agrees.
const SERVE_FEATURE_NAMES: &[&str] = &[
    "return_refund_rate",
    "log_account_age_days",
    "distinct_merchants_norm",
    "breadth_norm",
    "dispute_ratio",
    "sync_share_72h",
    "cluster_size_norm",
    "cluster_pooled_return_rate",
];

pub struct DefaultLearnedScorer {
    model: Model,
    tau_clear: f64,
    tau_block: f64,
}

impl DefaultLearnedScorer {
    pub fn from_embedded() -> Self {
        let art: Artifact = serde_json::from_str(ARTIFACT).expect("lr_model.json parses");
        // Loud contract: a renamed/reordered artifact must fail here, never
        // silently skew every p_hat by feeding features to wrong weights.
        assert_eq!(
            art.model.feature_names, SERVE_FEATURE_NAMES,
            "lr_model.json feature order drifted from serving extractor"
        );
        Self {
            model: art.model,
            tau_clear: art.thresholds.tau_clear,
            tau_block: art.thresholds.tau_block,
        }
    }
    fn predict(&self, features: &[f64]) -> f64 {
        let mut z = self.model.bias;
        for (i, &x) in features.iter().enumerate() {
            let std = self.model.stds[i].max(1e-9);
            z += self.model.weights[i] * ((x - self.model.means[i]) / std);
        }
        1.0 / (1.0 + (-z).exp())
    }
    fn extract(
        &self,
        evidence: &Evidence,
        _request: &AgentActionRequest,
        investigation: Option<&InvestigationSummary>,
    ) -> Vec<f64> {
        let agent = &evidence.agent_history;
        let vel = &evidence.recent_velocity;
        let return_refund_rate = agent.refund_rate.clamp(0.0, 1.0);
        let age_days = (chrono::Utc::now() - agent.first_seen).num_days().max(1) as f64;
        let log_account_age_days = (1.0 + age_days).ln() / 10.0;
        let distinct_merchants_norm = if vel.unique_merchants_24h == 0 && vel.actions_last_24h == 0 {
            0.57
        } else {
            (vel.unique_merchants_24h as f64 / 12.0).clamp(0.0, 1.0)
        };
        // Serving analog of train's distinct_products/30 (see SERVE_FEATURE_NAMES):
        // per-agent customer breadth on the same /30 scale.
        let unique_customers_norm = if vel.unique_customers_24h == 0 && vel.actions_last_24h == 0 {
            0.54
        } else {
            (vel.unique_customers_24h as f64 / 30.0).clamp(0.0, 1.0)
        };
        let dispute_ratio = agent.block_rate.clamp(0.0, 1.0);
        let sync_share_72h = if vel.actions_last_hour == 0 || vel.actions_last_24h == 0 {
            0.02
        } else {
            let burst = vel.actions_last_hour as f64 / vel.actions_last_24h.max(1) as f64;
            (burst * 0.3).clamp(0.0, 1.0)
        };
        let (cluster_size_norm, cluster_pooled_return_rate) = match investigation {
            Some(inv) if inv.structurally_suspicious => {
                let size_norm = if inv.support_signals >= 2 { 0.5 } else { 0.375 };
                let pooled = if inv.verdict == risk_governor_types::InvestigationVerdict::Supported {
                    (return_refund_rate + 0.15).clamp(0.0, 1.0)
                } else {
                    return_refund_rate
                };
                (size_norm, pooled)
            }
            Some(_) => (0.125, return_refund_rate),
            None => (0.125, return_refund_rate),
        };
        vec![
            return_refund_rate,
            log_account_age_days,
            distinct_merchants_norm,
            unique_customers_norm,
            dispute_ratio,
            sync_share_72h,
            cluster_size_norm,
            cluster_pooled_return_rate,
        ]
    }
}

impl LearnedScorer for DefaultLearnedScorer {
    fn score_with_investigation(
        &self,
        evidence: &Evidence,
        request: &AgentActionRequest,
        investigation: Option<&InvestigationSummary>,
    ) -> LearnedInsight {
        let feats = self.extract(evidence, request, investigation);
        let p_hat = self.predict(&feats);
        let band = if p_hat < self.tau_clear {
            "clear"
        } else if p_hat > self.tau_block {
            "block"
        } else {
            "review"
        }
        .to_string();
        let mut map = HashMap::new();
        for (k, v) in self.model.feature_names.iter().zip(feats.iter()) {
            map.insert(k.clone(), *v);
        }
        let mut contrib_vec: Vec<(String, f64)> = self
            .model
            .feature_names
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let std = self.model.stds[i].max(1e-9);
                let standardized = (feats[i] - self.model.means[i]) / std;
                (k.clone(), self.model.weights[i] * standardized)
            })
            .collect();
        contrib_vec.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap_or(std::cmp::Ordering::Equal));
        let contrib: BTreeMap<String, f64> = contrib_vec.into_iter().collect();
        LearnedInsight {
            model_version: self.model.version.clone(),
            p_hat,
            tau_clear: self.tau_clear,
            tau_block: self.tau_block,
            band,
            features: map,
            contributions: Some(contrib),
        }
    }
}
