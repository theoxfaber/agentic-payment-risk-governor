use risk_governor_types::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RiskEngineError {
    #[error("risk scoring failed: {0}")]
    Scoring(String),
}

pub struct RiskEngine {
    model_version: String,
}

impl RiskEngine {
    pub fn new(model_version: String) -> Self {
        Self { model_version }
    }

    pub async fn score(&self, request: &AgentActionRequest, evidence: &Evidence) -> Result<RiskResult, RiskEngineError> {
        let features = self.extract_features(request, evidence);
        let risk_score = self.calculate_risk_score(&features);
        let intent_mismatch_score = self.calculate_intent_mismatch(request, evidence);

        Ok(RiskResult {
            risk_score,
            intent_mismatch_score,
            features,
            model_version: self.model_version.clone(),
            evaluated_at: now_utc(),
        })
    }

    fn extract_features(&self, request: &AgentActionRequest, evidence: &Evidence) -> RiskFeatures {
        let agent = &evidence.agent_history;
        let velocity = &evidence.recent_velocity;

        let amount_zscore = if agent.avg_amount > 0 {
            (request.amount as f64 - agent.avg_amount as f64) / (agent.max_amount.max(1) as f64 * 0.5)
        } else {
            0.0
        };

        let velocity_zscore = if velocity.actions_last_hour > 0 {
            (velocity.actions_last_hour as f64 - 5.0) / 10.0
        } else {
            0.0
        };

        let amount_vs_avg_ratio = if agent.avg_amount > 0 {
            request.amount as f64 / agent.avg_amount as f64
        } else {
            1.0
        };

        let time_since_last_action_hours = (now_utc() - agent.last_action).num_minutes() as f64 / 60.0;

        RiskFeatures {
            amount_zscore: amount_zscore.clamp(-5.0, 5.0),
            velocity_zscore: velocity_zscore.clamp(-5.0, 5.0),
            intent_mismatch_score: 0.0, // calculated separately
            behavioral_drift_score: self.calculate_behavioral_drift(agent, velocity),
            merchant_risk_score: self.calculate_merchant_risk(&evidence.merchant_policy),
            agent_risk_score: self.calculate_agent_risk(agent),
            customer_risk_score: evidence.customer_history.as_ref().map(|c| c.risk_score).unwrap_or(0.0),
            time_since_last_action_hours,
            amount_vs_avg_ratio,
        }
    }

    fn calculate_behavioral_drift(&self, agent: &AgentHistory, velocity: &VelocityStats) -> f64 {
        let mut drift = 0.0;
        if agent.total_actions_30d > 0 {
            let expected_hourly = agent.total_actions_30d as f64 / (30.0 * 24.0);
            if expected_hourly > 0.0 {
                drift = (velocity.actions_last_hour as f64 - expected_hourly).abs() / expected_hourly.max(1.0);
            }
        }
        drift.clamp(0.0, 5.0)
    }

    fn calculate_merchant_risk(&self, policy: &MerchantPolicy) -> f64 {
        let mut risk = 0.0f64;
        if !policy.custom_rules.is_empty() {
            risk += 0.1;
        }
        if policy.blocked_countries.len() > 5 {
            risk += 0.1;
        }
        if policy.velocity_threshold_per_hour < 5 {
            risk += 0.2;
        }
        risk.clamp(0.0, 1.0)
    }

    fn calculate_agent_risk(&self, agent: &AgentHistory) -> f64 {
        let mut risk = 0.0;
        risk += agent.refund_rate * 0.3;
        risk += agent.block_rate * 0.4;
        risk += agent.review_rate * 0.2;
        risk += (agent.anomaly_flags.len() as f64 * 0.1).min(0.5);
        risk.clamp(0.0, 1.0)
    }

    fn calculate_risk_score(&self, features: &RiskFeatures) -> f64 {
        let weights = [
            (features.amount_zscore.abs() / 5.0, 0.20),
            (features.velocity_zscore.abs() / 5.0, 0.15),
            (features.behavioral_drift_score / 5.0, 0.15),
            (features.merchant_risk_score, 0.10),
            (features.agent_risk_score, 0.20),
            (features.customer_risk_score, 0.10),
            ((features.amount_vs_avg_ratio - 1.0).abs().min(5.0) / 5.0, 0.10),
        ];

        let score: f64 = weights.iter().map(|(v, w)| v * w).sum();
        score.clamp(0.0, 1.0)
    }

    fn calculate_intent_mismatch(&self, request: &AgentActionRequest, _evidence: &Evidence) -> f64 {
        let declared = request.declared_intent.to_lowercase();
        let mut mismatch = 0.0f64;

        // Check if intent matches action type
        let action_keyword = match request.action_type {
            ActionType::Refund => "refund",
            ActionType::Payout => "payout",
            ActionType::PaymentLink => "payment link",
            ActionType::Transfer => "transfer",
            ActionType::Capture => "capture",
            ActionType::Void => "void",
        };

        if !declared.contains(action_keyword) {
            mismatch += 0.3;
        }

        // Check for suspicious keywords
        let suspicious = ["urgent", "immediate", "bypass", "override", "emergency", "test", "fake"];
        for word in suspicious {
            if declared.contains(word) {
                mismatch += 0.1;
            }
        }

        // Check amount consistency with intent
        if declared.contains("small") && request.amount > 10000 {
            mismatch += 0.2;
        }
        if declared.contains("large") && request.amount < 1000 {
            mismatch += 0.2;
        }

        mismatch.clamp(0.0, 1.0)
    }
}

impl Default for RiskEngine {
    fn default() -> Self {
        Self::new("1.0.0".to_string())
    }
}

#[async_trait::async_trait]
impl action_service::RiskEngine for RiskEngine {
    async fn score(&self, request: &AgentActionRequest, evidence: &Evidence) -> Result<RiskResult, action_service::ActionServiceError> {
        self.score(request, evidence).await.map_err(|e| action_service::ActionServiceError::RiskEngine(e.to_string()))
    }
}