//! Synthetic adversarial dataset — seven worlds, fully seeded (deterministic).
//!
//! | World | Purpose                                   |
//! |-------|-------------------------------------------|
//! | A     | Normal population (baseline)              |
//! | B     | Households — shared device/address, LEGIT (the FP trap) |
//! | C     | Coordinated return abuse rings            |
//! | D     | Refund-abuse rings (refunds w/o returns)  |
//! | E     | Distributed rings — share instrument only, no device |
//! | F     | Merchant-collusion rings                  |
//! | G     | Adversarial evasion — minimal sharing, jittered timing |
//!
//! Everything is generated from `seed` so the eval table in the README is
//! reproducible byte-for-byte.

use investigation_engine::CustomerBehavior;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use risk_graph::{EntityKind, GraphBuilder, PropertyGraph, RelationKind};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldKind {
    Normal,
    Household,
    /// Legit background customers share resources with STRANGERS:
    /// NAT/CGNAT IPs, popular device models, reused address formats.
    /// Exists to destroy the 100%-precision artifact — in the real world,
    /// resource overlap does not imply coordination.
    CoincidentalSharing,
    ReturnAbuse,
    RefundAbuse,
    DistributedRing,
    MerchantCollusion,
    AdversarialEvasion,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorldSpec {
    pub kind: WorldKind,
    /// background (non-ring) customers
    pub n_background: usize,
    pub n_rings: usize,
    pub ring_size: usize,
    pub seed: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct World {
    pub name: String,
    #[serde(skip)]
    pub graph: PropertyGraph,
    pub behaviors: HashMap<String, CustomerBehavior>,
    /// external customer id -> total exposed refund value (paise)
    pub exposure_paise: HashMap<String, i64>,
    /// external customer id -> is_abuser ground truth
    pub ground_truth: HashMap<String, bool>,
    /// known abuse rings by external customer id (cluster-recall target)
    pub abuse_rings: Vec<Vec<String>>,
}

/// Population baseline computed from the world's own background customers —
/// the investigator compares clusters against the world it lives in.
pub fn baseline_of(world: &World) -> investigation_engine::Baseline {
    let bg: Vec<&CustomerBehavior> = world
        .behaviors
        .iter()
        .filter(|(id, _)| !world.ground_truth.get(id.as_str()).copied().unwrap_or(false))
        .map(|(_, b)| b)
        .collect();
    let orders: u32 = bg.iter().map(|b| b.order_count).sum();
    let returns: u32 = bg.iter().map(|b| b.return_count).sum();
    let rate = if orders == 0 {
        0.05
    } else {
        returns as f64 / orders as f64
    };
    investigation_engine::Baseline {
        avg_return_rate: rate,
        return_rate_anomaly_multiplier: 2.5,
    }
}

pub fn generate_world(spec: WorldSpec) -> World {
    let mut rng = StdRng::seed_from_u64(spec.seed);
    let mut b = Builder::default();

    // --- background population (World A semantics for every world) ---
    for i in 0..spec.n_background {
        let id = format!("BG{i:04}");
        let dev = format!("DEV_BG{i:04}");
        let adr = format!("ADR_BG{i:04}");
        let pin = format!("PIN_BG{i:04}");
        push_background_customer(&mut rng, &mut b, &id, &dev, &adr, &pin);
    }

    // --- households: shared resources, legitimate behavior ---
    if matches!(spec.kind, WorldKind::Household) {
        for h in 0..spec.n_rings {
            let dev = format!("DEV_HH{h}");
            let adr = format!("ADR_HH{h}");
            let members: Vec<String> = (0..spec.ring_size).map(|m| format!("HH{h}_{m}")).collect();
            for m in &members {
                push_household_member(&mut rng, &mut b, m, &dev, &adr);
            }
        }
    }

    // --- coincidental sharing: strangers overlap via infrastructure ---
    // NAT gateways (many customers, one IP), popular device models
    // (same phone ≠ same person), reused address strings (hostel/office
    // blocks). Every participant is LEGITIMATE — this world exists to
    // destroy the 100%-precision artifact.
    if matches!(spec.kind, WorldKind::CoincidentalSharing) {
        for g in 0..spec.n_rings {
            // Group 1: strangers behind one NAT gateway.
            let nat = format!("IP_NAT{g}");
            for i in 0..(spec.ring_size + 2) {
                let id = format!("NAT{g}_{i}");
                push_background_customer(
                    &mut rng,
                    &mut b,
                    &id,
                    &format!("DEV_NAT{g}_{i}"),
                    &format!("ADR_NAT{g}_{i}"),
                    &format!("PIN_NAT{g}_{i}"),
                );
                b.relate_cust_ip(&id, &nat);
            }
            // Group 2: strangers who happen to own the same popular device.
            let popdev = format!("DEV_POPULAR{g}");
            for i in 0..spec.ring_size {
                let id = format!("POP{g}_{i}");
                push_background_customer(
                    &mut rng,
                    &mut b,
                    &id,
                    &popdev,
                    &format!("ADR_POP{g}_{i}"),
                    &format!("PIN_POP{g}_{i}"),
                );
            }
            // Group 3: strangers reusing an address string (hostel/office).
            let blk = format!("ADR_BLOCK{g}");
            for i in 0..spec.ring_size {
                let id = format!("BLK{g}_{i}");
                push_background_customer(
                    &mut rng,
                    &mut b,
                    &id,
                    &format!("DEV_BLK{g}_{i}"),
                    &blk,
                    &format!("PIN_BLK{g}_{i}"),
                );
            }
        }
    }

    // --- abuse rings ---
    let ring_kind = matches!(
        spec.kind,
        WorldKind::ReturnAbuse
            | WorldKind::RefundAbuse
            | WorldKind::DistributedRing
            | WorldKind::MerchantCollusion
            | WorldKind::AdversarialEvasion
    );
    if ring_kind {
        for r in 0..spec.n_rings {
            let members: Vec<String> = (0..spec.ring_size).map(|m| format!("AB{r:02}_{m}")).collect();

            let (dev_shared, adr_shared, pin_shared) = match spec.kind {
                // classic: everything shared
                WorldKind::ReturnAbuse | WorldKind::RefundAbuse => (true, true, true),
                // distributed: no device, no address — instrument only
                WorldKind::DistributedRing => (false, false, true),
                // collusion: full sharing + one merchant sink
                WorldKind::MerchantCollusion => (true, true, true),
                // evasion: exactly ONE resource type
                WorldKind::AdversarialEvasion => (true, false, false),
                _ => unreachable!(),
            };
            let dev = format!("DEV_AB{r:02}");
            let adr = format!("ADR_AB{r:02}");
            let pin = format!("PIN_AB{r:02}");
            let merchant = format!("MER_COLL{r:02}");

            for m in &members {
                match spec.kind {
                    WorldKind::RefundAbuse => push_refund_abuser(&mut rng, &mut b, m),
                    WorldKind::DistributedRing => push_distributed_abuser(&mut rng, &mut b, m),
                    WorldKind::AdversarialEvasion => push_evasive_abuser(&mut rng, &mut b, m),
                    _ => push_return_abuser(&mut rng, &mut b, m),
                }

                if dev_shared {
                    b.relate_cust_dev(m, &dev);
                }
                if adr_shared {
                    b.relate_cust_adr(m, &adr);
                }
                if pin_shared {
                    b.relate_cust_pin(m, &pin);
                }
                // Adversarial evasion handled by pairwise slips after the loop.

                if matches!(spec.kind, WorldKind::MerchantCollusion) {
                    // every payment flows to the colluding merchant
                    ingest_payment_to_merchant(&mut b, m, &merchant);
                }
                b.truth.insert(m.clone(), true);
            }
            // Post-pass for evasion rings: pairwise instrument slips along
            // the ring (m0↔m1, m1↔m2, ...). Transitivity still clusters the
            // whole ring; per-pair overlap stays minimal. Seeded rng decides
            // which pairs slip (~50%).
            if matches!(spec.kind, WorldKind::AdversarialEvasion) {
                let pin = format!("PIN_AB{r:02}");
                for i in 0..members.len().saturating_sub(1) {
                    if !rng.random_bool(0.5) {
                        continue;
                    }
                    let slip = format!("{pin}_slip{i}");
                    b.gb =
                        b.gb.clone()
                            .relate(
                                EntityKind::Customer,
                                &members[i],
                                RelationKind::UsesInstrument,
                                EntityKind::PaymentInstrument,
                                &slip,
                            )
                            .relate(
                                EntityKind::Customer,
                                &members[i + 1],
                                RelationKind::UsesInstrument,
                                EntityKind::PaymentInstrument,
                                &slip,
                            );
                }
            }
            b.rings.push(members);
        }
    }

    World {
        name: format!(
            "{:?}_bg{}_r{}x{}",
            spec.kind, spec.n_background, spec.n_rings, spec.ring_size
        )
        .to_lowercase(),
        graph: b.gb.build(),
        behaviors: b.behaviors,
        exposure_paise: b.exposure,
        ground_truth: b.truth,
        abuse_rings: b.rings,
    }
}

// ---------------------------------------------------------------------------
// Per-archetype generators (rng-seeded)
// ---------------------------------------------------------------------------

fn push_background_customer(rng: &mut StdRng, b: &mut Builder, id: &str, dev: &str, adr: &str, pin: &str) {
    let orders = rng.random_range(20..80u32);
    let returns = ((orders as f64) * rng.random_range(0.01..0.10)) as u32;
    let behavior = CustomerBehavior {
        customer_id: id.into(),
        order_count: orders,
        return_count: returns,
        refund_count: returns,
        dispute_count: rng.random_range(0..2u32),
        distinct_merchants: rng.random_range(3..12u32),
        distinct_products: rng.random_range(5..30u32),
        account_age_days: rng.random_range(200..1500u64),
        purchase_to_return_hours: (0..returns.max(1)).map(|_| rng.random_range(200.0..900.0)).collect(),
    };
    b.push_behavior(id, behavior, false);

    b.relate_cust_dev(id, dev);
    b.relate_cust_adr(id, adr);
    b.relate_cust_pin(id, pin);
    add_order_edges(b, id, orders.min(3));
}

fn push_household_member(rng: &mut StdRng, b: &mut Builder, id: &str, dev: &str, adr: &str) {
    let orders = rng.random_range(25..70u32);
    let returns = ((orders as f64) * rng.random_range(0.02..0.09)) as u32;
    let behavior = CustomerBehavior {
        customer_id: id.into(),
        order_count: orders,
        return_count: returns,
        refund_count: returns,
        dispute_count: rng.random_range(0..2u32),
        distinct_merchants: rng.random_range(4..12u32),
        distinct_products: rng.random_range(6..24u32),
        account_age_days: rng.random_range(300..1200u64), // family = established
        purchase_to_return_hours: vec![rng.random_range(100.0..800.0)], // unsynchronized
    };
    b.push_behavior(id, behavior, false);
    b.relate_cust_dev(id, dev);
    b.relate_cust_adr(id, adr);
    add_order_edges(b, id, orders.min(3));
}

fn push_return_abuser(rng: &mut StdRng, b: &mut Builder, id: &str) {
    let orders = rng.random_range(8..15u32);
    // Rates STRADDLE the naive rule threshold (~3× baseline ≈ 0.15): real
    // rings keep each account below per-customer alarms. The cluster's
    // aggregate rate is what exposes them — that's the graph's entire value.
    let returns = ((orders as f64) * rng.random_range(0.08..0.28)) as u32;
    let behavior = CustomerBehavior {
        customer_id: id.into(),
        order_count: orders,
        return_count: returns,
        refund_count: returns,
        dispute_count: 0,
        distinct_merchants: rng.random_range(1..2u32),
        distinct_products: rng.random_range(1..3u32),
        account_age_days: rng.random_range(5..40u64),
        purchase_to_return_hours: (0..returns.max(1)).map(|_| rng.random_range(18.0..36.0)).collect(), // tight sync
    };
    b.push_behavior(id, behavior, true);
    add_order_edges(b, id, orders.min(3));
}

fn push_refund_abuser(rng: &mut StdRng, b: &mut Builder, id: &str) {
    let orders = rng.random_range(6..12u32);
    // Floor at 2: a member with <2 refunds isn't taking refunds at all,
    // and would silently deflate the refund-abuse pattern we're modeling.
    let refunds = (((orders as f64) * rng.random_range(0.15..0.35)).round() as u32).max(2);
    let behavior = CustomerBehavior {
        customer_id: id.into(),
        order_count: orders,
        return_count: 0, // refunds WITHOUT returns — the tell
        refund_count: refunds,
        dispute_count: 0,
        distinct_merchants: 1,
        distinct_products: 1,
        account_age_days: rng.random_range(3..30u64),
        purchase_to_return_hours: vec![],
    };
    b.push_behavior(id, behavior, true);
    add_order_edges(b, id, orders.min(3));
}

fn push_distributed_abuser(rng: &mut StdRng, b: &mut Builder, id: &str) {
    let orders = rng.random_range(8..14u32);
    let returns = ((orders as f64) * rng.random_range(0.08..0.25)) as u32;
    let behavior = CustomerBehavior {
        customer_id: id.into(),
        order_count: orders,
        return_count: returns,
        refund_count: returns,
        dispute_count: 0,
        distinct_merchants: rng.random_range(1..3u32),
        distinct_products: rng.random_range(1..4u32),
        account_age_days: rng.random_range(10..60u64),
        purchase_to_return_hours: (0..returns.max(1)).map(|_| rng.random_range(24.0..60.0)).collect(), // loose sync
    };
    b.push_behavior(id, behavior, true);
    add_order_edges(b, id, orders.min(2));
}

fn push_evasive_abuser(rng: &mut StdRng, b: &mut Builder, id: &str) {
    let orders = rng.random_range(10..18u32);
    let returns = ((orders as f64) * rng.random_range(0.06..0.22)) as u32; // moderated rate
    let behavior = CustomerBehavior {
        customer_id: id.into(),
        order_count: orders,
        return_count: returns,
        refund_count: returns,
        dispute_count: 0,
        distinct_merchants: rng.random_range(2..4u32), // some diversity to look legit
        distinct_products: rng.random_range(2..5u32),
        account_age_days: rng.random_range(30..90u64), // aged accounts
        // WIDE jitter: defeats synchronized_returns detection
        purchase_to_return_hours: (0..returns.max(1)).map(|_| rng.random_range(12.0..140.0)).collect(),
    };
    b.push_behavior(id, behavior, true);
    add_order_edges(b, id, orders.min(2));
}

fn add_order_edges(b: &mut Builder, cust: &str, n_orders: u32) {
    let mer_ext = format!("MER_{cust}");
    b.gb = b.gb.clone().entity(EntityKind::Merchant, &mer_ext);
    for i in 0..n_orders {
        let pay = format!("{cust}_PAY{i}");
        let ord = format!("{cust}_ORD{i}");
        b.gb =
            b.gb.clone()
                .entity(EntityKind::Payment, &pay)
                .entity(EntityKind::Order, &ord)
                .relate(
                    EntityKind::Customer,
                    cust,
                    RelationKind::Made,
                    EntityKind::Payment,
                    &pay,
                )
                .relate(
                    EntityKind::Payment,
                    &pay,
                    RelationKind::BelongsTo,
                    EntityKind::Merchant,
                    &mer_ext,
                )
                .relate(
                    EntityKind::Payment,
                    &pay,
                    RelationKind::FulfilledOrder,
                    EntityKind::Order,
                    &ord,
                );
    }
}

fn ingest_payment_to_merchant(b: &mut Builder, cust: &str, merchant: &str) {
    let pay = format!("{cust}_COLL_PAY");
    b.gb =
        b.gb.clone()
            .entity(EntityKind::Payment, &pay)
            .entity(EntityKind::Merchant, merchant)
            .relate(
                EntityKind::Customer,
                cust,
                RelationKind::Made,
                EntityKind::Payment,
                &pay,
            )
            .relate(
                EntityKind::Payment,
                &pay,
                RelationKind::BelongsTo,
                EntityKind::Merchant,
                merchant,
            );
}

// ---------------------------------------------------------------------------
struct Builder {
    gb: GraphBuilder,
    behaviors: HashMap<String, CustomerBehavior>,
    exposure: HashMap<String, i64>,
    truth: HashMap<String, bool>,
    rings: Vec<Vec<String>>,
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            gb: GraphBuilder::new(),
            behaviors: HashMap::new(),
            exposure: HashMap::new(),
            truth: HashMap::new(),
            rings: Vec::new(),
        }
    }
}

impl Builder {
    fn push_behavior(&mut self, id: &str, behavior: CustomerBehavior, abusive: bool) {
        // Exposure: abusers cash out ~their refund volume; legit ~quarter that.
        let exposure = if abusive {
            (behavior.refund_count.max(behavior.return_count) as i64) * 45_000 // ~₹450/refund
        } else {
            (behavior.return_count as i64) * 45_000 / 4
        };
        self.exposure.insert(id.to_string(), exposure);
        self.truth.insert(id.to_string(), abusive);
        self.behaviors.insert(id.to_string(), behavior);
    }

    fn relate_cust_dev(&mut self, cust: &str, dev: &str) {
        self.gb = self.gb.clone().relate(
            EntityKind::Customer,
            cust,
            RelationKind::UsesDevice,
            EntityKind::Device,
            dev,
        );
    }

    fn relate_cust_adr(&mut self, cust: &str, adr: &str) {
        self.gb = self.gb.clone().relate(
            EntityKind::Customer,
            cust,
            RelationKind::ShipsTo,
            EntityKind::Address,
            adr,
        );
    }

    fn relate_cust_pin(&mut self, cust: &str, pin: &str) {
        self.gb = self.gb.clone().relate(
            EntityKind::Customer,
            cust,
            RelationKind::UsesInstrument,
            EntityKind::PaymentInstrument,
            pin,
        );
    }

    fn relate_cust_ip(&mut self, cust: &str, ip: &str) {
        self.gb = self.gb.clone().relate(
            EntityKind::Customer,
            cust,
            RelationKind::FromIp,
            EntityKind::IpAddress,
            ip,
        );
    }
}
