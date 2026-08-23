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
