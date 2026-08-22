use audit_service::{AuditService, AuditStore};
use risk_governor_types::*;
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("decision not found in audit log: {0}")]
    DecisionNotFound(Uuid),
    #[error("incomplete audit trail for decision {0}: missing {1}")]
    IncompleteTrail(Uuid, &'static str),
}

/// Read-only reconstruction of exactly what evidence/features/policy/model
/// version produced a given decision. A library, not a service — used directly
/// by dashboard and evaluation-service.
pub struct ReplayEngine<S: AuditStore> {
    audit: Arc<AuditService<S>>,
}

impl<S: AuditStore + 'static> ReplayEngine<S> {
    pub fn new(audit: Arc<AuditService<S>>) -> Self {
        Self { audit }
    }

    pub async fn replay(&self, decision_id: Uuid) -> Result<ReplaySnapshot, ReplayError> {
        let trail = self
            .audit
            .trail_for(decision_id)
            .await
            .map_err(|_| ReplayError::IncompleteTrail(decision_id, "unreachable audit store"))?;

        let decision_record = trail
            .iter()
            .find(|r| r.event_type == AuditEventType::DecisionMade)
            .ok_or(ReplayError::DecisionNotFound(decision_id))?;

        let decision: Decision = serde_json::from_value(decision_record.payload.clone())
            .map_err(|_| ReplayError::IncompleteTrail(decision_id, "corrupt decision payload"))?;

        let policy_version = trail
            .iter()
            .find(|r| r.event_type == AuditEventType::PolicyEvaluated)
            .map(|r| {
                r.payload
                    .get("policy_version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string()
            })
            .unwrap_or_else(|| "unknown".into());

        let risk_model_version = decision.risk_result.model_version.clone();
        let evidence_at_decision = decision.evidence_snapshot.clone();

        Ok(ReplaySnapshot {
            decision,
            policy_version,
            risk_model_version,
            evidence_at_decision,
            audit_trail: trail,
        })
    }
}