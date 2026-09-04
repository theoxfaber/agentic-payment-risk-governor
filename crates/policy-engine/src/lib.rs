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

        if let Some(fri) = policy.fri_score {
            match fri {
                0..=33 => {}
                34..=66 if policy.risk_tier == RiskTier::Standard => violated_thresholds.push(format!(
                    "FRI medium ({fri}) requires risk_tier >= Medium — RBI CDD missing (fail-closed)"
                )),
                67..=89 => {
                    if policy.risk_tier == RiskTier::Standard || policy.risk_tier == RiskTier::Medium {
                        violated_thresholds.push(format!(
                            "FRI high ({fri}) requires EDD — risk_tier {:?} insufficient",
                            policy.risk_tier
                        ));
                    }
                }
                90..=100 => violated_thresholds.push(format!("FRI VeryHigh ({fri}) — DoT block, RBI EDD/STR required")),
                _ => violated_thresholds.push(format!("FRI score {fri} out of range")),
            }
        }
        if policy.pmla_retention_days < 1825 {
            violated_thresholds.push(format!(
                "PMLA retention {}d < 1825d — RBI Master KYC Jun 12 2025 violation",
                policy.pmla_retention_days
            ));
        }

        // Check amount thresholds — all amounts are integer paise (i64), no floats
        match request.action_type {
            ActionType::Refund => {
                if request.amount <= 0 {
                    violated_thresholds.push(format!(
                        "refund amount {} must be positive integer paise",
                        request.amount
                    ));
                }
                // Ground truth: Razorpay payment_snapshot (fetched via GET /v1/payments/{id}) is authoritative.
                // Context-declared payment_state/captured_paise is treated as unverified hint only.
                if let Some(snap) = &evidence.payment_snapshot {
                    if snap.payment_id != request.context.get("payment_id").and_then(|v| v.as_str()).unwrap_or("") {
                        violated_thresholds.push(format!(
                            "payment_id mismatch: request {} vs verified {}",
                            request.context.get("payment_id").and_then(|v| v.as_str()).unwrap_or(""),
                            snap.payment_id
                        ));
                    }
                    if !snap.captured || !snap.status.eq_ignore_ascii_case("captured") {
                        violated_thresholds.push(format!(
                            "verified payment {} status '{}' is not captured — refund requires captured (Razorpay ground truth)",
                            snap.payment_id, snap.status
                        ));
                    }
                    let captured = snap.captured_amount.unwrap_or(snap.amount);
                    let refunded = snap.refunded_amount.unwrap_or(0);
                    let available = captured.saturating_sub(refunded);
                    if request.amount > available {
                        violated_thresholds.push(format!(
                            "refund amount {} exceeds verified available balance {} (captured {} - refunded {}) from Razorpay",
                            request.amount, available, captured, refunded
                        ));
                    }
                } else {
                    // No verified snapshot (MockGateway or fetch failed): fall back to self-declared context, but mark as unverified.
                    match request
                        .context
                        .get("payment_state")
                        .or_else(|| request.context.get("paymentStatus"))
                        .or_else(|| request.context.get("payment_status"))
                        .and_then(|v| v.as_str())
                    {
                        Some(state) if state.eq_ignore_ascii_case("captured") => {}
                        Some(state) => violated_thresholds.push(format!(
                            "payment state '{}' is not captured — refund requires captured (unverified, no Razorpay snapshot)",
                            state
                        )),
                        None => violated_thresholds
                            .push("missing payment_state — refund requires captured (fail-closed)".into()),
                    }
                    let captured = Self::extract_paise(
                        &request.context,
                        &["captured_paise", "captured_amount", "amount_captured"],
                    );
                    let refunded = Self::extract_paise(
                        &request.context,
                        &["refunded_paise", "refunded_amount", "amount_refunded"],
                    )
                    .unwrap_or(0);
                    match captured {
                        Some(cap) => {
                            let available = cap.saturating_sub(refunded);
                            if request.amount > available {
                                violated_thresholds.push(format!(
                                    "refund amount {} exceeds available balance {} (captured {} - refunded {}) (unverified)",
                                    request.amount, available, cap, refunded
                                ));
                            }
                        }
                        None => violated_thresholds
                            .push("missing captured_paise — refund requires captured amount (fail-closed)".into()),
                    }
                    // In production with live gateway, missing snapshot should be REVIEW, not silent ALLOW.
                    if request.context.get("payment_id").and_then(|v| v.as_str()).is_some() {
                        matched_rules.push("unverified_payment_snapshot".into());
                    }
                }
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
                if request.amount <= 0 {
                    violated_thresholds.push(format!(
                        "payout amount {} must be positive integer paise",
                        request.amount
                    ));
                }
                if request.amount > policy.max_payout_amount {
                    violated_thresholds.push(format!(
                        "payout amount {} exceeds max {}",
                        request.amount, policy.max_payout_amount
                    ));
                }
                if request.amount > policy.require_approval_above {
                    matched_rules.push("requires_approval_above_threshold".to_string());
                }
            }
            ActionType::PaymentLink if request.amount > policy.max_payment_link_amount => {
                violated_thresholds.push(format!(
                    "payment link amount {} exceeds max {}",
                    request.amount, policy.max_payment_link_amount
                ));
            }
            ActionType::PaymentLink => {
                if request.amount <= 0 {
                    violated_thresholds.push(format!(
                        "payment link amount {} must be positive integer paise",
                        request.amount
                    ));
                }
                if request.amount > policy.require_approval_above {
                    matched_rules.push("requires_approval_above_threshold".to_string());
                }
            }
            // Transfer/Capture/Void move or finalize money with no dedicated
            // Razorpay endpoint wired in the gateway (which fails closed) — the
            // policy plane still bounds them so a future wiring cannot pass
            // unbounded amounts: positive paise, payout-scale cap, approval
            // marker above the review threshold.
            ActionType::Transfer | ActionType::Capture | ActionType::Void => {
                if request.amount <= 0 {
                    violated_thresholds.push(format!(
                        "{:?} amount {} must be positive integer paise",
                        request.action_type, request.amount
                    ));
                }
                if request.amount > policy.max_payout_amount {
                    violated_thresholds.push(format!(
                        "{:?} amount {} exceeds max {}",
                        request.action_type, request.amount, policy.max_payout_amount
                    ));
                }
                if request.amount > policy.require_approval_above {
                    matched_rules.push("requires_approval_above_threshold".to_string());
                }
            }
        }

        // Check velocity
        if evidence.recent_velocity.actions_last_hour > policy.velocity_threshold_per_hour {
            violated_thresholds.push(format!(
                "velocity {} exceeds threshold {} per hour",
                evidence.recent_velocity.actions_last_hour, policy.velocity_threshold_per_hour
            ));
        }

        // Check country restrictions (from context) — case-insensitive
        // ("KP" vs "kp" must not bypass a block). A missing country with ANY
        // geo policy configured fails closed: otherwise an agent omits the
        // field and skips every geo gate (fail-open).
        match request.context.get("country").and_then(|v| v.as_str()) {
            Some(country) => {
                let norm = country.trim().to_uppercase();
                let blocked = policy.blocked_countries.iter().any(|c| c.trim().to_uppercase() == norm);
                if blocked {
                    violated_thresholds.push(format!("country {} is blocked", country));
                }
                if !policy.allowed_countries.is_empty()
                    && !policy.allowed_countries.iter().any(|c| c.trim().to_uppercase() == norm)
                {
                    violated_thresholds.push(format!("country {} not in allowed list", country));
                }
            }
            None => {
                if !policy.allowed_countries.is_empty() || !policy.blocked_countries.is_empty() {
                    violated_thresholds.push(
                        "missing country — geo restrictions configured, cannot verify jurisdiction (fail-closed)"
                            .into(),
                    );
                }
            }
        }

        // Check custom rules
        for rule in &policy.custom_rules {
            match self.evaluate_custom_rule(rule, request, evidence) {
                CustomRuleOutcome::Triggered => {
                    matched_rules.push(rule.rule_id.clone());
                    if rule.action == PolicyVerdict::Block {
                        violated_thresholds.push(format!("custom rule {} triggered block", rule.rule_id));
                    }
                }
                CustomRuleOutcome::NotTriggered => {}
                // A condition string we don't recognize is a MISCONFIGURATION,
                // not a pass. Silently evaluating to false fails OPEN — the
                // opposite of what a blocking rule exists for.
                CustomRuleOutcome::UnknownCondition(c) => violated_thresholds.push(format!(
                    "custom rule {} has unknown condition '{}' (fail-closed)",
                    rule.rule_id, c
                )),
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

    fn parse_paise(s: &str) -> Option<i64> {
        // Integer paise ONLY. Decimal strings are rejected outright rather than
        // stripped: stripping "." turns "100.50" into 10050 (right by accident)
        // but makes "100" (₹1) and "100.00" (₹100) ambiguous inputs that must
        // never silently mean different things. A leading "-" is allowed for
        // negatives (rejected downstream by the positivity gates); interior
        // "-" is malformed.
        let t = s.trim();
        if t.contains('.') {
            return None;
        }
        let body = t.strip_prefix('-').unwrap_or(t);
        if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        t.parse::<i64>().ok()
    }

    fn extract_paise(ctx: &serde_json::Value, keys: &[&str]) -> Option<i64> {
        for k in keys {
            if let Some(v) = ctx.get(*k) {
                if let Some(n) = v.as_i64() {
                    return Some(n);
                }
                if let Some(s) = v.as_str() {
                    if let Some(n) = Self::parse_paise(s) {
                        return Some(n);
                    }
                }
                if let Some(n) = v.as_u64() {
                    if let Ok(conv) = i64::try_from(n) {
                        return Some(conv);
                    }
                }
            }
            if let Some(obj) = ctx.get("payment").and_then(|p| p.get(*k)) {
                if let Some(n) = obj.as_i64() {
                    return Some(n);
                }
                if let Some(s) = obj.as_str() {
                    if let Some(n) = Self::parse_paise(s) {
                        return Some(n);
                    }
                }
                if let Some(n) = obj.as_u64() {
                    if let Ok(conv) = i64::try_from(n) {
                        return Some(conv);
                    }
                }
            }
        }
        None
    }

    fn evaluate_custom_rule(
        &self,
        rule: &CustomRule,
        request: &AgentActionRequest,
        evidence: &Evidence,
    ) -> CustomRuleOutcome {
        // Simple condition evaluation - in production, use a proper expression engine.
        // Unknown conditions are a misconfiguration → fail CLOSED (UnknownCondition),
        // never silently false.
        match rule.condition.as_str() {
            "amount_gt_avg_3x" => {
                let avg = evidence.agent_history.avg_amount;
                if avg <= 0 {
                    CustomRuleOutcome::NotTriggered
                } else if let Some(thresh) = avg.checked_mul(3) {
                    if request.amount > thresh {
                        CustomRuleOutcome::Triggered
                    } else {
                        CustomRuleOutcome::NotTriggered
                    }
                } else {
                    CustomRuleOutcome::Triggered
                }
            }
            "refund_rate_gt_10pct" => {
                if evidence.agent_history.refund_rate > 0.1 {
                    CustomRuleOutcome::Triggered
                } else {
                    CustomRuleOutcome::NotTriggered
                }
            }
            "new_agent_lt_7d" => {
                let days_since_first = (now_utc() - evidence.agent_history.first_seen).num_days();
                if days_since_first < 7 {
                    CustomRuleOutcome::Triggered
                } else {
                    CustomRuleOutcome::NotTriggered
                }
            }
            "high_velocity" => {
                if evidence.recent_velocity.actions_last_hour > 10 {
                    CustomRuleOutcome::Triggered
                } else {
                    CustomRuleOutcome::NotTriggered
                }
            }
            unknown => CustomRuleOutcome::UnknownCondition(unknown.to_string()),
        }
    }
}

/// Tri-state result of evaluating one custom rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomRuleOutcome {
    /// Condition evaluated true — the rule fired.
    Triggered,
    /// Condition evaluated false — normal path.
    NotTriggered,
    /// The condition string is not recognized: a policy misconfiguration.
    /// The caller must treat this as a violation (fail closed).
    UnknownCondition(String),
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
            risk_tier: RiskTier::Standard,
            pmla_retention_days: 1825,
            fri_score: None,
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
                std_amount: 15_000,
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
            payment_snapshot: None,
            fetched_at: now_utc(),
        }
    }

    fn request(action: ActionType, amount: i64) -> AgentActionRequest {
        let ctx = match action {
            ActionType::Refund => serde_json::json!({
                "payment_state": "captured",
                "captured_paise": 500000,
                "refunded_paise": 0
            }),
            _ => serde_json::json!({}),
        };
        AgentActionRequest {
            agent_id: "agent-01".into(),
            merchant_id: "merchant-001".into(),
            action_type: action,
            amount,
            currency: "INR".into(),
            declared_intent: "refund for order #1".into(),
            context: ctx,
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
    async fn missing_country_fails_closed_when_geo_configured() {
        // Omitting `country` must not skip geo gates when the merchant
        // configures any allow/block list.
        for lists in [(vec!["IN".to_string()], vec![]), (vec![], vec!["KP".to_string()])] {
            let mut p = policy();
            p.allowed_countries = lists.0;
            p.blocked_countries = lists.1;
            let req = request(ActionType::Refund, 10_000); // no country in ctx
            let r = engine().evaluate(&req, &evidence(p)).await.unwrap();
            assert_eq!(r.verdict, PolicyVerdict::Block);
            assert!(r.violated_thresholds.iter().any(|t| t.contains("missing country")));
        }
    }

    #[tokio::test]
    async fn missing_country_passes_when_no_geo_configured() {
        let req = request(ActionType::Refund, 10_000); // no country, no lists
        let r = engine().evaluate(&req, &evidence(policy())).await.unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Allow);
    }

    #[tokio::test]
    async fn transfer_capture_void_are_bounded() {
        // Previously zero-checked: over-cap Transfer/Capture/Void must Block,
        // and large ones carry the approval marker for the combiner's Review.
        for action in [ActionType::Transfer, ActionType::Capture, ActionType::Void] {
            let r = engine()
                .evaluate(&request(action, 2_000_000), &evidence(policy()))
                .await
                .unwrap();
            assert_eq!(r.verdict, PolicyVerdict::Block, "{action:?} over cap must Block");
            let r = engine()
                .evaluate(&request(action, 150_000), &evidence(policy()))
                .await
                .unwrap();
            assert!(
                r.matched_rules
                    .contains(&"requires_approval_above_threshold".to_string()),
                "{action:?} above threshold must carry approval marker"
            );
        }
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
    async fn unknown_custom_rule_condition_fails_closed() {
        // A typo'd condition string must BLOCK, never silently pass —
        // fail-open here would defeat the rule's entire purpose.
        let mut p = policy();
        p.custom_rules = vec![CustomRule {
            rule_id: "misconfigured".into(),
            condition: "amount_gt_averge_3x".into(), // typo
            action: PolicyVerdict::Block,
            description: "typo'd condition".into(),
        }];
        let r = engine()
            .evaluate(&request(ActionType::Refund, 1_000), &evidence(p))
            .await
            .unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Block);
        assert!(r.violated_thresholds[0].contains("fail-closed"));
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

    #[tokio::test]
    async fn missing_payment_state_fails_closed() {
        let mut req = request(ActionType::Refund, 10_000);
        req.context = serde_json::json!({ "captured_paise": 500000 });
        let r = engine().evaluate(&req, &evidence(policy())).await.unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Block);
        assert!(r
            .violated_thresholds
            .iter()
            .any(|t| t.contains("missing payment_state")));
    }

    #[tokio::test]
    async fn missing_captured_paise_fails_closed() {
        let mut req = request(ActionType::Refund, 10_000);
        req.context = serde_json::json!({ "payment_state": "captured" });
        let r = engine().evaluate(&req, &evidence(policy())).await.unwrap();
        assert_eq!(r.verdict, PolicyVerdict::Block);
        assert!(r
            .violated_thresholds
            .iter()
            .any(|t| t.contains("missing captured_paise")));
    }

    #[test]
    fn parse_paise_accepts_plain_integers_only() {
        assert_eq!(PolicyEngine::parse_paise("500000"), Some(500_000));
        assert_eq!(PolicyEngine::parse_paise("  2500  "), Some(2_500));
        assert_eq!(PolicyEngine::parse_paise("-100"), Some(-100));
        // Decimals are rejected, never stripped: "100" (₹1) vs "100.00" (₹100)
        // must not silently diverge.
        assert_eq!(PolicyEngine::parse_paise("100.50"), None);
        assert_eq!(PolicyEngine::parse_paise("100.00"), None);
        assert_eq!(PolicyEngine::parse_paise("1,000"), None);
        assert_eq!(PolicyEngine::parse_paise("12-34"), None);
        assert_eq!(PolicyEngine::parse_paise(""), None);
        assert_eq!(PolicyEngine::parse_paise("lots 500000"), None);
    }
}
