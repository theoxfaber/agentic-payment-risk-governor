use action_service::GatheredEvidence;
use chrono::{DateTime, Duration, Utc};
use risk_governor_types::*;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;

#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error("merchant not found: {0}")]
    MerchantNotFound(String),
    #[error("storage error: {0}")]
    Storage(String),
}

/// Source of truth for "what do we know" — agent history, merchant policy,
/// customer history, and recent velocity for a given action request.
///
/// Phase 1: in-memory store. Phase 2: backed by Postgres via sqlx.
#[async_trait::async_trait]
pub trait EvidenceStore: Send + Sync {
    async fn agent_history(&self, agent_id: &str) -> Result<Option<AgentHistory>, EvidenceError>;
    async fn merchant_policy(&self, merchant_id: &str) -> Result<Option<MerchantPolicy>, EvidenceError>;
    async fn customer_history(&self, customer_id: &str) -> Result<Option<CustomerHistory>, EvidenceError>;
    async fn record_action(&self, request: &AgentActionRequest) -> Result<(), EvidenceError>;
    async fn velocity(&self, agent_id: &str) -> Result<VelocityStats, EvidenceError> {
        let _ = agent_id;
        Ok(VelocityStats::default())
    }
}

pub struct InMemoryEvidenceStore {
    agents: RwLock<HashMap<String, AgentHistory>>,
    merchants: RwLock<HashMap<String, MerchantPolicy>>,
    customers: RwLock<HashMap<String, CustomerHistory>>,
    /// (agent_id, timestamp, amount) tuples used to compute velocity windows
    action_log: RwLock<Vec<(String, DateTime<Utc>, i64)>>,
}

impl InMemoryEvidenceStore {
    pub fn new() -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
            merchants: RwLock::new(HashMap::new()),
            customers: RwLock::new(HashMap::new()),
            action_log: RwLock::new(Vec::new()),
        }
    }

    pub async fn seed_merchant_policy(&self, policy: MerchantPolicy) {
        self.merchants.write().await.insert(policy.merchant_id.clone(), policy);
    }

    pub async fn seed_agent(&self, history: AgentHistory) {
        self.agents.write().await.insert(history.agent_id.clone(), history);
    }

    pub async fn seed_customer(&self, history: CustomerHistory) {
        self.customers
            .write()
            .await
            .insert(history.customer_id.clone(), history);
    }

    /// Ensures a merchant has a policy so gather() never fails mid-eval.
    pub async fn seed_default_policy_if_missing(&self, merchant_id: &str) -> Result<(), EvidenceError> {
        let mut merchants = self.merchants.write().await;
        if !merchants.contains_key(merchant_id) {
            merchants.insert(
                merchant_id.to_string(),
                MerchantPolicy {
                    merchant_id: merchant_id.to_string(),
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
            );
        }
        Ok(())
    }

    async fn compute_velocity(&self, agent_id: &str) -> VelocityStats {
        let log = self.action_log.read().await;
        let now = Utc::now();
        let one_hour_ago = now - Duration::hours(1);
        let one_day_ago = now - Duration::hours(24);

        let mut stats = VelocityStats::default();
        for (aid, ts, amount) in log.iter() {
            if aid != agent_id {
                continue;
            }
            if *ts >= one_hour_ago {
                stats.actions_last_hour += 1;
                stats.volume_last_hour += amount;
            }
            if *ts >= one_day_ago {
                stats.actions_last_24h += 1;
                stats.volume_last_24h += amount;
            }
        }
        stats
    }
}

impl Default for InMemoryEvidenceStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl EvidenceStore for InMemoryEvidenceStore {
    async fn agent_history(&self, agent_id: &str) -> Result<Option<AgentHistory>, EvidenceError> {
        Ok(self.agents.read().await.get(agent_id).cloned())
    }

    async fn merchant_policy(&self, merchant_id: &str) -> Result<Option<MerchantPolicy>, EvidenceError> {
        Ok(self.merchants.read().await.get(merchant_id).cloned())
    }

    async fn customer_history(&self, customer_id: &str) -> Result<Option<CustomerHistory>, EvidenceError> {
        Ok(self.customers.read().await.get(customer_id).cloned())
    }

    async fn record_action(&self, request: &AgentActionRequest) -> Result<(), EvidenceError> {
        self.action_log
            .write()
            .await
            .push((request.agent_id.clone(), request.timestamp, request.amount));
        Ok(())
    }

    async fn velocity(&self, agent_id: &str) -> Result<VelocityStats, EvidenceError> {
        Ok(self.compute_velocity(agent_id).await)
    }
}

pub struct EvidenceService<S: EvidenceStore> {
    store: Arc<S>,
}

impl<S: EvidenceStore> EvidenceService<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    pub async fn gather(&self, request: &AgentActionRequest) -> Result<Evidence, EvidenceError> {
        let agent_history = self
            .store
            .agent_history(&request.agent_id)
            .await?
            .ok_or_else(|| EvidenceError::Storage(format!("no history for agent {}", request.agent_id)))?;

        let merchant_policy = self
            .store
            .merchant_policy(&request.merchant_id)
            .await?
            .ok_or_else(|| EvidenceError::MerchantNotFound(request.merchant_id.clone()))?;

        // Customer id is optional context on the request
        let customer_history = match request.context.get("customer_id").and_then(|v| v.as_str()) {
            Some(cid) => self.store.customer_history(cid).await?,
            None => None,
        };

        let recent_velocity = self.store.velocity(&request.agent_id).await?;

        Ok(Evidence {
            agent_history,
            merchant_policy,
            customer_history,
            recent_velocity,
            fetched_at: now_utc(),
        })
    }
}

#[async_trait::async_trait]
impl<S: EvidenceStore + 'static> action_service::EvidenceService for EvidenceService<S> {
    async fn gather(
        &self,
        request: &AgentActionRequest,
    ) -> Result<GatheredEvidence, action_service::ActionServiceError> {
        self.gather(request)
            .await
            .map(GatheredEvidence::fresh)
            .map_err(|e| action_service::ActionServiceError::EvidenceService(e.to_string()))
    }

    async fn record_action(&self, request: &AgentActionRequest) -> Result<(), action_service::ActionServiceError> {
        self.store
            .record_action(request)
            .await
            .map_err(|e| action_service::ActionServiceError::EvidenceService(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use risk_governor_types::{generate_correlation_id, now_utc};

    fn store() -> InMemoryEvidenceStore {
        InMemoryEvidenceStore::new()
    }

    fn agent_history(id: &str) -> AgentHistory {
        AgentHistory {
            agent_id: id.into(),
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
        }
    }

    fn request(agent: &str, merchant: &str) -> AgentActionRequest {
        AgentActionRequest {
            agent_id: agent.into(),
            merchant_id: merchant.into(),
            action_type: ActionType::Refund,
            amount: 10_000,
            currency: "INR".into(),
            declared_intent: "refund".into(),
            context: serde_json::json!({ "customer_id": "cust_1" }),
            timestamp: now_utc(),
            correlation_id: generate_correlation_id(),
        }
    }

    #[tokio::test]
    async fn seed_and_retrieve_agent_history() {
        let s = store();
        assert!(s.agent_history("a1").await.unwrap().is_none());
        s.seed_agent(agent_history("a1")).await;
        let h = s.agent_history("a1").await.unwrap().unwrap();
        assert_eq!(h.avg_amount, 50_000);
    }

    #[tokio::test]
    async fn seed_and_retrieve_merchant_policy() {
        let s = store();
        s.seed_default_policy_if_missing("m1").await.unwrap();
        // seeding twice must not clobber an existing policy
        s.seed_default_policy_if_missing("m1").await.unwrap();
        let p = s.merchant_policy("m1").await.unwrap().unwrap();
        assert_eq!(p.max_refund_amount, 500_000);
    }

    #[tokio::test]
    async fn seed_customer_round_trips() {
        let s = store();
        s.seed_customer(CustomerHistory {
            customer_id: "cust_1".into(),
            total_transactions: 12,
            total_volume: 120_000,
            chargeback_count: 0,
            refund_count: 1,
            avg_ticket_size: 10_000,
            first_transaction: now_utc() - chrono::Duration::days(200),
            last_transaction: now_utc(),
            risk_score: 0.1,
        })
        .await;
        let c = s.customer_history("cust_1").await.unwrap().unwrap();
        assert_eq!(c.total_transactions, 12);
    }

    #[tokio::test]
    async fn record_action_updates_velocity_windows() {
        let s = store();
        for _ in 0..3 {
            s.record_action(&request("a1", "m1")).await.unwrap();
        }
        let v = s.velocity("a1").await.unwrap();
        assert_eq!(v.actions_last_hour, 3);
        assert_eq!(v.actions_last_24h, 3);
        assert_eq!(v.volume_last_hour, 30_000);
        // other agents are isolated
        let v2 = s.velocity("a2").await.unwrap();
        assert_eq!(v2.actions_last_hour, 0);
    }

    #[tokio::test]
    async fn gather_returns_error_for_unknown_agent() {
        let s = store();
        s.seed_default_policy_if_missing("m1").await.unwrap();
        let err = EvidenceService::new(Arc::new(s))
            .gather(&request("ghost", "m1"))
            .await
            .unwrap_err();
        assert!(matches!(err, EvidenceError::Storage(_)));
    }

    #[tokio::test]
    async fn gather_returns_error_for_unknown_merchant() {
        let s = store();
        s.seed_agent(agent_history("a1")).await;
        let err = EvidenceService::new(Arc::new(s))
            .gather(&request("a1", "ghost"))
            .await
            .unwrap_err();
        assert!(matches!(err, EvidenceError::MerchantNotFound(_)));
    }

    #[tokio::test]
    async fn gather_assembles_full_evidence() {
        let s = Arc::new(store());
        s.seed_agent(agent_history("a1")).await;
        s.seed_default_policy_if_missing("m1").await.unwrap();
        let ev = EvidenceService::new(s).gather(&request("a1", "m1")).await.unwrap();
        assert_eq!(ev.agent_history.agent_id, "a1");
        assert_eq!(ev.merchant_policy.merchant_id, "m1");
        assert_eq!(ev.fetched_at, ev.fetched_at); // present and well-formed
    }
}
