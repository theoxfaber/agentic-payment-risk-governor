//! Store and gateway backend selection. In-memory (dev/tests) or Postgres /
//! live Razorpay (production), chosen at boot by env vars — identical trait
//! surface and wire format either way.

use action_service::{ActionServiceError, RazorpayGateway};
use audit_service::{AuditError, AuditStore, InMemoryAuditStore};
use evidence_service::{EvidenceError, EvidenceStore, InMemoryEvidenceStore};
use pg_store::PgStore;
use razorpay_gateway::{HttpGateway, MockGateway};
use risk_governor_types::*;
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Evidence store
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) enum EvidenceBackend {
    Mem(Arc<InMemoryEvidenceStore>),
    Pg(Arc<PgStore>),
}

#[async_trait::async_trait]
impl EvidenceStore for EvidenceBackend {
    async fn agent_history(&self, agent_id: &str) -> Result<Option<AgentHistory>, EvidenceError> {
        match self {
            Self::Mem(s) => s.agent_history(agent_id).await,
            Self::Pg(s) => s.agent_history(agent_id).await,
        }
    }
    async fn merchant_policy(&self, merchant_id: &str) -> Result<Option<MerchantPolicy>, EvidenceError> {
        match self {
            Self::Mem(s) => s.merchant_policy(merchant_id).await,
            Self::Pg(s) => s.merchant_policy(merchant_id).await,
        }
    }
    async fn customer_history(&self, customer_id: &str) -> Result<Option<CustomerHistory>, EvidenceError> {
        match self {
            Self::Mem(s) => s.customer_history(customer_id).await,
            Self::Pg(s) => s.customer_history(customer_id).await,
        }
    }
    async fn record_action(&self, request: &AgentActionRequest) -> Result<(), EvidenceError> {
        match self {
            Self::Mem(s) => s.record_action(request).await,
            Self::Pg(s) => s.record_action(request).await,
        }
    }
    async fn velocity(&self, agent_id: &str) -> Result<VelocityStats, EvidenceError> {
        match self {
            Self::Mem(s) => s.velocity(agent_id).await,
            Self::Pg(s) => s.velocity(agent_id).await,
        }
    }
}

// ---------------------------------------------------------------------------
// Audit store
// ---------------------------------------------------------------------------

pub(crate) enum AuditBackend {
    Mem(Arc<InMemoryAuditStore>),
    Pg(Arc<PgStore>),
}

impl Clone for AuditBackend {
    fn clone(&self) -> Self {
        match self {
            Self::Mem(s) => Self::Mem(s.clone()),
            Self::Pg(s) => Self::Pg(s.clone()),
        }
    }
}

#[async_trait::async_trait]
impl AuditStore for AuditBackend {
    async fn append(&self, record: AuditRecord) -> Result<(), AuditError> {
        match self {
            Self::Mem(s) => s.append(record).await,
            Self::Pg(s) => s.append(record).await,
        }
    }
    async fn by_decision(&self, decision_id: Uuid) -> Result<Vec<AuditRecord>, AuditError> {
        match self {
            Self::Mem(s) => s.by_decision(decision_id).await,
            Self::Pg(s) => s.by_decision(decision_id).await,
        }
    }
    async fn all(&self) -> Result<Vec<AuditRecord>, AuditError> {
        match self {
            Self::Mem(s) => s.all().await,
            Self::Pg(s) => s.all().await,
        }
    }
}

// ---------------------------------------------------------------------------
// Gateway selection
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) enum Gateway {
    Mock(Arc<MockGateway>),
    Http(Arc<HttpGateway>),
}

#[async_trait::async_trait]
impl RazorpayGateway for Gateway {
    async fn execute(
        &self,
        request: &AgentActionRequest,
        decision_id: Uuid,
    ) -> Result<serde_json::Value, ActionServiceError> {
        match self {
            Gateway::Mock(g) => g.execute(request, decision_id).await,
            Gateway::Http(h) => h.execute(request, decision_id).await,
        }
    }
    async fn verify_payment(
        &self,
        payment_id: &str,
    ) -> Result<Option<action_service::VerifiedPayment>, ActionServiceError> {
        match self {
            Gateway::Mock(g) => g.verify_payment(payment_id).await,
            Gateway::Http(h) => h.verify_payment(payment_id).await,
        }
    }
}

impl Gateway {
    pub(crate) async fn fetch_real_payments(&self, count: usize) -> Result<serde_json::Value, ActionServiceError> {
        match self {
            Gateway::Http(h) => h
                .get_json(&format!("/payments?count={count}"))
                .await
                .map_err(|e| ActionServiceError::RazorpayGateway(e.to_string())),
            Gateway::Mock(_) => Err(ActionServiceError::RazorpayGateway(
                "real data requires RAZORPAY_KEY_ID/SECRET (MockGateway has no live data)".into(),
            )),
        }
    }
    pub(crate) async fn fetch_real_orders(&self, count: usize) -> Result<serde_json::Value, ActionServiceError> {
        match self {
            Gateway::Http(h) => h
                .get_json(&format!("/orders?count={count}"))
                .await
                .map_err(|e| ActionServiceError::RazorpayGateway(e.to_string())),
            Gateway::Mock(_) => Err(ActionServiceError::RazorpayGateway(
                "real data requires RAZORPAY_KEY_ID/SECRET (MockGateway has no live data)".into(),
            )),
        }
    }
}

pub(crate) fn pick_gateway() -> Arc<Gateway> {
    match (std::env::var("RAZORPAY_KEY_ID"), std::env::var("RAZORPAY_KEY_SECRET")) {
        (Ok(id), Ok(secret)) if !id.is_empty() && !secret.is_empty() => {
            tracing::info!("live Razorpay TEST-MODE gateway enabled");
            Arc::new(Gateway::Http(Arc::new(HttpGateway::new(id, secret))))
        }
        _ => {
            tracing::info!("no Razorpay keys — using MockGateway (moves no money)");
            Arc::new(Gateway::Mock(Arc::new(MockGateway::default())))
        }
    }
}
