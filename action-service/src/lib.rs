use risk_governor_correlation::scope_correlation;
use risk_governor_types::*;
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ActionServiceError {
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("policy engine error: {0}")]
    PolicyEngine(String),
    #[error("risk engine error: {0}")]
    RiskEngine(String),
    #[error("evidence service error: {0}")]
    EvidenceService(String),
    #[error("audit service error: {0}")]
    AuditService(String),
    #[error("razorpay gateway error: {0}")]
    RazorpayGateway(String),
}

#[async_trait::async_trait]
pub trait PolicyEngine: Send + Sync {
    async fn evaluate(
        &self,
        request: &AgentActionRequest,
        evidence: &Evidence,
    ) -> Result<PolicyResult, ActionServiceError>;
}

#[async_trait::async_trait]
pub trait RiskEngine: Send + Sync {
    async fn score(&self, request: &AgentActionRequest, evidence: &Evidence) -> Result<RiskResult, ActionServiceError>;
}

/// Evidence plus an explicit degradation flag. Transport-level failures
/// (evidence service down/slow) return benign defaults with a reason instead
/// of erroring: the combiner forces Review on degraded evidence. Application-
/// level failures (unknown merchant) still return Err and fail closed.
#[derive(Debug, Clone)]
pub struct GatheredEvidence {
    pub evidence: Evidence,
    pub degraded_reason: Option<String>,
}

impl GatheredEvidence {
    pub fn fresh(evidence: Evidence) -> Self {
        Self {
            evidence,
            degraded_reason: None,
        }
    }
}

#[async_trait::async_trait]
pub trait EvidenceService: Send + Sync {
    async fn gather(&self, request: &AgentActionRequest) -> Result<GatheredEvidence, ActionServiceError>;
    async fn record_action(&self, request: &AgentActionRequest) -> Result<(), ActionServiceError>;
}

#[async_trait::async_trait]
pub trait AuditService: Send + Sync {
    async fn record(&self, record: AuditRecord) -> Result<(), ActionServiceError>;
}

#[async_trait::async_trait]
pub trait RazorpayGateway: Send + Sync {
    async fn execute(
        &self,
        request: &AgentActionRequest,
        decision_id: Uuid,
    ) -> Result<serde_json::Value, ActionServiceError>;
}

/// Intelligence plane boundary. Returns the combiner-facing summary plus a
/// full serializable payload for the audit trail (GraphAnalyzed event).
#[async_trait::async_trait]
pub trait Investigator: Send + Sync {
    async fn investigate(
        &self,
        request: &AgentActionRequest,
        evidence: &Evidence,
    ) -> Result<(InvestigationSummary, serde_json::Value), ActionServiceError>;
}

pub struct ActionService<P, R, E, A, G>
where
    P: PolicyEngine,
    R: RiskEngine,
    E: EvidenceService,
    A: AuditService,
    G: RazorpayGateway,
{
    policy_engine: Arc<P>,
    risk_engine: Arc<R>,
    evidence_service: Arc<E>,
    audit_service: Arc<A>,
    razorpay_gateway: Arc<G>,
    investigator: Option<Arc<dyn Investigator>>,
}

impl<P, R, E, A, G> ActionService<P, R, E, A, G>
where
    P: PolicyEngine,
    R: RiskEngine,
    E: EvidenceService,
    A: AuditService,
    G: RazorpayGateway,
{
    pub fn new(
        policy_engine: Arc<P>,
        risk_engine: Arc<R>,
        evidence_service: Arc<E>,
        audit_service: Arc<A>,
        razorpay_gateway: Arc<G>,
    ) -> Self {
        Self {
            policy_engine,
            risk_engine,
            evidence_service,
            audit_service,
            razorpay_gateway,
            investigator: None,
        }
    }

    /// Attach the intelligence plane. Optional for backwards compatibility,
    /// mandatory in production configs.
    pub fn with_investigator(mut self, investigator: Arc<dyn Investigator>) -> Self {
        self.investigator = Some(investigator);
        self
    }

    pub async fn process_action(&self, mut request: AgentActionRequest) -> Result<Decision, ActionServiceError> {
        if request.correlation_id.is_nil() {
            request.correlation_id = generate_correlation_id();
        }
        // One ID from here down: every log line in this service AND every
        // downstream bus call (which reads current_correlation_id()) carries
        // the same correlation_id as the decision itself.
        let cid = request.correlation_id;
        scope_correlation(cid, self.process_inner(request)).await
    }

    async fn process_inner(&self, request: AgentActionRequest) -> Result<Decision, ActionServiceError> {
        // Validate BEFORE anything else — every entry path (HTTP, NATS
        // workers, demos, tests) gets the same business-field guarantees:
        // positive amount, non-empty identity/intent. Skipping this let a
        // negative amount flow through the whole pipeline.
        validate_request(&request)?;

        // The decision ID exists from the moment the request arrives — every
        // audit record for this action (including pre-decision evaluations)
        // carries it, so replay reconstructs the FULL trail.
        let decision_id = generate_correlation_id();
        let did = Some(decision_id);

        let gathered = self.evidence_service.gather(&request).await?;
        let evidence = gathered.evidence;
        // Feedback loop: every processed request updates velocity/history.
        self.evidence_service.record_action(&request).await?;

        let mut req_payload =
            serde_json::to_value(&request).map_err(|e| ActionServiceError::Validation(e.to_string()))?;
        if let Some(obj) = req_payload.as_object_mut() {
            obj.insert("input_hash".into(), serde_json::Value::String(request.input_hash()));
        }

        self.audit_service
            .record(AuditRecord {
                record_id: generate_correlation_id(),
                decision_id: did,
                event_type: AuditEventType::ActionRequested,
                payload: req_payload,
                created_at: now_utc(),
                previous_hash: None,
                current_hash: String::new(),
            })
            .await?;

        let policy_result = self.policy_engine.evaluate(&request, &evidence).await?;

        self.audit_service
            .record(AuditRecord {
                record_id: generate_correlation_id(),
                decision_id: did,
                event_type: AuditEventType::PolicyEvaluated,
                payload: serde_json::to_value(&policy_result)
                    .map_err(|e| ActionServiceError::Validation(e.to_string()))?,
                created_at: now_utc(),
                previous_hash: None,
                current_hash: String::new(),
            })
            .await?;

        let risk_result = self.risk_engine.score(&request, &evidence).await?;

        self.audit_service
            .record(AuditRecord {
                record_id: generate_correlation_id(),
                decision_id: did,
                event_type: AuditEventType::RiskScored,
                payload: serde_json::to_value(&risk_result)
                    .map_err(|e| ActionServiceError::Validation(e.to_string()))?,
                created_at: now_utc(),
                previous_hash: None,
                current_hash: String::new(),
            })
            .await?;

        // Intelligence plane: construct/test the risk hypothesis BEFORE the
        // combiner acts, so a high score with weak evidence can't auto-act.
        let investigation: Option<InvestigationSummary> = match &self.investigator {
            Some(inv) => {
                let (summary, payload) = inv.investigate(&request, &evidence).await?;
                self.audit_service
                    .record(AuditRecord {
                        record_id: generate_correlation_id(),
                        decision_id: did,
                        event_type: AuditEventType::GraphAnalyzed,
                        payload,
                        created_at: now_utc(),
                        previous_hash: None,
                        current_hash: String::new(),
                    })
                    .await?;
                Some(summary)
            }
            None => None,
        };

        // Degraded evidence must be visible in the audit trail AND force
        // Review — a downed evidence service otherwise produces benign
        // defaults that score as Allow (silent-allow hazard).
        let mut policy_result = policy_result;
        if let Some(reason) = &gathered.degraded_reason {
            policy_result
                .matched_rules
                .push(format!("evidence_service_unavailable:{reason}"));
        }

        let decision = self.combine_decision(
            decision_id,
            request.clone(),
            evidence.clone(),
            policy_result.clone(),
            risk_result.clone(),
            investigation.as_ref(),
        )?;

        self.audit_service
            .record(AuditRecord {
                record_id: generate_correlation_id(),
                decision_id: Some(decision.decision_id),
                event_type: AuditEventType::DecisionMade,
                payload: serde_json::to_value(&decision).map_err(|e| ActionServiceError::Validation(e.to_string()))?,
                created_at: now_utc(),
                previous_hash: None,
                current_hash: String::new(),
            })
            .await?;

        match decision.decision {
            DecisionOutcome::Allow => {
                let razorpay_response = self.razorpay_gateway.execute(&request, decision.decision_id).await?;

                self.audit_service
                    .record(AuditRecord {
                        record_id: generate_correlation_id(),
                        decision_id: Some(decision.decision_id),
                        event_type: AuditEventType::RazorpayCalled,
                        payload: razorpay_response,
                        created_at: now_utc(),
                        previous_hash: None,
                        current_hash: String::new(),
                    })
                    .await?;
            }
            DecisionOutcome::Review => {
                // Human review queue - will be handled by dashboard
            }
            DecisionOutcome::Block => {
                // Blocked - no further action
            }
        }

        Ok(decision)
    }

    fn combine_decision(
        &self,
        decision_id: Uuid,
        request: AgentActionRequest,
        evidence: Evidence,
        policy_result: PolicyResult,
        risk_result: RiskResult,
        investigation: Option<&InvestigationSummary>,
    ) -> Result<Decision, ActionServiceError> {
        // SAFETY PRINCIPLE (README-bold material):
        //   a risk score alone can never force an automatic action when
        //   evidence quality is insufficient or contradicted.
        //
        // Combiner, in explainability order:
        //   1. Policy hard block (threshold/scope violation) → Block
        //   2. Very high risk score → Block — UNLESS the investigation plane
        //      is conflicted / low-confidence, in which case → Review
        //   3. Investigation contradictions/conflicts without high score → Review
        //   4. Approval threshold OR service-unavailable fail-safes OR elevated
        //      risk → Review
        //   5. Otherwise → Allow

        let mut extra_rules: Vec<String> = Vec::new();
        // Declared intent vs hard fields: a high mismatch score means the
        // agent's stated reason contradicts what it is asking for (missing
        // action keyword, urgency pressure, claimed amount ≠ requested
        // amount). That is deception evidence — it can never BLOCK on its
        // own (too gameable), but it must never sail through as a silent
        // ALLOW either.
        if risk_result.intent_mismatch_score >= 0.5 {
            extra_rules.push("intent_contradiction".into());
        }
        if let Some(inv) = investigation {
            // Unsupported = hypothesis not established → no added friction,
            // EXCEPT the adversarial-evasion case: strong structural linkage
            // with weak/no counter-evidence and no behavioral confirmation.
            // Absence of confirmation is itself suspicious → human review.
            if inv.verdict == InvestigationVerdict::Conflicted {
                extra_rules.push("evidence_contradiction".into());
            } else if inv.verdict == InvestigationVerdict::Unsupported
                && inv.structurally_suspicious
                && inv.counter_weight < 0.25
            {
                extra_rules.push("unconfirmed_structural_linkage".into());
            }
            if inv.evidence_confidence < 0.5 && inv.verdict != InvestigationVerdict::Unsupported {
                extra_rules.push("low_evidence_confidence".into());
            }
        }

        let needs_review = policy_result.matched_rules.iter().any(|r| {
            r == "requires_approval_above_threshold"
                || r.starts_with("policy_engine_unavailable")
                || r.starts_with("evidence_service_unavailable")
        });

        let high_risk = risk_result.risk_score >= 0.8;
        let evidence_unreliable = investigation
            .map(|inv| inv.verdict == InvestigationVerdict::Conflicted || inv.evidence_confidence < 0.5)
            .unwrap_or(false);
        let contradicted = investigation.map(|inv| inv.contradiction_count > 0).unwrap_or(false);

        let decision = if policy_result.verdict == PolicyVerdict::Block {
            DecisionOutcome::Block
        } else if high_risk && evidence_unreliable {
            // The safety property in action.
            DecisionOutcome::Review
        } else if high_risk {
            DecisionOutcome::Block
        } else if contradicted || !extra_rules.is_empty() || needs_review || risk_result.risk_score >= 0.5 {
            DecisionOutcome::Review
        } else {
            DecisionOutcome::Allow
        };

        let mut policy_result = policy_result;
        policy_result.matched_rules.append(&mut extra_rules);

        Ok(Decision {
            decision_id,
            action: request,
            policy_result,
            risk_result,
            learned_insight: None,
            decision,
            model_version: "1.1.0-investigated".to_string(),
            evidence_snapshot: evidence,
            created_at: now_utc(),
            human_review: None,
        })
    }
}

pub fn validate_request(request: &AgentActionRequest) -> Result<(), ActionServiceError> {
    if request.agent_id.is_empty() {
        return Err(ActionServiceError::Validation("agent_id is required".to_string()));
    }
    if request.merchant_id.is_empty() {
        return Err(ActionServiceError::Validation("merchant_id is required".to_string()));
    }
    if request.amount <= 0 {
        return Err(ActionServiceError::Validation("amount must be positive".to_string()));
    }
    if request.currency.is_empty() {
        return Err(ActionServiceError::Validation("currency is required".to_string()));
    }
    let valid_currencies = ["INR", "USD", "EUR", "GBP", "SGD", "AED", "AUD", "CAD"];
    let curr_upper = request.currency.to_uppercase();
    if !valid_currencies.contains(&curr_upper.as_str()) {
        return Err(ActionServiceError::Validation(format!(
            "unsupported or invalid currency '{}': must be ISO 4217 (e.g. INR, USD)",
            request.currency
        )));
    }
    if request.declared_intent.is_empty() {
        return Err(ActionServiceError::Validation(
            "declared_intent is required".to_string(),
        ));
    }
    if request.action_type == ActionType::Refund {
        let pid = request
            .context
            .get("payment_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if pid.is_empty() {
            return Err(ActionServiceError::Validation(
                "payment_id is required in context for refund actions".to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use risk_governor_types::{generate_correlation_id, now_utc};

    // --- input validation ---------------------------------------------

    fn valid_request() -> AgentActionRequest {
        AgentActionRequest {
            agent_id: "agent-01".into(),
            merchant_id: "merchant-001".into(),
            action_type: ActionType::Refund,
            amount: 50_000,
            currency: "INR".into(),
            declared_intent: "refund for order #123".into(),
            context: serde_json::json!({ "customer_id": "cust_1", "payment_id": "pay_test_123" }),
            timestamp: now_utc(),
            correlation_id: generate_correlation_id(),
        }
    }

    #[test]
    fn validate_accepts_valid_request() {
        assert!(validate_request(&valid_request()).is_ok());
    }

    #[test]
    fn validate_rejects_empty_agent_id() {
        let mut r = valid_request();
        r.agent_id = String::new();
        assert!(matches!(validate_request(&r), Err(ActionServiceError::Validation(_))));
    }

    #[test]
    fn validate_rejects_empty_merchant_id() {
        let mut r = valid_request();
        r.merchant_id = String::new();
        assert!(validate_request(&r).is_err());
    }

    #[test]
    fn validate_rejects_zero_amount() {
        let mut r = valid_request();
        r.amount = 0;
        assert!(validate_request(&r).is_err());
    }

    #[test]
    fn validate_rejects_negative_amount() {
        let mut r = valid_request();
        r.amount = -100;
        assert!(validate_request(&r).is_err());
    }

    #[test]
    fn validate_rejects_empty_currency() {
        let mut r = valid_request();
        r.currency = String::new();
        assert!(validate_request(&r).is_err());
    }

    #[test]
    fn validate_rejects_empty_declared_intent() {
        let mut r = valid_request();
        r.declared_intent = String::new();
        assert!(validate_request(&r).is_err());
    }

    // --- combiner behavior (full pipeline with stub planes) ------------

    /// Allow-all policy plane.
    struct AllowAll;

    #[async_trait::async_trait]
    impl PolicyEngine for AllowAll {
        async fn evaluate(&self, _: &AgentActionRequest, _: &Evidence) -> Result<PolicyResult, ActionServiceError> {
            Ok(PolicyResult {
                verdict: PolicyVerdict::Allow,
                matched_rules: vec![],
                violated_thresholds: vec![],
                evaluated_at: now_utc(),
            })
        }
    }

    struct FixedRisk(f64);

    /// Fixed risk + fixed intent-mismatch (for combiner interaction tests).
    struct FixedIntent {
        score: f64,
        mismatch: f64,
    }

    #[async_trait::async_trait]
    impl RiskEngine for FixedIntent {
        async fn score(&self, _: &AgentActionRequest, _: &Evidence) -> Result<RiskResult, ActionServiceError> {
            Ok(RiskResult {
                risk_score: self.score,
                intent_mismatch_score: self.mismatch,
                features: FixedRisk(0.0).dummy_features(),
                model_version: "fixed-intent-test".into(),
                evaluated_at: now_utc(),
            })
        }
    }

    impl FixedRisk {
        fn dummy_features(&self) -> RiskFeatures {
            RiskFeatures {
                amount_zscore: 0.0,
                velocity_zscore: 0.0,
                intent_mismatch_score: 0.0,
                behavioral_drift_score: 0.0,
                merchant_risk_score: 0.0,
                agent_risk_score: 0.0,
                customer_risk_score: 0.0,
                time_since_last_action_hours: 0.0,
                amount_vs_avg_ratio: 1.0,
            }
        }
    }

    #[async_trait::async_trait]
    impl RiskEngine for FixedRisk {
        async fn score(&self, _: &AgentActionRequest, _: &Evidence) -> Result<RiskResult, ActionServiceError> {
            Ok(RiskResult {
                risk_score: self.0,
                intent_mismatch_score: 0.0,
                features: self.dummy_features(),
                model_version: "fixed-test".into(),
                evaluated_at: now_utc(),
            })
        }
    }

    struct EvidenceOk;

    fn benign_evidence(req: &AgentActionRequest) -> Evidence {
        Evidence {
            agent_history: AgentHistory {
                agent_id: req.agent_id.clone(),
                total_actions_30d: 10,
                total_volume_30d: 500_000,
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
            merchant_policy: MerchantPolicy {
                merchant_id: req.merchant_id.clone(),
                max_refund_amount: i64::MAX / 2,
                max_payout_amount: i64::MAX / 2,
                max_payment_link_amount: i64::MAX / 2,
                daily_refund_limit: i64::MAX / 2,
                daily_payout_limit: i64::MAX / 2,
                velocity_threshold_per_hour: u32::MAX,
                allowed_countries: vec![],
                blocked_countries: vec![],
                require_approval_above: i64::MAX / 2,
                custom_rules: vec![],
            },
            customer_history: None,
            recent_velocity: VelocityStats::default(),
            fetched_at: now_utc(),
        }
    }

    #[async_trait::async_trait]
    impl EvidenceService for EvidenceOk {
        async fn gather(&self, req: &AgentActionRequest) -> Result<GatheredEvidence, ActionServiceError> {
            Ok(GatheredEvidence::fresh(benign_evidence(req)))
        }
        async fn record_action(&self, _: &AgentActionRequest) -> Result<(), ActionServiceError> {
            Ok(())
        }
    }

    struct AuditOk;

    #[async_trait::async_trait]
    impl AuditService for AuditOk {
        async fn record(&self, _: AuditRecord) -> Result<(), ActionServiceError> {
            Ok(())
        }
    }

    struct GatewayOk;

    #[async_trait::async_trait]
    impl RazorpayGateway for GatewayOk {
        async fn execute(
            &self,
            _: &AgentActionRequest,
            decision_id: Uuid,
        ) -> Result<serde_json::Value, ActionServiceError> {
            Ok(serde_json::json!({ "id": format!("rfnd_mock_{decision_id}"), "status": "processed" }))
        }
    }

    fn service_with_risk(score: f64) -> ActionService<AllowAll, FixedRisk, EvidenceOk, AuditOk, GatewayOk> {
        ActionService::new(
            Arc::new(AllowAll),
            Arc::new(FixedRisk(score)),
            Arc::new(EvidenceOk),
            Arc::new(AuditOk),
            Arc::new(GatewayOk),
        )
    }

    #[tokio::test]
    async fn low_risk_allows_clean_action_and_executes() {
        let svc = service_with_risk(0.05);
        let d = svc.process_action(valid_request()).await.unwrap();
        assert_eq!(d.decision, DecisionOutcome::Allow);
    }

    #[tokio::test]
    async fn moderate_risk_reviews_not_blocks() {
        let svc = service_with_risk(0.6);
        let d = svc.process_action(valid_request()).await.unwrap();
        assert_eq!(d.decision, DecisionOutcome::Review);
    }

    #[tokio::test]
    async fn high_risk_without_investigation_blocks() {
        // No investigation plane attached: a high risk score acts on its own.
        // The safety property needs the investigator present to soften this —
        // pinned here so removing the combiner's high-risk branch is caught.
        let svc = service_with_risk(0.9);
        let d = svc.process_action(valid_request()).await.unwrap();
        assert_eq!(d.decision, DecisionOutcome::Block);
    }

    #[tokio::test]
    async fn intent_contradiction_forces_review_even_at_low_risk() {
        // The lying-agent signal: risk score is tiny, but the declared intent
        // contradicts the hard fields. Must surface as Review with an audit
        // marker — never a silent ALLOW, and never an auto-BLOCK either.
        let svc: ActionService<AllowAll, FixedIntent, EvidenceOk, AuditOk, GatewayOk> = ActionService::new(
            Arc::new(AllowAll),
            Arc::new(FixedIntent {
                score: 0.01,
                mismatch: 0.6,
            }),
            Arc::new(EvidenceOk),
            Arc::new(AuditOk),
            Arc::new(GatewayOk),
        );
        let d = svc.process_action(valid_request()).await.unwrap();
        assert_eq!(d.decision, DecisionOutcome::Review);
        assert!(d
            .policy_result
            .matched_rules
            .iter()
            .any(|r| r == "intent_contradiction"));
    }

    #[tokio::test]
    async fn mild_intent_mismatch_does_not_add_friction() {
        let svc: ActionService<AllowAll, FixedIntent, EvidenceOk, AuditOk, GatewayOk> = ActionService::new(
            Arc::new(AllowAll),
            Arc::new(FixedIntent {
                score: 0.01,
                mismatch: 0.3,
            }),
            Arc::new(EvidenceOk),
            Arc::new(AuditOk),
            Arc::new(GatewayOk),
        );
        let d = svc.process_action(valid_request()).await.unwrap();
        assert_eq!(d.decision, DecisionOutcome::Allow);
    }

    #[tokio::test]
    async fn correlation_id_generated_when_absent() {
        let svc = service_with_risk(0.05);
        let mut req = valid_request();
        req.correlation_id = Uuid::nil();
        let d = svc.process_action(req).await.unwrap();
        assert!(!d.action.correlation_id.is_nil());
    }

    #[tokio::test]
    async fn degraded_evidence_forces_review() {
        // Simulates the evidence plane answering but flagging degradation:
        // the combiner must refuse silent-allow.
        struct Degraded;
        #[async_trait::async_trait]
        impl EvidenceService for Degraded {
            async fn gather(&self, req: &AgentActionRequest) -> Result<GatheredEvidence, ActionServiceError> {
                let mut g = GatheredEvidence::fresh(benign_evidence(req));
                g.degraded_reason = Some("nats_timeout".into());
                Ok(g)
            }
            async fn record_action(&self, _: &AgentActionRequest) -> Result<(), ActionServiceError> {
                Ok(())
            }
        }
        let svc: ActionService<AllowAll, FixedRisk, Degraded, AuditOk, GatewayOk> = ActionService::new(
            Arc::new(AllowAll),
            Arc::new(FixedRisk(0.01)), // would be a clean ALLOW if degradation were ignored
            Arc::new(Degraded),
            Arc::new(AuditOk),
            Arc::new(GatewayOk),
        );
        let d = svc.process_action(valid_request()).await.unwrap();
        assert_eq!(d.decision, DecisionOutcome::Review);
        assert!(d
            .policy_result
            .matched_rules
            .iter()
            .any(|r| r.starts_with("evidence_service_unavailable")));
    }
}
