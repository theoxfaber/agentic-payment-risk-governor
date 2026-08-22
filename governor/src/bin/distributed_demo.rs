//! Distributed demo (Phase 2 step 4): policy AND evidence both run in
//! separate processes over NATS.
//!
//!   docker compose up -d          # nats + postgres + rg-policy + rg-evidence
//!   cargo run -p governor --bin distributed-demo

use action_service::ActionService;
use audit_service::{AuditService, InMemoryAuditStore};
use nats_link::{NatsEvidenceService, NatsPolicyEngine};
use razorpay_gateway::MockGateway;
use risk_governor_types::*;
use std::sync::Arc;

type DistributedService = ActionService<
    NatsPolicyEngine,
    risk_engine::RiskEngine,
    NatsEvidenceService,
    AuditService<InMemoryAuditStore>,
    MockGateway,
>;

fn wire(client: async_nats::Client) -> Arc<DistributedService> {
    Arc::new(ActionService::new(
        Arc::new(NatsPolicyEngine::new(client.clone())),
        Arc::new(risk_engine::RiskEngine::default()),
        Arc::new(NatsEvidenceService::new(client.clone())),
        Arc::new(AuditService::new(Arc::new(InMemoryAuditStore::new()))),
        Arc::new(MockGateway::default()),
    ))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    risk_governor_correlation::init_tracing("info");

    let url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
    let client = async_nats::ConnectOptions::new()
        .connection_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await?;
    let svc = wire(client);

    let cases: Vec<(&str, i64)> = vec![
        ("small legit", 50_000),
        ("approval threshold", 150_000),
    ];

    for (label, amount) in cases {
        match svc
            .process_action(refund("agent-trusted-01", amount))
            .await
        {
            Ok(d) => println!(
                "{label}: {:?} | cid={} | policy rules={:?}",
                d.decision, d.action.correlation_id, d.policy_result.matched_rules
            ),
            Err(e) => println!("{label}: ERROR {e}"),
        }
    }
    Ok(())
}

fn refund(agent: &str, amount: i64) -> AgentActionRequest {
    AgentActionRequest {
        agent_id: agent.into(),
        merchant_id: "merchant-001".into(),
        action_type: ActionType::Refund,
        amount,
        currency: "INR".into(),
        declared_intent: format!("refund for order #{}", amount),
        context: serde_json::json!({ "payment_id": "pay_X" }),
        timestamp: now_utc(),
        correlation_id: generate_correlation_id(),
    }
}