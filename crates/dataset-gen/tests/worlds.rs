use dataset_gen::{baseline_of, generate_world, WorldKind, WorldSpec};

fn spec(kind: WorldKind, seed: u64) -> WorldSpec {
    WorldSpec {
        kind,
        n_background: 200,
        n_rings: 6,
        ring_size: 3,
        seed,
    }
}

#[test]
fn worlds_are_deterministic_given_seed() {
    let a = generate_world(spec(WorldKind::ReturnAbuse, 42));
    let b = generate_world(spec(WorldKind::ReturnAbuse, 42));
    assert_eq!(a.behaviors.len(), b.behaviors.len());
    // spot-check deep equality via serialized behavior of first ring member
    let key = &a.abuse_rings[0][0];
    assert_eq!(
        serde_json::to_string(&a.behaviors[key]).unwrap(),
        serde_json::to_string(&b.behaviors[key]).unwrap()
    );
    assert_eq!(
        serde_json::to_string(&a.abuse_rings).unwrap(),
        serde_json::to_string(&b.abuse_rings).unwrap()
    );
}

#[test]
fn normal_world_has_no_abusers_and_no_clusters() {
    let w = generate_world(spec(WorldKind::Normal, 7));
    assert_eq!(w.abuse_rings.len(), 0);
    assert!(w.ground_truth.values().all(|v| !*v));
    // background customers each own their resources → no clusters ≥ 2
    assert!(w.graph.abuse_ring_clusters(2).is_empty());
}

/// THE false-positive trap: households cluster structurally but are all legit.
#[test]
fn household_world_produces_all_legit_clusters() {
    let w = generate_world(spec(WorldKind::Household, 11));
    let clusters = w.graph.abuse_ring_clusters(2);
    assert!(!clusters.is_empty(), "households must cluster structurally");

    // every member of every cluster is labeled legitimate
    for c in &clusters {
        for m in &c.members {
            let ext = m.0.split_once(':').unwrap().1;
            assert_eq!(
                w.ground_truth.get(ext),
                Some(&false),
                "{ext} must be legit in Household world"
            );
        }
    }

    // and the population baseline stays sane (households don't skew it)
    let bl = baseline_of(&w);
    assert!((0.02..=0.12).contains(&bl.avg_return_rate));
}

#[test]
fn return_abuse_world_structurally_recovers_rings() {
    let w = generate_world(spec(WorldKind::ReturnAbuse, 42));
    let clusters = w.graph.abuse_ring_clusters(2);

    // Every planted ring must appear as some cluster's members (structural recall = 1.0)
    for ring in &w.abuse_rings {
        let found = clusters
            .iter()
            .any(|c| ring.iter().all(|m| c.members.iter().any(|cm| cm.0.ends_with(m))));
        assert!(found, "planted ring {ring:?} not recovered as a cluster");
    }

    // ground truth consistent
    assert_eq!(
        w.ground_truth.values().filter(|v| **v).count(),
        w.abuse_rings.iter().map(|r| r.len()).sum::<usize>()
    );
}

#[test]
fn distributed_ring_shares_instrument_only() {
    let w = generate_world(spec(WorldKind::DistributedRing, 42));
    let clusters = w.graph.abuse_ring_clusters(2);
    assert!(!clusters.is_empty());

    // No ring member should share a DEVICE with another ring member
    use risk_graph::{EntityId, EntityKind};
    for ring in &w.abuse_rings {
        for m in ring {
            let cid = EntityId::new(EntityKind::Customer, m);
            let devices: Vec<_> = w
                .graph
                .related_of_kind(&cid, EntityKind::Device)
                .iter()
                .map(|d| d.id.0.clone())
                .collect();
            // each abuser owns a unique device — clustering came from instrument only
            for other in ring {
                if other != m {
                    let oid = EntityId::new(EntityKind::Customer, other);
                    let other_devices: Vec<_> = w
                        .graph
                        .related_of_kind(&oid, EntityKind::Device)
                        .iter()
                        .map(|d| d.id.0.clone())
                        .collect();
                    assert!(
                        devices.iter().all(|d| !other_devices.contains(d)),
                        "distributed ring members must not share devices"
                    );
                }
            }
        }
    }
}

#[test]
fn adversarial_evasion_still_clusters_but_with_weaker_behavior() {
    let w = generate_world(spec(WorldKind::AdversarialEvasion, 42));

    // structure survives: single shared device links the ring
    let clusters = w.graph.abuse_ring_clusters(2);
    assert!(!clusters.is_empty(), "evasion must not break structural clustering");

    // but behavioral sync is destroyed by jitter: median gap spread per member is wide.
    // The investigator's synchronized_returns signal should rarely fire — asserted
    // at eval level; here we just verify the fixture has wide spreads.
    let abuser = &w.abuse_rings[0][0];
    let b = &w.behaviors[abuser];
    let mut gaps = b.purchase_to_return_hours.clone();
    gaps.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let spread = gaps.last().unwrap_or(&0.0) - gaps.first().unwrap_or(&0.0);
    assert!(
        spread > 50.0,
        "evasion jitter should produce spread > 50h, got {spread}"
    );
}
