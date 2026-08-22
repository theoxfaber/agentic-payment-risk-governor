//! Phase 2 chaos + remote-evidence tests.
//!
//! `remote_evidence_allow` spawns workers as in-process tasks (fast, no docker).
//! `chaos_evidence_container_kill` requires the compose stack UP and does a
//! REAL `docker kill -9` of the evidence container mid-pipeline — the exact
//! failure mode a live demo produces. Run with:
//!
//!   docker compose up -d
//!   cargo test -p governor --test chaos -- --ignored --test-threads=1

use action_service::ActionService;
use audit_service::{AuditService, InMemoryAuditStore};
use evidence_service::InMemoryEvidenceStore;
use nats_link::{NatsEvidenceService, NatsPolicyEngine};
use razorpay_gateway::MockGateway;
use risk_governor_types::*;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

type Svc = ActionService<
    NatsPolicyEngine,
    risk_engine::RiskEngine,
    NatsEvidenceService,
    AuditService<InMemoryAuditStore>,
    MockGateway,
>;

async fn nats() -> async_nats::Client {
    let url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
    async_nats::ConnectOptions::new()
        .connection_timeout(Duration::from_secs(5))
        .connect(&url)
        .await
        .expect("nats must be reachable for these tests")
}

/// Full pipeline over BOTH hops (evidence + policy), workers in-process.
#[tokio::test]
#[ignore = "requires local NATS"]
async fn remote_evidence_allow() {
    let client = nats().await;
    let store = Arc::new(InMemoryEvidenceStore::new());
    seed(&store).await;

    let policy_w = nats_link::spawn_policy_worker(client.clone());
    let evidence_w = nats_link::spawn_evidence_worker(client.clone(), store.clone());
    tokio::time::sleep(Duration::from_millis(200)).await; // subscription registration

    let svc: Arc<Svc> = Arc::new(ActionService::new(
        Arc::new(NatsPolicyEngine::new(client.clone())),
        Arc::new(risk_engine::RiskEngine::default()),
        Arc::new(NatsEvidenceService::new(client.clone())),
        Arc::new(AuditService::new(Arc::new(InMemoryAuditStore::new()))),
        Arc::new(MockGateway::default()),
    ));

    let d = svc.process_action(refund("agent-1", 50_000)).await.unwrap();

    assert_eq!(d.decision, DecisionOutcome::Allow);
    assert!(d.policy_result.matched_rules.iter().all(|r| {
        !r.starts_with("policy_engine_unavailable")
            && !r.starts_with("evidence_service_unavailable")
    }));

    policy_w.abort();
    evidence_w.abort();
}

/// THE Phase 2 checkpoint, executed for real:
/// hard-kill the evidence container mid-pipeline → Review, never silent allow,
/// never a hard error. Exercises the no_responders/timeout path under SIGKILL.
#[tokio::test]
#[ignore = "requires compose stack up (rg-evidence running)"]
async fn chaos_evidence_container_kill_fails_safe() {
    const CONTAINER: &str = "rg-evidence";

    // Precondition: stack actually up, else this test lies.
    let state = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", CONTAINER])
        .output()
        .expect("docker CLI must be available");
    assert!(
        String::from_utf8_lossy(&state.stdout).trim() == "true",
        "{CONTAINER} must be running — docker compose up -d first"
    );

    let client = nats().await;
    let svc: Arc<Svc> = Arc::new(ActionService::new(
        Arc::new(NatsPolicyEngine::new(client.clone())),
        Arc::new(risk_engine::RiskEngine::default()),
        Arc::new(NatsEvidenceService::new(client.clone()).with_timeout(Duration::from_secs(2))),
        Arc::new(AuditService::new(Arc::new(InMemoryAuditStore::new()))),
        Arc::new(MockGateway::default()),
    ));

    // 1) Sanity: pipeline works while evidence-service is alive.
    let before = svc.process_action(refund("agent-trusted-01", 50_000)).await;
    match &before {
        Ok(d) => println!("pre-kill decision: {:?}", d.decision),
        Err(e) => panic!("pipeline broken BEFORE kill — test setup wrong: {e}"),
    }

    // 2) Make the kill stick: restart policy would otherwise resurrect it.
    let _ = Command::new("docker").args(["update", "--restart=no", CONTAINER]).output();
    let killed = Command::new("docker")
        .args(["kill", "-s", "KILL", CONTAINER]) // SIGKILL: no graceful shutdown
        .output()
        .expect("docker kill failed");
    assert!(killed.status.success(), "docker kill failed: {}", String::from_utf8_lossy(&killed.stderr));
    println!("evidence container SIGKILLed mid-pipeline");

    // 3) Immediately fire a request. Expect fail-safe Review.
    let after = svc.process_action(refund("agent-trusted-01", 50_000)).await;
    let d = after.expect("downed evidence MUST NOT hard-fail the caller");
    assert_eq!(
        d.decision,
        DecisionOutcome::Review,
        "downed evidence service must route to human review, got {:?}",
        d.decision
    );
    let marker_present = d
        .policy_result
        .matched_rules
        .iter()
        .any(|r| r.starts_with("evidence_service_unavailable"));
    assert!(marker_present, "degradation must be visible in the audit trail");

    // 4) Restore for subsequent runs.
    let _ = Command::new("docker-compose").args(["up", "-d", "evidence-service"]).output();
    let _ = Command::new("docker").args(["update", "--restart=unless-stopped", CONTAINER]).output();
}

async fn seed(store: &InMemoryEvidenceStore) {
    for agent_id in ["agent-1", "agent-trusted-01"] {
        store
            .seed_agent(AgentHistory {
                agent_id: agent_id.into(),
                total_actions_30d: 30,
                total_volume_30d: 1_500_000,
                avg_amount: 50_000,
                max_amount: 100_000,
                refund_rate: 0.05,
                block_rate: 0.02,
                review_rate: 0.03,
                first_seen: now_utc() - chrono::Duration::days(90),
                last_action: now_utc() - chrono::Duration::hours(2),
                action_type_distribution: Default::default(),
                anomaly_flags: vec![],
            })
            .await;
    }
    store.seed_default_policy_if_missing("m-001").await.unwrap();
    store.seed_default_policy_if_missing("merchant-001").await.unwrap();
}

fn refund(agent: &str, amount: i64) -> AgentActionRequest {
    AgentActionRequest {
        agent_id: agent.into(),
        merchant_id: "merchant-001".into(),
        action_type: ActionType::Refund,
        amount,
        currency: "INR".into(),
        declared_intent: format!("refund order {amount}"),
        context: serde_json::json!({ "payment_id": "pay_X" }),
        timestamp: now_utc(),
        correlation_id: generate_correlation_id(),
    }
}
