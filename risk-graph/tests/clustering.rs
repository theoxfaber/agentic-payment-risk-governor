use risk_graph::*;

/// The canonical abuse-ring fixture: 3 customers sharing device+address,
/// plus 2 unrelated customers. Ring must be exactly the 3.
fn ring_fixture() -> PropertyGraph {
    GraphBuilder::new()
        // shared resources
        .entity(EntityKind::Device, "DEV_X")
        .entity(EntityKind::Address, "ADR_Y")
        .entity(EntityKind::PaymentInstrument, "PIN_1")
        // ring members
        .entity_with(EntityKind::Customer, "CUST_A", serde_json::json!({"created":"2026-01-01"}))
        .entity_with(EntityKind::Customer, "CUST_B", serde_json::json!({"created":"2026-02-01"}))
        .entity_with(EntityKind::Customer, "CUST_C", serde_json::json!({"created":"2026-02-15"}))
        // unrelated customers
        .entity(EntityKind::Customer, "CUST_LONE")
        // merchants + payments for flavor
        .entity(EntityKind::Merchant, "MER_1")
        // links: A,B,C share device and address
        .relate(EntityKind::Customer, "CUST_A", RelationKind::UsesDevice, EntityKind::Device, "DEV_X")
        .relate(EntityKind::Customer, "CUST_B", RelationKind::UsesDevice, EntityKind::Device, "DEV_X")
        .relate(EntityKind::Customer, "CUST_C", RelationKind::UsesDevice, EntityKind::Device, "DEV_X")
        .relate(EntityKind::Customer, "CUST_A", RelationKind::ShipsTo, EntityKind::Address, "ADR_Y")
        .relate(EntityKind::Customer, "CUST_B", RelationKind::ShipsTo, EntityKind::Address, "ADR_Y")
        .relate(EntityKind::Customer, "CUST_C", RelationKind::ShipsTo, EntityKind::Address, "ADR_Y")
        // A also shares an instrument with B
        .relate(EntityKind::Customer, "CUST_A", RelationKind::UsesInstrument, EntityKind::PaymentInstrument, "PIN_1")
        .relate(EntityKind::Customer, "CUST_B", RelationKind::UsesInstrument, EntityKind::PaymentInstrument, "PIN_1")
        // lone customer touches nothing shared
        .build()
}

#[test]
fn detects_three_member_ring_and_leaves_lone_customer_out() {
    let g = ring_fixture();
    let clusters = g.abuse_ring_clusters(2);

    assert_eq!(clusters.len(), 1, "expected exactly one multi-member cluster");
    let c = &clusters[0];
    let members: Vec<&str> = c.members.iter().map(|m| m.0.as_str()).collect();
    assert_eq!(members.len(), 3);
    assert!(members.iter().any(|m| m.contains("CUST_A")));
    assert!(members.iter().any(|m| m.contains("CUST_B")));
    assert!(members.iter().any(|m| m.contains("CUST_C")));
    assert!(!members.iter().any(|m| m.contains("LONE")));

    // Shared resources recorded for investigator consumption
    assert!(c.shared_resources.iter().any(|r| r.0.contains("DEV_X")));
    assert!(c.shared_resources.iter().any(|r| r.0.contains("ADR_Y")));
    assert_eq!(c.link_kinds.len(), 3); // device, instrument, address
}

#[test]
fn transitive_join_two_hops() {
    // A—dev1—B, B—addr—C : A,B,C one cluster even though A,C share nothing directly
    let g = GraphBuilder::new()
        .entity(EntityKind::Device, "D1")
        .entity(EntityKind::Address, "AD1")
        .entity(EntityKind::Customer, "CA")
        .entity(EntityKind::Customer, "CB")
        .entity(EntityKind::Customer, "CC")
        .relate(EntityKind::Customer, "CA", RelationKind::UsesDevice, EntityKind::Device, "D1")
        .relate(EntityKind::Customer, "CB", RelationKind::UsesDevice, EntityKind::Device, "D1")
        .relate(EntityKind::Customer, "CB", RelationKind::ShipsTo, EntityKind::Address, "AD1")
        .relate(EntityKind::Customer, "CC", RelationKind::ShipsTo, EntityKind::Address, "AD1")
        .build();

    let clusters = g.abuse_ring_clusters(2);
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].members.len(), 3, "transitive resource sharing joins all three");
}

#[test]
fn min_size_filters_singletons_and_pairs_when_requested() {
    let g = ring_fixture();
    let clusters = g.abuse_ring_clusters(4);
    assert!(clusters.is_empty(), "no cluster reaches size 4");
}

#[test]
fn deterministic_output_across_runs() {
    let a = ring_fixture().abuse_ring_clusters(2);
    let b = ring_fixture().abuse_ring_clusters(2);
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap(),
        "clustering must be deterministic (replay/eval depend on it)"
    );
}

#[test]
fn co_customers_query_matches_cluster_membership() {
    let g = ring_fixture();
    let aid = EntityId::new(EntityKind::Customer, "CUST_A");
    let co = g.co_customers_via_resources(&aid);
    assert_eq!(co.len(), 2);
    assert!(co.iter().all(|c| c.0.contains("CUST_")));
    assert!(!co.iter().any(|c| c.0.contains("LONE")));
}

#[test]
fn graph_integrity_counts_and_neighbors() {
    let g = ring_fixture();
    assert_eq!(g.node_count(), 8); // 3 resources + 4 customers + 1 merchant
    assert_eq!(g.edge_count(), 8); // 3 device + 3 address + 2 instrument

    let dev = EntityId::new(EntityKind::Device, "DEV_X");
    let users = g.related_of_kind(&dev, EntityKind::Customer);
    assert_eq!(users.len(), 3);
}

#[test]
fn add_edge_missing_endpoint_is_error_not_panic() {
    let mut g = PropertyGraph::new();
    g.upsert_node(Entity {
        id: EntityId::new(EntityKind::Customer, "X"),
        kind: EntityKind::Customer,
        attrs: Default::default(),
    });
    let missing = EntityId::new(EntityKind::Device, "GHOST");
    assert!(g.add_edge(EntityId::new(EntityKind::Customer, "X"), RelationKind::UsesDevice, missing).is_err());
}
