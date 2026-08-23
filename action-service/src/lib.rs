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
    async fn evaluate(&self, request: &AgentActionRequest, evidence: &Evidence) -> Result<PolicyResult, ActionServiceError>;
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
        Self { evidence, degraded_reason: None }
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
    async fn execute(&self, request: &AgentActionRequest, decision_id: Uuid) -> Result<serde_json::Value, ActionServiceError>;
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
        // The decision ID exists from the moment the request arrives — every
        // audit record for this action (including pre-decision evaluations)
        // carries it, so replay reconstructs the FULL trail.
        let decision_id = generate_correlation_id();
        let did = Some(decision_id);

        let gathered = self.evidence_service.gather(&request).await?;
        let evidence = gathered.evidence;
        // Feedback loop: every processed request updates velocity/history.
        self.evidence_service.record_action(&request).await?;

        self.audit_service.record(AuditRecord {
            record_id: generate_correlation_id(),
            decision_id: did,
            event_type: AuditEventType::ActionRequested,
            payload: serde_json::to_value(&request).map_err(|e| ActionServiceError::Validation(e.to_string()))?,
            created_at: now_utc(),
        }).await?;

        let policy_result = self.policy_engine.evaluate(&request, &evidence).await?;

        self.audit_service.record(AuditRecord {
            record_id: generate_correlation_id(),
            decision_id: did,
            event_type: AuditEventType::PolicyEvaluated,
            payload: serde_json::to_value(&policy_result).map_err(|e| ActionServiceError::Validation(e.to_string()))?,
            created_at: now_utc(),
        }).await?;

        let risk_result = self.risk_engine.score(&request, &evidence).await?;

        self.audit_service.record(AuditRecord {
            record_id: generate_correlation_id(),
            decision_id: did,
            event_type: AuditEventType::RiskScored,
            payload: serde_json::to_value(&risk_result).map_err(|e| ActionServiceError::Validation(e.to_string()))?,
            created_at: now_utc(),
        }).await?;

        // Intelligence plane: construct/test the risk hypothesis BEFORE the
        // combiner acts, so a high score with weak evidence can't auto-act.
        let investigation: Option<InvestigationSummary> = match &self.investigator {
            Some(inv) => {
                let (summary, payload) = inv.investigate(&request, &evidence).await?;
                self.audit_service.record(AuditRecord {
                    record_id: generate_correlation_id(),
                    decision_id: did,
                    event_type: AuditEventType::GraphAnalyzed,
                    payload,
                    created_at: now_utc(),
                }).await?;
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

        let decision = self.combine_decision(decision_id, request.clone(), evidence.clone(), policy_result.clone(), risk_result.clone(), investigation.as_ref())?;

        self.audit_service.record(AuditRecord {
            record_id: generate_correlation_id(),
            decision_id: Some(decision.decision_id),
            event_type: AuditEventType::DecisionMade,
            payload: serde_json::to_value(&decision).map_err(|e| ActionServiceError::Validation(e.to_string()))?,
            created_at: now_utc(),
        }).await?;

        match decision.decision {
            DecisionOutcome::Allow => {
                let razorpay_response = self.razorpay_gateway.execute(&request, decision.decision_id).await?;

                self.audit_service.record(AuditRecord {
                    record_id: generate_correlation_id(),
                    decision_id: Some(decision.decision_id),
                    event_type: AuditEventType::RazorpayCalled,
                    payload: razorpay_response,
                    created_at: now_utc(),
                }).await?;
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
        let contradicted = investigation
            .map(|inv| inv.contradiction_count > 0)
            .unwrap_or(false);

        let decision = if policy_result.verdict == PolicyVerdict::Block {
            DecisionOutcome::Block
        } else if high_risk && evidence_unreliable {
            // The safety property in action.
            DecisionOutcome::Review
        } else if high_risk {
            DecisionOutcome::Block
        } else if contradicted
            || !extra_rules.is_empty()
            || needs_review
            || risk_result.risk_score >= 0.5
        {
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
    if request.declared_intent.is_empty() {
        return Err(ActionServiceError::Validation("declared_intent is required".to_string()));
    }
    Ok(())
}