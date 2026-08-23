//! The safety property, executable:
//!   a high risk score can NEVER force an automatic action when the
//!   investigation plane is conflicted or low-confidence.

use action_service::{
    ActionService, GatheredEvidence, Investigator,
};
use audit_service::{AuditService, InMemoryAuditStore};
use investigation_engine::*;
use razorpay_gateway::MockGateway;
use risk_governor_types::*;
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test stubs
// ---------------------------------------------------------------------------

struct FixedRisk(f64);

#[async_trait::async_trait]
impl action_service::RiskEngine for FixedRisk {
    async fn score(&self, _r: &AgentActionRequest, _e: &Evidence) -> Result<RiskResult, action_service::ActionServiceError> {
        Ok(RiskResult {
            risk_score: self.0,
            intent_mismatch_score: 0.0,
            features: RiskFeatures {
                amount_zscore: 0.0, velocity_zscore: 0.0, intent_mismatch_score: 0.0,
                behavioral_drift_score: 0.0, merchant_risk_score: 0.0, agent_risk_score: 0.0,
                customer_risk_score: 0.0, time_since_last_action_hours: 0.0, amount_vs_avg_ratio: 1.0,
            },
            model_version: "fixed-test".into(),
            evaluated_at: now_utc(),
        })
    }
}

struct PolicyAllowAll;

#[async_trait::async_trait]
impl action_service::PolicyEngine for PolicyAllowAll {
    async fn evaluate(&self, _: &AgentActionRequest, _: &Evidence) -> Result<PolicyResult, action_service::ActionServiceError> {
        Ok(PolicyResult {
            verdict: PolicyVerdict::Allow,
            matched_rules: vec![],
            violated_thresholds: vec![],
            evaluated_at: now_utc(),
        })
    }
}

struct EvidenceOk;

#[async_trait::async_trait]
impl action_service::EvidenceService for EvidenceOk {
    async fn gather(&self, req: &AgentActionRequest) -> Result<GatheredEvidence, action_service::ActionServiceError> {
        Ok(GatheredEvidence::fresh(Evidence {
            agent_history: AgentHistory {
                agent_id: req.agent_id.clone(),
                total_actions_30d: 10, total_volume_30d: 500_000, avg_amount: 50_000,
                max_amount: 100_000, refund_rate: 0.05, block_rate: 0.02, review_rate: 0.03,
                first_seen: now_utc() - chrono::Duration::days(90),
                last_action: now_utc() - chrono::Duration::hours(2),
                action_type_distribution: Default::default(), anomaly_flags: vec![],
            },
            merchant_policy: MerchantPolicy {
                merchant_id: req.merchant_id.clone(),
                max_refund_amount: i64::MAX / 2, max_payout_amount: i64::MAX / 2,
                max_payment_link_amount: i64::MAX / 2, daily_refund_limit: i64::MAX / 2,
                daily_payout_limit: i64::MAX / 2, velocity_threshold_per_hour: u32::MAX,
                allowed_countries: vec![], blocked_countries: vec![],
                require_approval_above: i64::MAX / 2, custom_rules: vec![],
            },
            customer_history: None,
            recent_velocity: VelocityStats::default(),
            fetched_at: now_utc(),
        }))
    }

    async fn record_action(&self, _: &AgentActionRequest) -> Result<(), action_service::ActionServiceError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn ring_graph() -> Arc<risk_graph::PropertyGraph> {
    let mut b = risk_graph::GraphBuilder::new()
        .entity(risk_graph::EntityKind::Device, "D")
        .entity(risk_graph::EntityKind::Address, "A")
        .entity(risk_graph::EntityKind::Customer, "R1")
        .entity(risk_graph::EntityKind::Customer, "R2")
        .entity(risk_graph::EntityKind::Customer, "R3");
    for c in ["R1", "R2", "R3"] {
        b = b
            .relate(risk_graph::EntityKind::Customer, c, risk_graph::RelationKind::UsesDevice, risk_graph::EntityKind::Device, "D")
            .relate(risk_graph::EntityKind::Customer, c, risk_graph::RelationKind::ShipsTo, risk_graph::EntityKind::Address, "A");
    }
    // third link kind (instrument) only between R1/R2 → link_kinds = 3
    let mut b = b.relate(
        risk_graph::EntityKind::Customer, "R1", risk_graph::RelationKind::UsesInstrument,
        risk_graph::EntityKind::PaymentInstrument, "PIN",
    );
    b = b.relate(
        risk_graph::EntityKind::Customer, "R2", risk_graph::RelationKind::UsesInstrument,
        risk_graph::EntityKind::PaymentInstrument, "PIN",
    );
    Arc::new(b.build())
}

/// High returns AND household-like diversity — sophisticated/ambiguous.
/// Structural support + behavioral contradiction → Conflicted.
fn conflicted_behaviors() -> HashMap<String, CustomerBehavior> {
    ["R1", "R2", "R3"]
        .iter()
        .map(|id| {
            (
                id.to_string(),
                CustomerBehavior {
                    customer_id: id.to_string(),
                    order_count: 30,
                    return_count: 9, // 30% ≫ baseline
                    refund_count: 9,
                    dispute_count: 0,
                    distinct_merchants: 6, // diverse
                    distinct_products: 11, // diverse
                    account_age_days: 800, // established
                    purchase_to_return_hours: vec![500.0], // unsynchronized
                },
            )
        })
        .collect()
}

/// Pure ring behavior: concentrated, new accounts, fast synchronized returns.
fn supported_behaviors() -> HashMap<String, CustomerBehavior> {
    ["R1", "R2", "R3"]
        .iter()
        .map(|id| {
            (
                id.to_string(),
                CustomerBehavior {
                    customer_id: id.to_string(),
                    order_count: 12,
                    return_count: 5,
                    refund_count: 5,
                    dispute_count: 0,
                    distinct_merchants: 1,
                    distinct_products: 1,
                    account_age_days: 15,
                    purchase_to_return_hours: vec![24.0, 28.0],
                },
            )
        })
        .collect()
}

#[allow(clippy::type_complexity)] // test fixture wiring; the generics ARE the pipeline
fn wire(
    risk_score: f64,
    investigator: Option<Arc<dyn Investigator>>,
) -> (
    Arc<ActionService<PolicyAllowAll, FixedRisk, EvidenceOk, AuditService<InMemoryAuditStore>, MockGateway>>,
    Arc<AuditService<InMemoryAuditStore>>,
    Arc<MockGateway>,
) {
    let gateway = Arc::new(MockGateway::default());
    let audit = Arc::new(AuditService::new(Arc::new(InMemoryAuditStore::new())));
    let mut svc = ActionService::new(
        Arc::new(PolicyAllowAll),
        Arc::new(FixedRisk(risk_score)),
        Arc::new(EvidenceOk),
        audit.clone(),
        gateway.clone(),
    );
    if let Some(inv) = investigator {
        svc = svc.with_investigator(inv);
    }
    (Arc::new(svc), audit, gateway)
}

fn request_for(customer: &str) -> AgentActionRequest {
    AgentActionRequest {
        agent_id: "agt-1".into(),
        merchant_id: "m-1".into(),
        action_type: ActionType::Refund,
        amount: 50_000,
        currency: "INR".into(),
        declared_intent: "refund order".into(),
        context: serde_json::json!({ "customer_id": customer }),
        timestamp: now_utc(),
        correlation_id: generate_correlation_id(),
    }
}

// ---------------------------------------------------------------------------
// The safety property
// ---------------------------------------------------------------------------

/// HIGH risk + CONFLICTED evidence → Review, never Block.
#[tokio::test]
async fn high_risk_conflicted_evidence_cannot_auto_block() {
    let inv = GraphInvestigator::new(ring_graph(), conflicted_behaviors(), HashMap::new(), Baseline::default());
    let (svc, _audit, gw) = wire(0.95, Some(inv.into_trait()));

    let d = svc.process_action(request_for("R1")).await.unwrap();
    assert_eq!(d.decision, DecisionOutcome::Review,
        "0.95 risk with conflicted evidence must go to a human");
    assert!(gw.calls.lock().unwrap().is_empty(), "no money moved on conflicted evidence");
    assert!(d.policy_result.matched_rules.iter().any(|r| r == "evidence_contradiction"),
        "the reason must be visible in the decision record");
}

/// HIGH risk + SUPPORTED evidence → Block is justified; graph analysis audited.
#[tokio::test]
async fn high_risk_supported_evidence_blocks_with_full_trail() {
    let inv = GraphInvestigator::new(ring_graph(), supported_behaviors(), HashMap::new(), Baseline::default());
    let (svc, audit, gw) = wire(0.9, Some(inv.into_trait()));

    let d = svc.process_action(request_for("R1")).await.unwrap();
    assert_eq!(d.decision, DecisionOutcome::Block);
    assert!(gw.calls.lock().unwrap().is_empty());

    let trail = audit.trail_for(d.decision_id).await.unwrap();
    let analyzed = trail.iter().find(|r| r.event_type == AuditEventType::GraphAnalyzed)
        .expect("investigation must appear in the audit trail");
    assert_eq!(analyzed.payload["verdict"], "supported");
    assert!(analyzed.payload["supporting"].as_array().unwrap().len() >= 4);
}

/// LOW confidence on a SUPPORTED hypothesis + HIGH risk → Review.
/// (Supported-but-incomplete is the dangerous combination: enough signal to
/// be suspicious, not enough data to act autonomously.)
#[tokio::test]
async fn low_confidence_downgrades_block_to_review() {
    let mut b = risk_graph::GraphBuilder::new()
        .entity(risk_graph::EntityKind::Device, "D2")
        .entity(risk_graph::EntityKind::Address, "A2")
        .entity(risk_graph::EntityKind::Customer, "L1")
        .entity(risk_graph::EntityKind::Customer, "L2")
        .entity(risk_graph::EntityKind::Customer, "L3");
    for c in ["L1", "L2", "L3"] {
        b = b
            .relate(risk_graph::EntityKind::Customer, c, risk_graph::RelationKind::UsesDevice, risk_graph::EntityKind::Device, "D2")
            .relate(risk_graph::EntityKind::Customer, c, risk_graph::RelationKind::ShipsTo, risk_graph::EntityKind::Address, "A2");
    }
    let graph = Arc::new(b.build());

    let mut behaviors: HashMap<String, CustomerBehavior> = HashMap::new();
    behaviors.insert("L1".into(), CustomerBehavior {
        customer_id: "L1".into(),
        order_count: 12, return_count: 6, refund_count: 6, dispute_count: 0,
        distinct_merchants: 1, distinct_products: 1, account_age_days: 5,
        purchase_to_return_hours: vec![20.0],
    });
    // L2/L3 intentionally absent → partial_behavior_data

    let inv = GraphInvestigator::new(graph, behaviors, HashMap::new(), Baseline::default());
    let (svc, _, gw) = wire(0.95, Some(inv.into_trait()));

    let d = svc.process_action(request_for("L1")).await.unwrap();
    assert_eq!(d.decision, DecisionOutcome::Review);
    assert!(d.policy_result.matched_rules.iter().any(|r| r == "low_evidence_confidence"));
    assert!(gw.calls.lock().unwrap().is_empty());
}

/// No investigator attached → legacy semantics unchanged (high risk blocks).
#[tokio::test]
async fn without_investigator_high_risk_still_blocks() {
    let (svc, _, _) = wire(0.95, None);
    let d = svc.process_action(request_for("NOBODY")).await.unwrap();
    assert_eq!(d.decision, DecisionOutcome::Block);
}

/// Solo customer (no cluster) → hypothesis unsupported, no added friction:
/// mid-range risk still routes by score alone.
#[tokio::test]
async fn unsupported_hypothesis_adds_no_friction() {
    let inv = GraphInvestigator::new(ring_graph(), supported_behaviors(), HashMap::new(), Baseline::default());
    let (svc, _, gw) = wire(0.4, Some(inv.into_trait()));

    let d = svc.process_action(request_for("SOLO")).await.unwrap();
    assert_eq!(d.decision, DecisionOutcome::Allow);
    assert!(gw.calls.lock().unwrap().len() == 1, "clean allow executes the refund");
}
