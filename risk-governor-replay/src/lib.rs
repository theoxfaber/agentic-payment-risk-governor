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

#[cfg(test)]
mod tests {
    use super::*;
    use audit_service::InMemoryAuditStore;
    use risk_governor_types::{generate_correlation_id, now_utc, ActionType};

    fn sample_decision(decision_id: Uuid) -> Decision {
        Decision {
            decision_id,
            action: AgentActionRequest {
                agent_id: "agent-01".into(),
                merchant_id: "merchant-001".into(),
                action_type: ActionType::Refund,
                amount: 50_000,
                currency: "INR".into(),
                declared_intent: "refund for order #1".into(),
                context: serde_json::json!({ "customer_id": "cust_1" }),
                timestamp: now_utc(),
                correlation_id: generate_correlation_id(),
            },
            policy_result: PolicyResult {
                verdict: PolicyVerdict::Allow,
                matched_rules: vec![],
                violated_thresholds: vec![],
                evaluated_at: now_utc(),
            },
            risk_result: RiskResult {
                risk_score: 0.07,
                intent_mismatch_score: 0.0,
                features: RiskFeatures {
                    amount_zscore: 0.0,
                    velocity_zscore: 0.0,
                    intent_mismatch_score: 0.0,
                    behavioral_drift_score: 0.0,
                    merchant_risk_score: 0.0,
                    agent_risk_score: 0.05,
                    customer_risk_score: 0.0,
                    time_since_last_action_hours: 2.0,
                    amount_vs_avg_ratio: 1.0,
                },
                model_version: "1.0.0-test".into(),
                evaluated_at: now_utc(),
            },
            decision: DecisionOutcome::Allow,
            model_version: "1.1.0-investigated".into(),
            evidence_snapshot: Evidence {
                agent_history: AgentHistory {
                    agent_id: "agent-01".into(),
                    total_actions_30d: 10,
                    total_volume_30d: 500_000,
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
                recent_velocity: VelocityStats::default(),
                fetched_at: now_utc(),
            },
            created_at: now_utc(),
            human_review: None,
        }
    }

    async fn seed_full_trail(store: &Arc<InMemoryAuditStore>, decision_id: Uuid, decision: &Decision) {
        let svc = AuditService::new(store.clone());
        let mut records = vec![
            AuditRecord {
                record_id: generate_correlation_id(),
                decision_id: Some(decision_id),
                event_type: AuditEventType::ActionRequested,
                payload: serde_json::to_value(&decision.action).unwrap(),
                created_at: now_utc(),
            },
            AuditRecord {
                record_id: generate_correlation_id(),
                decision_id: Some(decision_id),
                event_type: AuditEventType::PolicyEvaluated,
                payload: serde_json::json!({ "policy_version": "2026.08", "verdict": "allow" }),
                created_at: now_utc(),
            },
        ];
        records.push(AuditRecord {
            record_id: generate_correlation_id(),
            decision_id: Some(decision_id),
            event_type: AuditEventType::DecisionMade,
            payload: serde_json::to_value(decision).unwrap(),
            created_at: now_utc(),
        });
        for r in records {
            store.append(r).await.unwrap();
        }
    }

    #[tokio::test]
    async fn replay_reconstructs_decision_from_trail() {
        let store = Arc::new(InMemoryAuditStore::new());
        let id = generate_correlation_id();
        let decision = sample_decision(id);
        seed_full_trail(&store, id, &decision).await;

        let snapshot = ReplayEngine::new(Arc::new(AuditService::new(store)))
            .replay(id)
            .await
            .unwrap();

        assert_eq!(snapshot.decision.decision_id, id);
        assert_eq!(snapshot.decision.decision, DecisionOutcome::Allow);
        assert_eq!(snapshot.risk_model_version, "1.0.0-test");
        assert_eq!(snapshot.policy_version, "2026.08");
        assert_eq!(snapshot.evidence_at_decision.agent_history.agent_id, "agent-01");
        assert_eq!(snapshot.audit_trail.len(), 3);
    }

    #[tokio::test]
    async fn replay_returns_not_found_for_missing_decision() {
        let engine = ReplayEngine::new(Arc::new(AuditService::new(Arc::new(InMemoryAuditStore::new()))));
        let err = engine.replay(generate_correlation_id()).await.unwrap_err();
        assert!(matches!(err, ReplayError::DecisionNotFound(_)));
    }

    #[tokio::test]
    async fn replay_falls_back_to_unknown_policy_version() {
        let store = Arc::new(InMemoryAuditStore::new());
        let id = generate_correlation_id();
        let decision = sample_decision(id);
        // Only the DecisionMade record — no PolicyEvaluated to read a version from.
        store
            .append(AuditRecord {
                record_id: generate_correlation_id(),
                decision_id: Some(id),
                event_type: AuditEventType::DecisionMade,
                payload: serde_json::to_value(&decision).unwrap(),
                created_at: now_utc(),
            })
            .await
            .unwrap();

        let snapshot = ReplayEngine::new(Arc::new(AuditService::new(store)))
            .replay(id)
            .await
            .unwrap();
        assert_eq!(snapshot.policy_version, "unknown");
    }
}
