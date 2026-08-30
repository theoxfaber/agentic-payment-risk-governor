use chrono::Utc;
use hmac::{Hmac, Mac};
use risk_governor_types::*;
use sha2::Sha256;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

pub fn anchor_signature(head_hash: &str, key: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac key");
    mac.update(head_hash.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn verify_anchor_signature(head_hash: &str, signature: &str, key: &[u8]) -> bool {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac key");
    mac.update(head_hash.as_bytes());
    match hex::decode(signature.trim()) {
        Ok(sig) => mac.verify_slice(&sig).is_ok(),
        Err(_) => false,
    }
}

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
    last_hash: RwLock<Option<String>>,
}

impl InMemoryAuditStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl AuditStore for InMemoryAuditStore {
    async fn append(&self, mut record: AuditRecord) -> Result<(), AuditError> {
        let mut last = self.last_hash.write().await;
        if record.current_hash.is_empty() {
            record.previous_hash = last.clone();
            record.current_hash = AuditRecord::compute_hash(
                record.record_id,
                record.decision_id,
                record.event_type,
                &record.payload,
                record.created_at,
                record.previous_hash.as_deref(),
            );
        }
        *last = Some(record.current_hash.clone());
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

    pub fn redact_payload(v: serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(mut m) => {
                let mut out = serde_json::Map::new();
                for (k, val) in m.into_iter() {
                    if ["email", "phone", "customer_phone", "customer_email"].contains(&k.as_str()) {
                        out.insert(k, serde_json::Value::String("***".into()));
                    } else if k == "payment_id" {
                        if let Some(s) = val.as_str() {
                            use sha2::{Digest, Sha256};
                            let mut h = Sha256::new();
                            h.update(s.as_bytes());
                            out.insert(
                                "payment_id_sha256".into(),
                                serde_json::Value::String(hex::encode(h.finalize())),
                            );
                        }
                    } else {
                        out.insert(k.clone(), Self::redact_payload(val));
                    }
                }
                serde_json::Value::Object(out)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(Self::redact_payload).collect())
            }
            other => other,
        }
    }

    /// Fire-and-forget friendly: errors are logged, never propagated to the caller,
    /// so a slow/full audit sink never blocks or fails a decision.
    /// DPDP Act: PII is redacted before append — raw `payment_id`/`email`/`phone` never hits the chain.
    pub async fn record(&self, event_type: AuditEventType, decision_id: Option<Uuid>, payload: serde_json::Value) {
        let payload = Self::redact_payload(payload);
        let record_id = generate_correlation_id();
        let created_at = Utc::now();
        let record = AuditRecord {
            record_id,
            decision_id,
            event_type,
            payload,
            created_at,
            previous_hash: None,
            current_hash: String::new(),
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

    /// Verifies the cryptographic tamper-evident integrity of a contiguous audit record chain.
    /// Use for global `all()` ordering — it enforces previous_hash linkage.
    pub fn verify_chain(records: &[AuditRecord]) -> Result<(), String> {
        let mut prev_hash: Option<String> = None;
        for (i, r) in records.iter().enumerate() {
            let expected_hash = AuditRecord::compute_hash(
                r.record_id,
                r.decision_id,
                r.event_type,
                &r.payload,
                r.created_at,
                r.previous_hash.as_deref(),
            );
            if r.current_hash != expected_hash {
                return Err(format!(
                    "tamper detected at index {i} (record_id {}): hash mismatch (got {}, expected {})",
                    r.record_id, r.current_hash, expected_hash
                ));
            }
            if r.previous_hash != prev_hash && i > 0 {
                return Err(format!(
                    "chain broken at index {i}: previous_hash ({:?}) does not match prior record's hash ({:?})",
                    r.previous_hash, prev_hash
                ));
            }
            prev_hash = Some(r.current_hash.clone());
        }
        Ok(())
    }

    /// Verifies record-hash integrity for a filtered trail (e.g. by_decision).
    /// Checks each record's hash but not cross-record linkage, since the filter
    /// removes the global predecessor.
    pub fn verify_records(records: &[AuditRecord]) -> Result<(), String> {
        for (i, r) in records.iter().enumerate() {
            let expected = AuditRecord::compute_hash(
                r.record_id,
                r.decision_id,
                r.event_type,
                &r.payload,
                r.created_at,
                r.previous_hash.as_deref(),
            );
            if r.current_hash != expected {
                return Err(format!(
                    "tamper detected at index {i} (record_id {}): hash mismatch (got {}, expected {})",
                    r.record_id, r.current_hash, expected
                ));
            }
        }
        Ok(())
    }

    pub fn chain_head(records: &[AuditRecord]) -> Option<String> {
        records.last().map(|r| r.current_hash.clone())
    }

    pub fn anchor_head(head_hash: &str, key: &[u8]) -> String {
        anchor_signature(head_hash, key)
    }

    pub fn verify_chain_with_anchor(records: &[AuditRecord], key: &[u8], signature: &str) -> Result<(), String> {
        Self::verify_chain(records)?;
        let head = Self::chain_head(records).ok_or_else(|| "empty chain has no head".to_string())?;
        if !verify_anchor_signature(&head, signature, key) {
            return Err("anchor signature mismatch — chain was recomputed or key is wrong".into());
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl<S: AuditStore + 'static> action_service::AuditService for AuditService<S> {
    async fn record(&self, mut record: AuditRecord) -> Result<(), action_service::ActionServiceError> {
        record.payload = Self::redact_payload(record.payload);
        if record.current_hash.is_empty() {
            record.current_hash = AuditRecord::compute_hash(
                record.record_id,
                record.decision_id,
                record.event_type,
                &record.payload,
                record.created_at,
                record.previous_hash.as_deref(),
            );
        }
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

    #[tokio::test]
    async fn audit_chain_passes_verification_when_untampered() {
        let store = Arc::new(InMemoryAuditStore::new());
        let svc = AuditService::new(store.clone());
        let id = generate_correlation_id();
        svc.record(
            AuditEventType::ActionRequested,
            Some(id),
            serde_json::json!({"amt": 100}),
        )
        .await;
        svc.record(
            AuditEventType::PolicyEvaluated,
            Some(id),
            serde_json::json!({"verdict": "allow"}),
        )
        .await;
        svc.record(
            AuditEventType::DecisionMade,
            Some(id),
            serde_json::json!({"decision": "allow"}),
        )
        .await;

        let records = svc.all_records().await.unwrap();
        assert_eq!(records.len(), 3);
        assert!(AuditService::<InMemoryAuditStore>::verify_chain(&records).is_ok());
    }

    #[tokio::test]
    async fn audit_chain_detects_tampered_payload() {
        let store = Arc::new(InMemoryAuditStore::new());
        let svc = AuditService::new(store.clone());
        let id = generate_correlation_id();
        svc.record(
            AuditEventType::ActionRequested,
            Some(id),
            serde_json::json!({"amt": 100}),
        )
        .await;
        svc.record(
            AuditEventType::DecisionMade,
            Some(id),
            serde_json::json!({"decision": "allow"}),
        )
        .await;

        let mut records = svc.all_records().await.unwrap();
        // Tamper with record #0 payload
        records[0].payload = serde_json::json!({"amt": 999999});
        let result = AuditService::<InMemoryAuditStore>::verify_chain(&records);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("tamper detected"));
    }
}
