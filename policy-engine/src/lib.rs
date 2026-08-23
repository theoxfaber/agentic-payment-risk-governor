use risk_governor_types::*;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PolicyEngineError {
    #[error("policy evaluation failed: {0}")]
    Evaluation(String),
}

pub struct PolicyEngine {
    // In Phase 1, this is an in-process library
    // In Phase 2, this becomes a NATS consumer/producer
}

impl PolicyEngine {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn evaluate(
        &self,
        request: &AgentActionRequest,
        evidence: &Evidence,
    ) -> Result<PolicyResult, PolicyEngineError> {
        let mut matched_rules = Vec::new();
        let mut violated_thresholds = Vec::new();

        let policy = &evidence.merchant_policy;

        // Check amount thresholds
        match request.action_type {
            ActionType::Refund => {
                if request.amount > policy.max_refund_amount {
                    violated_thresholds.push(format!(
                        "refund amount {} exceeds max {}",
                        request.amount, policy.max_refund_amount
                    ));
                }
                if request.amount > policy.require_approval_above {
                    matched_rules.push("requires_approval_above_threshold".to_string());
                }
            }
            ActionType::Payout => {
                if request.amount > policy.max_payout_amount {
                    violated_thresholds.push(format!(
                        "payout amount {} exceeds max {}",
                        request.amount, policy.max_payout_amount
                    ));
                }
            }
            ActionType::PaymentLink if request.amount > policy.max_payment_link_amount => {
                violated_thresholds.push(format!(
                    "payment link amount {} exceeds max {}",
                    request.amount, policy.max_payment_link_amount
                ));
            }
            _ => {}
        }

        // Check velocity
        if evidence.recent_velocity.actions_last_hour > policy.velocity_threshold_per_hour {
            violated_thresholds.push(format!(
                "velocity {} exceeds threshold {} per hour",
                evidence.recent_velocity.actions_last_hour, policy.velocity_threshold_per_hour
            ));
        }

        // Check country restrictions (from context)
        if let Some(country) = request.context.get("country").and_then(|v| v.as_str()) {
            if policy.blocked_countries.contains(&country.to_string()) {
                violated_thresholds.push(format!("country {} is blocked", country));
            }
            if !policy.allowed_countries.is_empty() && !policy.allowed_countries.contains(&country.to_string()) {
                violated_thresholds.push(format!("country {} not in allowed list", country));
            }
        }

        // Check custom rules
        for rule in &policy.custom_rules {
            if self.evaluate_custom_rule(rule, request, evidence) {
                matched_rules.push(rule.rule_id.clone());
                if rule.action == PolicyVerdict::Block {
                    violated_thresholds.push(format!("custom rule {} triggered block", rule.rule_id));
                }
            }
        }

        // Check agent anomaly flags
        for flag in &evidence.agent_history.anomaly_flags {
            violated_thresholds.push(format!("agent anomaly: {}", flag));
        }

        let verdict = if violated_thresholds.is_empty() {
            PolicyVerdict::Allow
        } else {
            PolicyVerdict::Block
        };

        Ok(PolicyResult {
            verdict,
            matched_rules,
            violated_thresholds,
            evaluated_at: now_utc(),
        })
    }

    fn evaluate_custom_rule(&self, rule: &CustomRule, request: &AgentActionRequest, evidence: &Evidence) -> bool {
        // Simple condition evaluation - in production, use a proper expression engine
        // For now, support basic conditions
        match rule.condition.as_str() {
            "amount_gt_avg_3x" => request.amount as f64 > evidence.agent_history.avg_amount as f64 * 3.0,
            "refund_rate_gt_10pct" => evidence.agent_history.refund_rate > 0.1,
            "new_agent_lt_7d" => {
                let days_since_first = (now_utc() - evidence.agent_history.first_seen).num_days();
                days_since_first < 7
            }
            "high_velocity" => evidence.recent_velocity.actions_last_hour > 10,
            _ => false,
        }
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl action_service::PolicyEngine for PolicyEngine {
    async fn evaluate(
        &self,
        request: &AgentActionRequest,
        evidence: &Evidence,
    ) -> Result<PolicyResult, action_service::ActionServiceError> {
        self.evaluate(request, evidence)
            .await
            .map_err(|e| action_service::ActionServiceError::PolicyEngine(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use risk_governor_types::{generate_correlation_id, now_utc};

    fn policy() -> MerchantPolicy {
        MerchantPolicy {
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
        }
    }

    fn evidence(p: MerchantPolicy) -> Evidence {
        Evidence {
            agent_history: AgentHistory {
                agent_id: "agent-01".into(),
                total_actions_30d: 30,
                total_volume_30d: 1_500_000,
                avg_amount: 50_000,
                max_amount: 100_000,
                refund_rate: 0.05,
                block_rate: 0.02,
                review_rate: 0.03,
                first_seen: now_utc() - chrono::Duration::days(90),
                last_action: now_utc() - chrono::Duration::hours(2),
                action_type_distribution: Default::default(),
                anomaly_flags: vec![],
            },
            merchant_policy: p,
            customer_history: None,
            recent_velocity: VelocityStats::default(),
            fetched_at: now_utc(),
        }
    }

    fn request(action: ActionType, amount: i64) -> AgentActionRequest {
        AgentActionRequest {
            agent_id: "agent-01".into(),
            merchant_id: "merchant-001".into(),
            action_type: action,
            amount,
            currency: "INR".into(),
            declared_intent: "refund for order #1".into(),
            context: serde_json::json!({}),
            timestamp: now_utc(),
            correlation_id: generate_correlation_id(),
        }
    }

    fn engine() -> PolicyEngine {
        PolicyEngine::new()
    }

    #[tokio::test]
    async fn allows_within_limits() {
        let r = engine()
            .evaluate(&request(ActionType::Refund, 50_000), &evidence(policy()))
            .await
            .unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Allow);
        assert!(r.violated_thresholds.is_empty());
    }

    #[tokio::test]
    async fn blocks_refund_above_max() {
        let r = engine()
            .evaluate(&request(ActionType::Refund, 600_000), &evidence(policy()))
            .await
            .unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Block);
        assert!(r.violated_thresholds[0].contains("refund amount"));
    }

    #[tokio::test]
    async fn blocks_payout_above_max() {
        let r = engine()
            .evaluate(&request(ActionType::Payout, 2_000_000), &evidence(policy()))
            .await
            .unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Block);
        assert!(r.violated_thresholds[0].contains("payout amount"));
    }

    #[tokio::test]
    async fn blocks_payment_link_above_max() {
        let r = engine()
            .evaluate(&request(ActionType::PaymentLink, 300_000), &evidence(policy()))
            .await
            .unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Block);
        assert!(r.violated_thresholds[0].contains("payment link amount"));
    }

    #[tokio::test]
    async fn flags_approval_threshold_without_blocking() {
        // Above require_approval_above but under the hard cap: the rule is
        // MATCHED (review), not a threshold violation (block).
        let r = engine()
            .evaluate(&request(ActionType::Refund, 150_000), &evidence(policy()))
            .await
            .unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Allow);
        assert!(r
            .matched_rules
            .contains(&"requires_approval_above_threshold".to_string()));
    }

    #[tokio::test]
    async fn flags_velocity_breach() {
        let mut p = policy();
        p.velocity_threshold_per_hour = 5;
        let mut e = evidence(p);
        e.recent_velocity.actions_last_hour = 7;
        let r = engine()
            .evaluate(&request(ActionType::Refund, 10_000), &e)
            .await
            .unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Block);
        assert!(r.violated_thresholds[0].contains("velocity"));
    }

    #[tokio::test]
    async fn blocks_blocked_country() {
        let mut p = policy();
        p.blocked_countries = vec!["KP".into()];
        let mut req = request(ActionType::Refund, 10_000);
        req.context["country"] = serde_json::json!("KP");
        let r = engine().evaluate(&req, &evidence(p)).await.unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Block);
        assert!(r.violated_thresholds[0].contains("blocked"));
    }

    #[tokio::test]
    async fn allowlist_rejects_unlisted_country() {
        let mut p = policy();
        p.allowed_countries = vec!["IN".into()];
        let mut req = request(ActionType::Refund, 10_000);
        req.context["country"] = serde_json::json!("US");
        let r = engine().evaluate(&req, &evidence(p)).await.unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Block);
        assert!(r.violated_thresholds[0].contains("not in allowed list"));
    }

    #[tokio::test]
    async fn evaluates_custom_rule_amount_gt_avg_3x() {
        let mut p = policy();
        p.custom_rules = vec![CustomRule {
            rule_id: "big_spend".into(),
            condition: "amount_gt_avg_3x".into(),
            action: PolicyVerdict::Allow, // matched rule, no block
            description: "amount over 3x agent average".into(),
        }];
        let r = engine()
            .evaluate(&request(ActionType::Refund, 200_000), &evidence(p))
            .await
            .unwrap();
        assert!(r.matched_rules.contains(&"big_spend".to_string()));
    }

    #[tokio::test]
    async fn anomaly_flags_block_the_action() {
        let mut e = evidence(policy());
        e.agent_history.anomaly_flags = vec!["rapid_fire".into()];
        let r = engine()
            .evaluate(&request(ActionType::Refund, 50_000), &e)
            .await
            .unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Block);
        assert!(r.violated_thresholds[0].contains("agent anomaly: rapid_fire"));
    }
}
