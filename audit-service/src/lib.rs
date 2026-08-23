use chrono::Utc;
use risk_governor_types::*;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("audit write failed: {0}")]
    Write(String),
    #[error("decision not found: {0}")]
    DecisionNotFound(String),
}

/// Immutable log of every decision + every event.
/// Source of truth for replay-engine and evaluation-service.
///
/// Phase 1: in-memory append-only vec. Phase 2: Postgres via sqlx
/// with an append-only table (no UPDATE/DELETE grants).
#[async_trait::async_trait]
pub trait AuditStore: Send + Sync {
    async fn append(&self, record: AuditRecord) -> Result<(), AuditError>;
    async fn by_decision(&self, decision_id: Uuid) -> Result<Vec<AuditRecord>, AuditError>;
    async fn all(&self) -> Result<Vec<AuditRecord>, AuditError>;
}

#[derive(Default)]
pub struct InMemoryAuditStore {
    records: RwLock<Vec<AuditRecord>>,
}

impl InMemoryAuditStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl AuditStore for InMemoryAuditStore {
    async fn append(&self, record: AuditRecord) -> Result<(), AuditError> {
        self.records.write().await.push(record);
        Ok(())
    }

    async fn by_decision(&self, decision_id: Uuid) -> Result<Vec<AuditRecord>, AuditError> {
        let records = self.records.read().await;
        Ok(records
            .iter()
            .filter(|r| r.decision_id == Some(decision_id))
            .cloned()
            .collect())
    }

    async fn all(&self) -> Result<Vec<AuditRecord>, AuditError> {
        Ok(self.records.read().await.clone())
    }
}

pub struct AuditService<S: AuditStore> {
    store: Arc<S>,
}

impl<S: AuditStore> AuditService<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    /// Fire-and-forget friendly: errors are logged, never propagated to the caller,
    /// so a slow/full audit sink never blocks or fails a decision.
    pub async fn record(&self, event_type: AuditEventType, decision_id: Option<Uuid>, payload: serde_json::Value) {
        let record = AuditRecord {
            record_id: generate_correlation_id(),
            decision_id,
            event_type,
            payload,
            created_at: Utc::now(),
        };
        if let Err(e) = self.store.append(record).await {
            tracing::error!(?event_type, ?decision_id, "audit append failed: {e}");
        }
    }

    pub async fn trail_for(&self, decision_id: Uuid) -> Result<Vec<AuditRecord>, AuditError> {
        self.store.by_decision(decision_id).await
    }

    pub async fn all_records(&self) -> Result<Vec<AuditRecord>, AuditError> {
        self.store.all().await
    }
}

#[async_trait::async_trait]
impl<S: AuditStore + 'static> action_service::AuditService for AuditService<S> {
    async fn record(&self, record: AuditRecord) -> Result<(), action_service::ActionServiceError> {
        self.store
            .append(record)
            .await
            .map_err(|e| action_service::ActionServiceError::AuditService(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use risk_governor_types::generate_correlation_id;

    #[tokio::test]
    async fn append_and_retrieve_by_decision() {
        let store = Arc::new(InMemoryAuditStore::new());
        let id = generate_correlation_id();
        let svc = AuditService::new(store.clone());
        for event in [
            AuditEventType::ActionRequested,
            AuditEventType::PolicyEvaluated,
            AuditEventType::RiskScored,
            AuditEventType::DecisionMade,
        ] {
            svc.record(event, Some(id), serde_json::json!(null)).await;
        }
        let trail = svc.trail_for(id).await.unwrap();
        assert_eq!(trail.len(), 4);
        assert_eq!(trail[0].event_type, AuditEventType::ActionRequested);
        assert_eq!(trail[3].event_type, AuditEventType::DecisionMade);
    }

    #[tokio::test]
    async fn trail_returns_empty_for_unknown_id() {
        let svc = AuditService::new(Arc::new(InMemoryAuditStore::new()));
        assert!(svc.trail_for(generate_correlation_id()).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn all_records_includes_pre_decision_events() {
        // The unlinkable-trail bug (BUGS.md #1): records emitted before the
        // decision existed carried decision_id None and vanished from replay.
        // They must still land in the global log.
        let store = Arc::new(InMemoryAuditStore::new());
        let svc = AuditService::new(store.clone());
        svc.record(AuditEventType::ActionRequested, None, serde_json::json!(null))
            .await;
        assert_eq!(svc.all_records().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn scoped_records_do_not_leak_across_decisions() {
        let store = Arc::new(InMemoryAuditStore::new());
        let svc = AuditService::new(store.clone());
        let a = generate_correlation_id();
        let b = generate_correlation_id();
        svc.record(AuditEventType::DecisionMade, Some(a), serde_json::json!(null))
            .await;
        svc.record(AuditEventType::DecisionMade, Some(b), serde_json::json!(null))
            .await;
        assert_eq!(svc.trail_for(a).await.unwrap().len(), 1);
        assert_eq!(svc.trail_for(b).await.unwrap().len(), 1);
    }
}
