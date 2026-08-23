use std::collections::HashMap;

use investigation_engine::*;
use risk_graph::*;

/// Ring: shared device + address + instrument, 40% return rate,
/// everyone returns within ~24-36h of purchase.
fn abuse_ring() -> (
    PropertyGraph,
    Cluster,
    HashMap<String, CustomerBehavior>,
    HashMap<String, i64>,
) {
    let graph = GraphBuilder::new()
        .entity(EntityKind::Device, "DEV_1")
        .entity(EntityKind::Address, "ADR_1")
        .entity(EntityKind::PaymentInstrument, "PIN_9")
        .entity(EntityKind::Customer, "R1")
        .entity(EntityKind::Customer, "R2")
        .entity(EntityKind::Customer, "R3")
        .relate(
            EntityKind::Customer,
            "R1",
            RelationKind::UsesDevice,
            EntityKind::Device,
            "DEV_1",
        )
        .relate(
            EntityKind::Customer,
            "R2",
            RelationKind::UsesDevice,
            EntityKind::Device,
            "DEV_1",
        )
        .relate(
            EntityKind::Customer,
            "R3",
            RelationKind::UsesDevice,
            EntityKind::Device,
            "DEV_1",
        )
        .relate(
            EntityKind::Customer,
            "R1",
            RelationKind::ShipsTo,
            EntityKind::Address,
            "ADR_1",
        )
        .relate(
            EntityKind::Customer,
            "R2",
            RelationKind::ShipsTo,
            EntityKind::Address,
            "ADR_1",
        )
        .relate(
            EntityKind::Customer,
            "R3",
            RelationKind::ShipsTo,
            EntityKind::Address,
            "ADR_1",
        )
        .relate(
            EntityKind::Customer,
            "R1",
            RelationKind::UsesInstrument,
            EntityKind::PaymentInstrument,
            "PIN_9",
        )
        .relate(
            EntityKind::Customer,
            "R2",
            RelationKind::UsesInstrument,
            EntityKind::PaymentInstrument,
            "PIN_9",
        )
        .relate(
            EntityKind::Customer,
            "R3",
            RelationKind::UsesInstrument,
            EntityKind::PaymentInstrument,
            "PIN_9",
        )
        .build();

    let cluster = &graph.abuse_ring_clusters(2)[0];

    let behavior = |id: &str| CustomerBehavior {
        customer_id: id.into(),
        order_count: 10,
        return_count: 4,
        refund_count: 4,
        dispute_count: 0,
        distinct_merchants: 1,
        distinct_products: 1,
        account_age_days: 20,
        purchase_to_return_hours: vec![26.0, 30.0, 28.0],
    };
    let mut behaviors = HashMap::new();
    for id in ["R1", "R2", "R3"] {
        behaviors.insert(id.into(), behavior(id));
    }

    let exposure: HashMap<String, i64> = [("R1", 500_000), ("R2", 400_000), ("R3", 600_000)]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();

    (graph, cluster.clone(), behaviors, exposure)
}

/// Household FP trap: structurally identical clustering (shared device +
/// address), but diverse purchases, old accounts, normal return rates.
fn household() -> (
    PropertyGraph,
    Cluster,
    HashMap<String, CustomerBehavior>,
    HashMap<String, i64>,
) {
    let graph = GraphBuilder::new()
        .entity(EntityKind::Device, "FAM_LAPTOP")
        .entity(EntityKind::Address, "HOME")
        .entity(EntityKind::Customer, "DAD")
        .entity(EntityKind::Customer, "MOM")
        .entity(EntityKind::Customer, "KID")
        .relate(
            EntityKind::Customer,
            "DAD",
            RelationKind::UsesDevice,
            EntityKind::Device,
            "FAM_LAPTOP",
        )
        .relate(
            EntityKind::Customer,
            "MOM",
            RelationKind::UsesDevice,
            EntityKind::Device,
            "FAM_LAPTOP",
        )
        .relate(
            EntityKind::Customer,
            "KID",
            RelationKind::UsesDevice,
            EntityKind::Device,
            "FAM_LAPTOP",
        )
        .relate(
            EntityKind::Customer,
            "DAD",
            RelationKind::ShipsTo,
            EntityKind::Address,
            "HOME",
        )
        .relate(
            EntityKind::Customer,
            "MOM",
            RelationKind::ShipsTo,
            EntityKind::Address,
            "HOME",
        )
        .relate(
            EntityKind::Customer,
            "KID",
            RelationKind::ShipsTo,
            EntityKind::Address,
            "HOME",
        )
        .build();

    let cluster = &graph.abuse_ring_clusters(2)[0];

    let behavior = |id: &str, rets: u32, merch: u32, prod: u32, age: u64| CustomerBehavior {
        customer_id: id.into(),
        order_count: 40,
        return_count: rets,
        refund_count: rets,
        dispute_count: 0,
        distinct_merchants: merch,
        distinct_products: prod,
        account_age_days: age,
        purchase_to_return_hours: vec![400.0], // leisurely, unsynchronized
    };
    let mut behaviors = HashMap::new();
    behaviors.insert("DAD".into(), behavior("DAD", 2, 8, 15, 900));
    behaviors.insert("MOM".into(), behavior("MOM", 1, 6, 12, 850));
    behaviors.insert("KID".into(), behavior("KID", 3, 5, 9, 300));

    let exposure = HashMap::new();
    (graph, cluster.clone(), behaviors, exposure)
}

#[test]
fn coordinated_abuse_ring_is_supported() {
    let inv = Investigator::new(Baseline::default());
    let (g, c, b, e) = abuse_ring();
    let r = inv.investigate_return_abuse(&g, &c, &b, &e);

    assert_eq!(r.verdict, Verdict::Supported);
    assert!(r.supporting.iter().any(|x| x.signal == "cluster_size"));
    assert!(r
        .supporting
        .iter()
        .any(|x| x.signal == "multiple_shared_resource_types"));
    assert!(r.supporting.iter().any(|x| x.signal == "return_rate_anomaly"));
    assert!(r.supporting.iter().any(|x| x.signal == "synchronized_returns"));
    assert!(r.counter.is_empty(), "ring has no counter-evidence");
    // Exposure aggregated across the whole ring
    assert_eq!(r.estimated_exposure_paise, 1_500_000);
}

#[test]
fn household_cluster_is_not_auto_escalated() {
    let inv = Investigator::new(Baseline::default());
    let (g, c, b, e) = household();
    let r = inv.investigate_return_abuse(&g, &c, &b, &e);

    // The structural cluster exists, but the hypothesis must NOT be supported:
    assert_ne!(r.verdict, Verdict::Supported);
    assert!(r.counter.iter().any(|x| x.signal == "normal_return_rates"));
    assert!(
        r.counter.iter().any(|x| x.signal == "household_plausible"),
        "the household defense must be explicitly surfaced, not silently dropped"
    );
}

#[test]
fn partial_behavior_data_penalizes_confidence_and_is_recorded() {
    let inv = Investigator::new(Baseline::default());
    let (g, c, mut b, e) = abuse_ring();
    b.remove("R3"); // investigator is blind to one member

    let r = inv.investigate_return_abuse(&g, &c, &b, &e);
    assert!(r.missing.iter().any(|m| m.signal == "partial_behavior_data"));

    let full = Investigator::new(Baseline::default());
    let (g2, c2, b2, e2) = abuse_ring();
    let r_full = full.investigate_return_abuse(&g2, &c2, &b2, &e2);
    assert!(
        r.evidence_confidence < r_full.evidence_confidence,
        "missing data must lower confidence ({} vs {})",
        r.evidence_confidence,
        r_full.evidence_confidence
    );
}

#[test]
fn confidence_stays_bounded_and_explainable() {
    let inv = Investigator::new(Baseline::default());
    let (g, c, b, e) = abuse_ring();
    let r = inv.investigate_return_abuse(&g, &c, &b, &e);
    assert!((0.0..=1.0).contains(&r.evidence_confidence));

    // Every evidence item carries a human-readable description — replay/audit
    // and the pitch video both depend on this.
    for item in r.supporting.iter().chain(r.counter.iter()).chain(r.missing.iter()) {
        assert!(!item.description.is_empty(), "{} has no explanation", item.signal);
        assert!(item.weight > 0.0 && item.weight <= 1.0);
    }
}
