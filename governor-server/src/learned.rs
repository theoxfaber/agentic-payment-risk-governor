use risk_governor_types::{AgentActionRequest, Evidence, LearnedInsight};
use std::collections::HashMap;

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
struct Thresholds {
    tau_clear: f64,
    tau_block: f64,
}

pub struct LearnedScorer {
    model: Model,
    tau_clear: f64,
    tau_block: f64,
}

impl LearnedScorer {
    pub fn from_embedded() -> Self {
        let art: Artifact = serde_json::from_str(ARTIFACT).expect("lr_model.json parses");
        Self {
            model: art.model,
            tau_clear: art.thresholds.tau_clear,
            tau_block: art.thresholds.tau_block,
        }
    }

    pub fn score(&self, evidence: &Evidence, request: &AgentActionRequest) -> LearnedInsight {
        let feats = self.extract(evidence, request);
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
        LearnedInsight {
            model_version: self.model.version.clone(),
            p_hat,
            tau_clear: self.tau_clear,
            tau_block: self.tau_block,
            band,
            features: map,
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

    fn extract(&self, evidence: &Evidence, _request: &AgentActionRequest) -> Vec<f64> {
        let agent = &evidence.agent_history;
        let vel = &evidence.recent_velocity;
        let age_days = (chrono::Utc::now() - agent.first_seen).num_days().max(1) as f64;
        let log_age = age_days.ln().max(0.0);
        let distinct_merchants_norm = (vel.unique_merchants_24h as f64 / 10.0).clamp(0.0, 1.0);
        let distinct_products_norm = (vel.unique_customers_24h as f64 / 10.0).clamp(0.0, 1.0);
        let sync_share = (vel.actions_last_hour as f64 / 10.0).clamp(0.0, 1.0) * 0.15;
        let cluster_size_norm: f64 = 0.125;
        let return_refund_rate = agent.refund_rate.clamp(0.0, 1.0);
        let dispute_ratio = agent.block_rate.clamp(0.0, 1.0);
        let cluster_pooled = return_refund_rate;
        vec![
            return_refund_rate,
            log_age,
            distinct_merchants_norm,
            distinct_products_norm,
            dispute_ratio,
            sync_share,
            cluster_size_norm,
            cluster_pooled,
        ]
    }
}
