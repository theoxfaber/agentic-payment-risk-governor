//! Eval harness: runs all worlds through three detectors and produces the
//! honest comparison table (precision / recall / FP-cost / FN-cost /
//! prevented value).
//!
//! Detectors:
//!   1. `per_customer_rate_rule` — classic per-account velocity rule. No
//!      graph, no clusters. Flags any account whose own return+refund rate
//!      crosses 3× the world baseline.
//!   2. `structural_cluster_only` — flag every member of any resource-sharing
//!      cluster. No behavior checks. Maximum recall, brutal FP cost.
//!   3. `investigation_engine` — clusters + hypothesis testing with
//!      counter-evidence. Supported → auto-block; Conflicted → human review;
//!      Unsupported → clear.

use dataset_gen::{baseline_of, World};
use investigation_engine::Investigator;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Escalation {
    AutoBlock,
    HumanReview,
    Clear,
}

pub trait Detector {
    fn name(&self) -> &'static str;
    fn scan(&mut self, world: &World) -> HashMap<String, Escalation>;
}

// ---------------------------------------------------------------------------
// Detector 1: per-customer rate rule (the "existing system" strawman — but a
// fair one: it sees returns AND refunds)
// ---------------------------------------------------------------------------

pub struct PerCustomerRateRule;

impl Detector for PerCustomerRateRule {
    fn name(&self) -> &'static str {
        "per_customer_rate_rule"
    }

    fn scan(&mut self, world: &World) -> HashMap<String, Escalation> {
        let baseline = baseline_of(world);
        let threshold = baseline.avg_return_rate * baseline.return_rate_anomaly_multiplier;
        world
            .behaviors
            .iter()
            .map(|(id, b)| {
                // max() not sum(): returns and refunds usually describe the
                // same money event; naive summation double-counts normal
                // customers and torches precision.
                let rate = if b.order_count == 0 {
                    0.0
                } else {
                    b.return_count.max(b.refund_count) as f64 / b.order_count as f64
                };
                let esc = if rate > threshold {
                    Escalation::AutoBlock
                } else {
                    Escalation::Clear
                };
                (id.clone(), esc)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Detector 2: structural clustering only
// ---------------------------------------------------------------------------

pub struct StructuralClusterOnly {
    min_cluster_size: usize,
}

impl Default for StructuralClusterOnly {
    fn default() -> Self {
        Self { min_cluster_size: 2 }
    }
}

impl Detector for StructuralClusterOnly {
    fn name(&self) -> &'static str {
        "structural_cluster_only"
    }

    fn scan(&mut self, world: &World) -> HashMap<String, Escalation> {
        let mut out: HashMap<String, Escalation> =
            world.behaviors.keys().map(|k| (k.clone(), Escalation::Clear)).collect();
        for c in world.graph.abuse_ring_clusters(self.min_cluster_size) {
            for m in c.members {
                if let Some(ext) = m.0.split_once(':').map(|(_, e)| e.to_string()) {
                    out.insert(ext, Escalation::AutoBlock);
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Detector 3: investigation engine (ours)
// ---------------------------------------------------------------------------

pub struct InvestigationEngineDetector;

impl Detector for InvestigationEngineDetector {
    fn name(&self) -> &'static str {
        "investigation_engine"
    }

    fn scan(&mut self, world: &World) -> HashMap<String, Escalation> {
        let mut out: HashMap<String, Escalation> =
            world.behaviors.keys().map(|k| (k.clone(), Escalation::Clear)).collect();

        let inv = Investigator::new(baseline_of(world));
        for cluster in world.graph.abuse_ring_clusters(2) {
            let result = inv.investigate_return_abuse(&world.graph, &cluster, &world.behaviors, &world.exposure_paise);

            let esc = if !result.should_hold_funds() {
                Escalation::Clear
            } else if result.requires_human() {
                // Conflicted, low-confidence-supported, or unconfirmed
                // structural linkage — a human decides.
                Escalation::HumanReview
            } else {
                Escalation::AutoBlock
            };

            if esc != Escalation::Clear {
                for m in &cluster.members {
                    if let Some(ext) = m.0.split_once(':').map(|(_, e)| e.to_string()) {
                        out.insert(ext, esc);
                    }
                }
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct WorldMetrics {
    pub world: String,
    /// "calibration" (thresholds were tuned against these worlds) or
    /// "held-out" (seeds the detector never saw during development).
    pub split: &'static str,
    pub seed: u64,
    pub detector: &'static str,
    pub tp: u32,
    pub fp: u32,
    pub tn: u32,
    pub fn_count: u32,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    /// exposure (paise) of wrongly flagged legit customers — friction cost
    pub fp_cost_paise: i64,
    /// exposure (paise) of abusers missed — direct loss
    pub fn_cost_paise: i64,
    /// exposure (paise) of abusers caught
    pub prevented_paise: i64,
    pub auto_blocked: u32,
    pub human_reviewed: u32,
}

pub fn evaluate(world: &World, detector: &mut dyn Detector) -> WorldMetrics {
    evaluate_split(world, detector, "unspecified", 0)
}

pub fn evaluate_split(world: &World, detector: &mut dyn Detector, split: &'static str, seed: u64) -> WorldMetrics {
    let detections = detector.scan(world);

    let mut tp = 0u32;
    let mut fp = 0u32;
    let mut tn = 0u32;
    let mut fn_count = 0u32;
    let mut fp_cost = 0i64;
    let mut fn_cost = 0i64;
    let mut prevented = 0i64;
    let mut auto_blocked = 0u32;
    let mut human_reviewed = 0u32;

    for (id, esc) in &detections {
        let is_abuser = world.ground_truth.get(id.as_str()).copied().unwrap_or(false);
        let exposure = world.exposure_paise.get(id.as_str()).copied().unwrap_or(0);
        let escalated = *esc != Escalation::Clear;

        match (*esc, is_abuser) {
            (Escalation::AutoBlock, _) => auto_blocked += 1,
            (Escalation::HumanReview, _) => human_reviewed += 1,
            _ => {}
        }

        match (escalated, is_abuser) {
            (true, true) => {
                tp += 1;
                prevented += exposure;
            }
            (true, false) => {
                fp += 1;
                fp_cost += exposure;
            }
            (false, true) => {
                fn_count += 1;
                fn_cost += exposure;
            }
            (false, false) => tn += 1,
        }
    }

    let precision = if tp + fp == 0 {
        1.0
    } else {
        tp as f64 / (tp + fp) as f64
    };
    let recall = if tp + fn_count == 0 {
        1.0
    } else {
        tp as f64 / (tp + fn_count) as f64
    };
    let f1 = if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    };

    WorldMetrics {
        world: world.name.clone(),
        split,
        seed,
        detector: detector.name(),
        tp,
        fp,
        tn,
        fn_count,
        precision,
        recall,
        f1,
        fp_cost_paise: fp_cost,
        fn_cost_paise: fn_cost,
        prevented_paise: prevented,
        auto_blocked,
        human_reviewed,
    }
}

/// Detector names in canonical order (used by the CLI aggregate output).
pub const DETECTORS: &[&str] = &[
    "per_customer_rate_rule",
    "structural_cluster_only",
    "investigation_engine",
];

/// The seed every threshold in the investigation engine was tuned against.
pub const CALIBRATION_SEED: u64 = 2026;

/// Seeds the detector NEVER saw during development. Headline numbers are
/// reported on these worlds only — a model evaluated on the data it was
/// tuned against proves nothing (Track-02 bar: held-out test set).
pub const HELDOUT_SEEDS: &[u64] = &[31_415, 27_182, 16_180];

fn run_world_set(seed: u64, split: &'static str) -> Vec<WorldMetrics> {
    use dataset_gen::WorldKind;
    let specs = [
        WorldSpecShorthand::new(WorldKind::Normal, 300, 0, 3, seed),
        WorldSpecShorthand::new(WorldKind::Household, 300, 8, 3, seed),
        // Coincidental sharing: NAT IPs, popular devices, reused addresses.
        // All legit — measures honest precision under real-world overlap.
        WorldSpecShorthand::new(WorldKind::CoincidentalSharing, 300, 8, 3, seed),
        WorldSpecShorthand::new(WorldKind::ReturnAbuse, 300, 6, 3, seed),
        WorldSpecShorthand::new(WorldKind::RefundAbuse, 300, 6, 3, seed),
        WorldSpecShorthand::new(WorldKind::DistributedRing, 300, 6, 3, seed),
        WorldSpecShorthand::new(WorldKind::MerchantCollusion, 300, 6, 3, seed),
        WorldSpecShorthand::new(WorldKind::AdversarialEvasion, 300, 6, 3, seed),
    ];

    let mut results = Vec::new();
    for s in specs {
        let world = dataset_gen::generate_world(s.spec);
        for det in [
            &mut PerCustomerRateRule as &mut dyn Detector,
            &mut StructuralClusterOnly::default() as &mut dyn Detector,
            &mut InvestigationEngineDetector as &mut dyn Detector,
        ] {
            results.push(evaluate_split(&world, det, split, seed));
        }
    }
    results
}

/// Calibration worlds: what the thresholds were developed against. Shown for
/// transparency; NOT the headline numbers.
pub fn run_calibration() -> Vec<WorldMetrics> {
    run_world_set(CALIBRATION_SEED, "calibration")
}

/// Held-out worlds: three fresh seeds per world kind. THE headline numbers.
pub fn run_held_out() -> Vec<WorldMetrics> {
    HELDOUT_SEEDS
        .iter()
        .flat_map(|s| run_world_set(*s, "held-out"))
        .collect()
}

/// The full sweep: calibration + held-out.
pub fn run_all() -> Vec<WorldMetrics> {
    let mut results = run_calibration();
    results.extend(run_held_out());
    results
}

struct WorldSpecShorthand {
    spec: dataset_gen::WorldSpec,
}

impl WorldSpecShorthand {
    fn new(kind: dataset_gen::WorldKind, bg: usize, rings: usize, size: usize, seed: u64) -> Self {
        Self {
            spec: dataset_gen::WorldSpec {
                kind,
                n_background: bg,
                n_rings: rings,
                ring_size: size,
                seed,
            },
        }
    }
}

/// Markdown table for the README — one row per (world, detector).
pub fn render_markdown(results: &[&WorldMetrics]) -> String {
    let mut s = String::from(
        "| Split | Seed | World | Detector | Precision | Recall | F1 | FP cost | FN cost | Prevented | Auto | Review |\n\
         |---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|\n",
    );
    for m in results {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {:.0}% | {:.0}% | {:.2} | ₹{:.0} | ₹{:.0} | ₹{:.0} | {} | {} |\n",
            m.split,
            m.seed,
            m.world,
            m.detector,
            m.precision * 100.0,
            m.recall * 100.0,
            m.f1,
            m.fp_cost_paise as f64 / 100.0,
            m.fn_cost_paise as f64 / 100.0,
            m.prevented_paise as f64 / 100.0,
            m.auto_blocked,
            m.human_reviewed,
        ));
    }
    s
}
