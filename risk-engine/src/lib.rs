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

    pub async fn score(
        &self,
        request: &AgentActionRequest,
        evidence: &Evidence,
    ) -> Result<RiskResult, RiskEngineError> {
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
    async fn score(
        &self,
        request: &AgentActionRequest,
        evidence: &Evidence,
    ) -> Result<RiskResult, action_service::ActionServiceError> {
        self.score(request, evidence)
            .await
            .map_err(|e| action_service::ActionServiceError::RiskEngine(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use risk_governor_types::{generate_correlation_id, now_utc};

    fn evidence(avg: i64, max: i64, hourly_actions: u32, flags: &[&str]) -> Evidence {
        Evidence {
            agent_history: AgentHistory {
                agent_id: "agent-01".into(),
                total_actions_30d: 720, // exactly 1/hour expected
                total_volume_30d: 720 * avg.max(1),
                avg_amount: avg,
                max_amount: max,
                refund_rate: 0.05,
                block_rate: 0.02,
                review_rate: 0.03,
                first_seen: now_utc() - chrono::Duration::days(90),
                last_action: now_utc() - chrono::Duration::hours(2),
                action_type_distribution: Default::default(),
                anomaly_flags: flags.iter().map(|s| s.to_string()).collect(),
            },
            merchant_policy: MerchantPolicy {
                merchant_id: "merchant-001".into(),
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
                actions_last_hour: hourly_actions,
                ..Default::default()
            },
            fetched_at: now_utc(),
        }
    }

    fn request(amount: i64, intent: &str) -> AgentActionRequest {
        AgentActionRequest {
            agent_id: "agent-01".into(),
            merchant_id: "merchant-001".into(),
            action_type: ActionType::Refund,
            amount,
            currency: "INR".into(),
            declared_intent: intent.into(),
            context: serde_json::json!({}),
            timestamp: now_utc(),
            correlation_id: generate_correlation_id(),
        }
    }

    fn engine() -> RiskEngine {
        RiskEngine::default()
    }

    #[tokio::test]
    async fn low_risk_score_for_normal_request() {
        let r = engine()
            .score(
                &request(50_000, "refund for order #1"),
                &evidence(50_000, 100_000, 5, &[]),
            )
            .await
            .unwrap();
        assert!(r.risk_score < 0.2, "normal action scored {}", r.risk_score);
        assert_eq!(r.intent_mismatch_score, 0.0);
    }

    #[tokio::test]
    async fn high_risk_score_for_anomalous_request() {
        let r = engine()
            .score(
                &request(400_000, "URGENT bypass override"),
                &evidence(50_000, 100_000, 40, &["rapid_fire"]),
            )
            .await
            .unwrap();
        assert!(r.risk_score > 0.4, "anomalous action scored {}", r.risk_score);
    }

    #[tokio::test]
    async fn intent_mismatch_detects_suspicious_keywords() {
        let r = engine()
            .score(&request(50_000, "urgent bypass"), &evidence(50_000, 100_000, 5, &[]))
            .await
            .unwrap();
        // two suspicious keywords + missing "refund" keyword
        assert!(r.intent_mismatch_score >= 0.5);
    }

    #[tokio::test]
    async fn intent_mismatch_zero_on_matching_intent() {
        let r = engine()
            .score(
                &request(50_000, "routine refund for order #42"),
                &evidence(50_000, 100_000, 5, &[]),
            )
            .await
            .unwrap();
        assert_eq!(r.intent_mismatch_score, 0.0);
    }

    #[tokio::test]
    async fn intent_mismatch_catches_amount_contradiction() {
        // declared "small" but amount is large paise value
        let r = engine()
            .score(&request(500_000, "small refund"), &evidence(50_000, 100_000, 5, &[]))
            .await
            .unwrap();
        assert!(r.intent_mismatch_score >= 0.2);
    }

    #[tokio::test]
    async fn behavioral_drift_clamps_to_five() {
        let e = evidence(50_000, 100_000, u32::MAX / 2, &[]);
        let eng = engine();
        let drift = eng.calculate_behavioral_drift(&e.agent_history, &e.recent_velocity);
        assert_eq!(drift, 5.0);
    }

    #[tokio::test]
    async fn scores_are_bounded_in_unit_interval() {
        for (avg, amt, hour) in [(1i64, i64::MAX / 4, 1_000u32), (1, 10, 0)] {
            let r = engine()
                .score(
                    &request(amt, "x".repeat(200).as_str()),
                    &evidence(avg, avg.max(2), hour, &[]),
                )
                .await
                .unwrap();
            assert!((0.0..=1.0).contains(&r.risk_score));
            assert!((0.0..=1.0).contains(&r.intent_mismatch_score));
        }
    }

    #[tokio::test]
    async fn model_version_round_trips() {
        let e = RiskEngine::new("9.9.9-test".into());
        let r = e
            .score(&request(1_000, "refund"), &evidence(50_000, 100_000, 5, &[]))
            .await
            .unwrap();
        assert_eq!(r.model_version, "9.9.9-test");
    }
}
