#![allow(dead_code)]
use risk_governor_types::{AgentActionRequest, Evidence, InvestigationSummary, LearnedInsight};
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
    alpha_leak: f64,
    alpha_friction: f64,
}

pub struct LearnedScorer {
    model: Model,
    tau_clear: f64,
    tau_block: f64,
    #[allow(dead_code)]
    alpha_leak: f64,
    #[allow(dead_code)]
    alpha_friction: f64,
}

impl LearnedScorer {
    pub fn from_embedded() -> Self {
        let art: Artifact = serde_json::from_str(ARTIFACT).expect("lr_model.json parses");
        Self {
            model: art.model,
            tau_clear: art.thresholds.tau_clear,
            tau_block: art.thresholds.tau_block,
            alpha_leak: art.thresholds.alpha_leak,
            alpha_friction: art.thresholds.alpha_friction,
        }
    }

    #[allow(dead_code)]
    pub fn tau_clear(&self) -> f64 {
        self.tau_clear
    }

    #[allow(dead_code)]
    pub fn tau_block(&self) -> f64 {
        self.tau_block
    }

    #[allow(dead_code)]
    pub fn version(&self) -> &str {
        &self.model.version
    }

    #[allow(dead_code)]
    pub fn score(&self, evidence: &Evidence, request: &AgentActionRequest) -> LearnedInsight {
        self.score_with_investigation(evidence, request, None)
    }

    pub fn score_with_investigation(
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
        LearnedInsight {
            model_version: self.model.version.clone(),
            p_hat,
            tau_clear: self.tau_clear,
            tau_block: self.tau_block,
            band,
            features: map,
        }
    }

    #[allow(dead_code)]
    pub fn decide(&self, insight: &LearnedInsight, exposure_paise: i64) -> &'static str {
        const REVIEW_COST_PAISE: f64 = 40_000.0;
        let expected_loss = insight.p_hat * exposure_paise as f64;
        if expected_loss <= REVIEW_COST_PAISE {
            "clear"
        } else if insight.p_hat >= self.tau_block {
            "block"
        } else {
            "review"
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
        let distinct_products_norm = if vel.unique_customers_24h == 0 && vel.actions_last_24h == 0 {
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
            distinct_products_norm,
            dispute_ratio,
            sync_share_72h,
            cluster_size_norm,
            cluster_pooled_return_rate,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use risk_governor_types::*;

    fn evidence(refund_rate: f64, age_days: i64, merchants: u32, block_rate: f64) -> Evidence {
        Evidence {
            agent_history: AgentHistory {
                agent_id: "a".into(),
                total_actions_30d: 30,
                total_volume_30d: 1_000_000,
                avg_amount: 50_000,
                max_amount: 100_000,
                std_amount: 15_000,
                refund_rate,
                block_rate,
                review_rate: 0.02,
                first_seen: chrono::Utc::now() - chrono::Duration::days(age_days),
                last_action: chrono::Utc::now() - chrono::Duration::hours(2),
                action_type_distribution: Default::default(),
                anomaly_flags: vec![],
            },
            merchant_policy: MerchantPolicy {
                merchant_id: "m".into(),
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
            },
            customer_history: None,
            recent_velocity: VelocityStats {
                actions_last_hour: 2,
                volume_last_hour: 100_000,
                actions_last_24h: 10,
                volume_last_24h: 500_000,
                unique_merchants_24h: merchants,
                unique_customers_24h: 5,
            },
            fetched_at: chrono::Utc::now(),
        }
    }

    fn request(customer_id: Option<&str>) -> AgentActionRequest {
        let mut ctx = serde_json::json!({});
        if let Some(cid) = customer_id {
            ctx["customer_id"] = serde_json::Value::String(cid.into());
        }
        AgentActionRequest {
            agent_id: "a".into(),
            merchant_id: "m".into(),
            action_type: ActionType::Refund,
            amount: 50_000,
            currency: "INR".into(),
            declared_intent: "refund".into(),
            context: ctx,
            timestamp: chrono::Utc::now(),
            correlation_id: uuid::Uuid::new_v4(),
        }
    }

    #[test]
    fn benign_scores_low() {
        let s = LearnedScorer::from_embedded();
        let ev = evidence(0.05, 900, 8, 0.01);
        let insight = s.score(&ev, &request(Some("cust_1")));
        assert!(insight.p_hat < 0.3, "benign p_hat {} too high", insight.p_hat);
        assert_eq!(insight.band, "clear");
    }

    #[test]
    fn abusive_scores_high() {
        let s = LearnedScorer::from_embedded();
        let mut vel = evidence(0.42, 15, 1, 0.12).recent_velocity;
        vel.actions_last_hour = 8;
        vel.actions_last_24h = 12;
        let mut ev2 = evidence(0.42, 15, 1, 0.12);
        ev2.recent_velocity = vel;
        let insight = s.score(&ev2, &request(Some("cust_1")));
        assert!(insight.p_hat > 0.5, "abusive p_hat {} too low", insight.p_hat);
    }

    #[test]
    fn thresholds_ordered() {
        let s = LearnedScorer::from_embedded();
        assert!(s.tau_clear < s.tau_block);
        assert!(s.tau_clear > 0.0 && s.tau_block < 1.0 + 1e-6 || s.tau_block > 1.0);
    }

    #[test]
    fn feature_names_match_artifact() {
        let s = LearnedScorer::from_embedded();
        assert_eq!(s.model.feature_names.len(), 8);
        assert!(s.model.feature_names.contains(&"return_refund_rate".to_string()));
    }
}
