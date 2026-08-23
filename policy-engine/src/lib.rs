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
