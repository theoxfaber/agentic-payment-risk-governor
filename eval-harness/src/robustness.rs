//! Robustness evaluation: how does detection degrade as data gets MESSY?
//!
//! The headline tables measure performance on clean synthetic worlds. Real
//! evidence pipelines are not clean: behavioral histories arrive late or
//! partially (service lag, retention limits), timestamps drift, and event
//! counters are noisy. This module degrades held-out worlds along those axes
//! and measures what breaks first.
//!
//! Perturbations mirror real failure modes:
//!   - MISSING BEHAVIORAL RECORDS: the evidence service returns nothing for a
//!     fraction of customers (timeout, cold storage, erasure requests).
//!   - TIMING JITTER: purchase→return gaps get uniform noise (clock skew,
//!     timezone handling, batch ingestion delays).
//!   - COUNT NOISE: return/refund counters drift ±1 (missed webhooks,
//!     double-fired events).

use crate::{evaluate_split, InvestigationEngineDetector, DETECTORS};
use dataset_gen::{generate_world, WorldKind, WorldSpec};
use investigation_engine::CustomerBehavior;
use rand::rngs::StdRng;
use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};

/// How messy is the data?
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessLevel {
    /// Pristine — same as the headline tables.
    Clean,
    /// Mild: 10% of behavioral records missing, ±12h timing jitter, 5%
    /// of customers carry a ±1 count error.
    Mild,
    /// Heavy: 30% of records missing, ±48h jitter, 20% count errors.
    Heavy,
}

impl MessLevel {
    pub const ALL: [MessLevel; 3] = [MessLevel::Clean, MessLevel::Mild, MessLevel::Heavy];

    pub fn name(self) -> &'static str {
        match self {
            MessLevel::Clean => "clean",
            MessLevel::Mild => "mild",
            MessLevel::Heavy => "heavy",
        }
    }

    fn params(self) -> (f64, f64, f64) {
        match self {
            MessLevel::Clean => (0.0, 0.0, 0.0),
            MessLevel::Mild => (0.10, 12.0, 0.05),
            MessLevel::Heavy => (0.30, 48.0, 0.20),
        }
    }
}

/// Apply one messiness level to a copy of the world. Deterministic per seed.
pub fn degrade_world(world: &dataset_gen::World, level: MessLevel, seed: u64) -> dataset_gen::World {
    let (drop_rate, jitter_h, noise_rate) = level.params();
    let mut w = world.clone();
    let mut rng = StdRng::seed_from_u64(seed);

    // 1. Missing behavioral records: the investigator must reason through a
    //    keyhole. Graph structure stays intact — only the behavioral join
    //    loses rows, which is exactly what an evidence-service outage looks
    //    like upstream.
    if drop_rate > 0.0 {
        let all_ids: Vec<String> = w.behaviors.keys().cloned().collect();
        let n_drop = (all_ids.len() as f64 * drop_rate) as usize;
        let dropped: std::collections::HashSet<String> = all_ids.choose_multiple(&mut rng, n_drop).cloned().collect();
        w.behaviors.retain(|k, _| !dropped.contains(k));
    }

    // 2. Timing jitter + 3. count noise on whatever survived.
    let ids: Vec<String> = w.behaviors.keys().cloned().collect();
    for id in &ids {
        let b = w.behaviors.get_mut(id).unwrap();
        if jitter_h > 0.0 && !b.purchase_to_return_hours.is_empty() {
            b.purchase_to_return_hours = b
                .purchase_to_return_hours
                .iter()
                .map(|h| (*h + rng.random_range(-jitter_h..jitter_h)).max(0.0))
                .collect();
        }
        if noise_rate > 0.0 && rng.random::<f64>() < noise_rate {
            perturb_counts(&mut rng, b);
        }
    }

    w
}

fn perturb_counts(rng: &mut StdRng, b: &mut CustomerBehavior) {
    // ±1 on one of the counters — a missed webhook or a double-fired event.
    match rng.random_range(0..3u8) {
        0 if b.return_count > 0 => b.return_count -= 1,
        1 => b.return_count += 1,
        _ if b.refund_count > 0 => b.refund_count -= 1,
        _ => b.refund_count += 1,
    }
}

/// One row of the degradation table, pooled across all held-out seeds.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DegradationRow {
    pub level: &'static str,
    /// Pooled precision over abuse worlds.
    pub precision: f64,
    /// Pooled recall over abuse worlds.
    pub recall: f64,
    pub fp: u32,
    pub fn_count: u32,
    pub fn_cost_paise: i64,
    /// Legitimate customers across ALL overlap worlds at this level.
    pub legit_customers: u32,
    /// How many of them got flagged (false positives).
    pub legit_flagged: u32,
    /// Share of escalations that went to a human instead of auto-block —
    /// expected to RISE as data degrades, because confidence falls.
    pub human_review_share: f64,
}

const ABUSE_KINDS: [WorldKind; 5] = [
    WorldKind::ReturnAbuse,
    WorldKind::RefundAbuse,
    WorldKind::DistributedRing,
    WorldKind::MerchantCollusion,
    WorldKind::AdversarialEvasion,
];

const LEGIT_OVERLAP_KINDS: [WorldKind; 2] = [WorldKind::Household, WorldKind::CoincidentalSharing];

fn spec(kind: WorldKind, seed: u64) -> WorldSpec {
    let rings = match kind {
        WorldKind::Household | WorldKind::CoincidentalSharing => 8,
        _ => 6,
    };
    WorldSpec {
        kind,
        n_background: 300,
        n_rings: rings,
        ring_size: 3,
        seed,
    }
}

/// Degradation sweep: every held-out seed × every messiness level. Abuse
/// worlds measure recall survival; legitimate-overlap worlds measure whether
/// false positives appear as evidence thins out.
pub fn run_degradation_sweep(heldout_seeds: &[u64]) -> Vec<DegradationRow> {
    let mut out = Vec::new();

    for level in MessLevel::ALL {
        let mut tp = 0u32;
        let mut fp = 0u32;
        let mut fn_total = 0u32;
        let mut fn_cost = 0i64;
        let mut escalated = 0u32;
        let mut human_reviewed = 0u32;
        let mut legit_flagged = 0u32;
        let mut legit_customers = 0u32;

        for (i, seed) in heldout_seeds.iter().enumerate() {
            for &kind in ABUSE_KINDS.iter().chain(LEGIT_OVERLAP_KINDS.iter()) {
                let clean = generate_world(spec(kind, *seed));
                let degraded = degrade_world(&clean, level, *seed + 100_000 + i as u64);
                let mut det = InvestigationEngineDetector;
                let m = evaluate_split(&degraded, &mut det, "robustness", 0);

                if LEGIT_OVERLAP_KINDS.contains(&kind) {
                    legit_flagged += m.fp;
                    legit_customers += m.tp + m.fp + m.tn + m.fn_count;
                    continue;
                }
                tp += m.tp;
                fp += m.fp;
                fn_total += m.fn_count;
                fn_cost += m.fn_cost_paise;
                escalated += m.auto_blocked + m.human_reviewed;
                human_reviewed += m.human_reviewed;
            }
        }

        let precision = if tp + fp == 0 {
            1.0
        } else {
            tp as f64 / (tp + fp) as f64
        };
        let recall = if tp + fn_total == 0 {
            1.0
        } else {
            tp as f64 / (tp + fn_total) as f64
        };

        out.push(DegradationRow {
            level: level.name(),
            precision,
            recall,
            fp,
            fn_count: fn_total,
            fn_cost_paise: fn_cost,
            legit_customers,
            legit_flagged,
            human_review_share: if escalated == 0 {
                0.0
            } else {
                human_reviewed as f64 / escalated as f64
            },
        });
    }
    out
}

/// Randomized-parameter sweep: instead of fixed world templates, draw world
/// shapes (population size, ring count, ring size, seed) from distributions
/// the detector was never tuned against. Answers "is 100% a property of three
/// hand-built worlds?" with statistics instead of vibes.
///
/// Legitimate-overlap worlds (household / coincidental sharing) are scored
/// INTO the pooled precision — they contain zero abusers, so every flag is a
/// false positive, and excluding them would structurally inflate precision.
///
/// Returns (pooled_precision, pooled_recall, worlds_run, legit_flagged).
pub fn run_randomized_sweep(draws_per_kind: usize, master_seed: u64) -> (f64, f64, usize, u32) {
    let mut rng = StdRng::seed_from_u64(master_seed);
    let mut tp = 0u32;
    let mut fp = 0u32;
    let mut fn_total = 0u32;
    let mut legit_flagged = 0u32;
    let mut worlds = 0usize;

    for _ in 0..draws_per_kind {
        for &kind in ABUSE_KINDS.iter().chain(LEGIT_OVERLAP_KINDS.iter()) {
            let s = WorldSpec {
                kind,
                n_background: rng.random_range(150..450),
                n_rings: rng.random_range(2..9),
                ring_size: rng.random_range(3..6),
                seed: rng.random(),
            };
            let world = generate_world(s);
            let mut det = InvestigationEngineDetector;
            let m = evaluate_split(&world, &mut det, "randomized", 0);
            worlds += 1;

            if LEGIT_OVERLAP_KINDS.contains(&kind) {
                // Pure-legit world: every flag is a false positive and MUST
                // count against pooled precision.
                legit_flagged += m.fp;
                fp += m.fp;
                continue;
            }
            tp += m.tp;
            fp += m.fp;
            fn_total += m.fn_count;
        }
    }

    let precision = if tp + fp == 0 {
        1.0
    } else {
        tp as f64 / (tp + fp) as f64
    };
    let recall = if tp + fn_total == 0 {
        1.0
    } else {
        tp as f64 / (tp + fn_total) as f64
    };
    (precision, recall, worlds, legit_flagged)
}

/// Detector name used in robustness output (keeps DETECTORS referenced here).
pub fn detector_name() -> &'static str {
    DETECTORS[2]
}
