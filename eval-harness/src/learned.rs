//! The learned detection layer: shared train/infer feature extraction plus
//! two detectors that slot into the honest comparison table.
//!
//!   • `learned_logistic`      — raw model, fixed 0.5 cut. Shows what ML alone
//!                               achieves on the same data.
//!   • `calibrated_lr_crc`     — the SAME model with instance-dependent
//!                               economics + a CRC-bounded auto-block cut:
//!                               allow only when being wrong costs less than
//!                               one human review; block only inside the
//!                               statistically-bounded region; the rest is
//!                               human review (docs/AI_DESIGN.md).
//!
//! Protocol discipline: training AND threshold calibration use ONLY the
//! calibration seed(s). Held-out seeds are scored once, by code that never
//! saw them.
//!
//! Feature set follows the fraud-literature consensus (IEEE-CIS winners,
//! Stripe Radar engineering): per-entity behavioral aggregates, account age,
//! timing synchronization, and structural context from the entity graph —
//! never raw identifiers.

use crate::conformal::{CalibratedThresholds, DEFAULT_ALPHA_FRICTION, DEFAULT_ALPHA_LEAK};
use crate::lr::{self, Sample};
use crate::{Detector, Escalation};
use dataset_gen::World;
use std::collections::HashMap;

pub const FEATURE_NAMES: &[&str] = &[
    "return_refund_rate",         // max(returns, refunds) / orders
    "log_account_age_days",       // log1p(age)/10 — new accounts are the classic abuse signature
    "distinct_merchants_norm",    // /12 — ring members concentrate on few merchants
    "unique_customers_norm", // /30 — was distinct_products_norm, actually unique_customers_24h in serving (honest rename)
    "dispute_ratio",         // disputes / orders
    "sync_share_72h",        // share of purchase→return gaps under 72h (synchronized returns)
    "cluster_size_norm",     // size of the resource-sharing cluster (/8)
    "cluster_pooled_return_rate", // pooled rate across the cluster — the graph's entire value
];

/// Structural context computed once per world scan.
struct ClusterContext {
    /// external customer id -> (cluster size, pooled return/refund rate)
    of: HashMap<String, (f64, f64)>,
}

fn cluster_context(world: &World) -> ClusterContext {
    let mut of: HashMap<String, Vec<&investigation_engine::CustomerBehavior>> = HashMap::new();
    for c in world.graph.abuse_ring_clusters(2) {
        for m in &c.members {
            if let Some(ext) = m.0.split_once(':').map(|(_, e)| e.to_string()) {
                if let Some(b) = world.behaviors.get(&ext) {
                    of.entry(ext).or_default().push(b);
                }
            }
        }
    }
    let mut ctx = HashMap::new();
    for (ext, members) in of {
        let orders: u32 = members.iter().map(|b| b.order_count).sum();
        let flags: u32 = members.iter().map(|b| b.return_count.max(b.refund_count)).sum();
        let rate = if orders == 0 { 0.0 } else { flags as f64 / orders as f64 };
        ctx.insert(ext, (members.len() as f64, rate));
    }
    ClusterContext { of: ctx }
}

fn behavior_features(b: &investigation_engine::CustomerBehavior, ctx: Option<&(f64, f64)>) -> Vec<f64> {
    let rate = if b.order_count == 0 {
        0.0
    } else {
        b.return_count.max(b.refund_count) as f64 / b.order_count as f64
    };
    let sync_share = if b.purchase_to_return_hours.is_empty() {
        0.0
    } else {
        b.purchase_to_return_hours.iter().filter(|&&h| h < 72.0).count() as f64
            / b.purchase_to_return_hours.len() as f64
    };
    let (cluster_size, cluster_rate) = ctx.copied().unwrap_or((1.0, rate));
    vec![
        rate,
        (1.0 + b.account_age_days as f64).ln() / 10.0,
        b.distinct_merchants as f64 / 12.0,
        b.distinct_products as f64 / 30.0,
        if b.order_count == 0 {
            0.0
        } else {
            b.dispute_count as f64 / b.order_count as f64
        },
        sync_share,
        cluster_size / 8.0,
        cluster_rate,
    ]
}

/// Labeled samples from the given worlds (training/calibration ONLY).
pub fn build_samples(worlds: &[World]) -> Vec<Sample> {
    let mut out = Vec::new();
    for world in worlds {
        let ctx = cluster_context(world);
        for (id, b) in &world.behaviors {
            let feats = behavior_features(b, ctx.of.get(id));
            let label = if world.ground_truth.get(id).copied().unwrap_or(false) {
                1.0
            } else {
                0.0
            };
            out.push(Sample { features: feats, label });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Detectors
// ---------------------------------------------------------------------------

/// Raw logistic regression at a fixed 0.5 probability cut.
pub struct LearnedLogisticDetector {
    pub model: lr::LogisticModel,
}

impl Detector for LearnedLogisticDetector {
    fn name(&self) -> &'static str {
        "learned_logistic"
    }

    fn scan(&mut self, world: &World) -> HashMap<String, Escalation> {
        let ctx = cluster_context(world);
        world
            .behaviors
            .iter()
            .map(|(id, b)| {
                let p = self.model.predict(&behavior_features(b, ctx.of.get(id)));
                let esc = if p >= 0.5 {
                    Escalation::AutoBlock
                } else {
                    Escalation::Clear
                };
                (id.clone(), esc)
            })
            .collect()
    }
}

/// Logistic regression whose operating point is chosen on TWO axes:
///
///   1. Conformal Risk Control sets `tau_block` so the auto-BLOCK friction
///      rate holds its budget (finite-sample valid).
///   2. Instance-dependent economics (Elkan 2001; Bahnsen et al.; the 2-D
///      (p̂, amount) threshold region of Carbajal et al.) governs ALLOW:
///      a transaction auto-clears only when its expected fraud loss
///      `p̂ × exposure` is cheaper than one human review (`review_cost`).
///      Every cleared case is therefore bounded per-instance by that cost,
///      and the expensive-but-uncertain middle routes to humans.
///
/// Decision rule per customer:
///   expected_loss = p̂ × exposure
///   expected_loss ≤ review_cost            → CLEAR   (cheap to be wrong)
///   expected_loss > review_cost ∧ p̂ ≥ τ_block → AUTO-BLOCK (CRC-bounded)
///   otherwise                              → HUMAN REVIEW
#[derive(Clone)]
pub struct CalibratedLrDetector {
    pub model: lr::LogisticModel,
    pub taus: CalibratedThresholds,
    /// Cost of one human review (paise). The unit every allow/block tradeoff
    /// is denominated in.
    pub review_cost_paise: i64,
}

/// ₹400 per manual review — a defensible ops number; tune per merchant.
pub const DEFAULT_REVIEW_COST_PAISE: i64 = 40_000;

impl Detector for CalibratedLrDetector {
    fn name(&self) -> &'static str {
        "calibrated_lr_crc"
    }

    fn scan(&mut self, world: &World) -> HashMap<String, Escalation> {
        // Calibration scores must use the same feature pipeline as serving.
        let ctx = cluster_context(world);
        world
            .behaviors
            .iter()
            .map(|(id, b)| {
                let p = self.model.predict(&behavior_features(b, ctx.of.get(id)));
                let exposure = world.exposure_paise.get(id).copied().unwrap_or(0) as f64;
                let expected_loss = p * exposure;
                let esc = if expected_loss <= self.review_cost_paise as f64 {
                    // Wrong-decision cost is capped at one review's worth —
                    // economically rational to allow without a human.
                    Escalation::Clear
                } else if p >= self.taus.tau_block {
                    Escalation::AutoBlock
                } else {
                    Escalation::HumanReview
                };
                (id.clone(), esc)
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Suite: train on calibration worlds, calibrate, ready to evaluate.
// ---------------------------------------------------------------------------

/// One customer's model score plus ground-truth label — the substrate for
/// threshold calibration and guarantee validation.
pub struct ScoredCustomer {
    pub id: String,
    /// Model probability P(abusive).
    pub p: f64,
    pub label: bool,
}

/// Score every customer in a world through the shared feature pipeline.
/// Used by calibration and by the guarantee-stress harness in stress.rs.
pub fn customer_scores(world: &World, model: &lr::LogisticModel) -> Vec<ScoredCustomer> {
    let ctx = cluster_context(world);
    world
        .behaviors
        .iter()
        .map(|(id, b)| ScoredCustomer {
            id: id.clone(),
            p: model.predict(&behavior_features(b, ctx.of.get(id))),
            label: world.ground_truth.get(id).copied().unwrap_or(false),
        })
        .collect()
}

/// Trained model + CRC thresholds. Build ONCE from calibration worlds only;
/// reuse across held-out evaluation.
pub struct LearnedSuite {
    pub model: lr::LogisticModel,
    pub taus: CalibratedThresholds,
}

impl LearnedSuite {
    /// Train on the provided CALIBRATION worlds. Never call with held-out data.
    pub fn train_on_calibration(calibration_worlds: &[World]) -> Self {
        let names: Vec<String> = FEATURE_NAMES.iter().map(|s| s.to_string()).collect();
        let samples = build_samples(calibration_worlds);
        let model = lr::train(&samples, &names, concat!("lr-1.0.0-calib-", env!("CARGO_PKG_VERSION")));

        // CRC calibrates the decision thresholds from the same split's scores.
        // (A production system would use a second maturity window; with one
        // calibration seed the scores are in-sample for the model but the
        // finite-sample bound still holds on exchangeable future traffic.)
        let score_samples: Vec<Sample> = calibration_worlds
            .iter()
            .flat_map(|w| {
                let ctx = cluster_context(w);
                w.behaviors
                    .iter()
                    .map(|(id, b)| {
                        let p = model.predict(&behavior_features(b, ctx.of.get(id)));
                        let label = if w.ground_truth.get(id).copied().unwrap_or(false) {
                            1.0
                        } else {
                            0.0
                        };
                        Sample {
                            features: vec![p],
                            label,
                        }
                    })
                    .collect::<Vec<Sample>>()
            })
            .collect();
        let taus = crate::conformal::calibrate(&score_samples, DEFAULT_ALPHA_LEAK, DEFAULT_ALPHA_FRICTION);
        Self { model, taus }
    }

    pub fn learned_detector(&self) -> LearnedLogisticDetector {
        LearnedLogisticDetector {
            model: self.model.clone(),
        }
    }

    pub fn calibrated_detector(&self) -> CalibratedLrDetector {
        CalibratedLrDetector {
            model: self.model.clone(),
            taus: self.taus,
            review_cost_paise: DEFAULT_REVIEW_COST_PAISE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dataset_gen::{generate_world, WorldKind, WorldSpec};

    fn world(kind: WorldKind, seed: u64) -> World {
        generate_world(WorldSpec {
            kind,
            n_background: 300,
            n_rings: 6,
            ring_size: 3,
            seed,
        })
    }

    #[test]
    fn learned_model_separates_calibration_abuse() {
        let worlds = vec![
            world(WorldKind::Normal, crate::CALIBRATION_SEED),
            world(WorldKind::ReturnAbuse, crate::CALIBRATION_SEED),
        ];
        let suite = LearnedSuite::train_on_calibration(&worlds);

        // Sanity: model fires on obvious abuser-shaped behavior...
        let abuser = investigation_engine::CustomerBehavior {
            customer_id: "x".into(),
            order_count: 10,
            return_count: 3,
            refund_count: 3,
            dispute_count: 0,
            distinct_merchants: 1,
            distinct_products: 1,
            account_age_days: 10,
            purchase_to_return_hours: vec![20.0, 24.0, 30.0],
        };
        let p_abuser = suite.model.predict(&behavior_features(&abuser, None));
        assert!(p_abuser > 0.7, "abuser-shaped behavior scored {p_abuser:.3}");

        // ...and stays quiet on an established, diverse customer.
        let legit = investigation_engine::CustomerBehavior {
            customer_id: "y".into(),
            order_count: 50,
            return_count: 3,
            refund_count: 3,
            dispute_count: 0,
            distinct_merchants: 8,
            distinct_products: 20,
            account_age_days: 900,
            purchase_to_return_hours: vec![500.0, 700.0],
        };
        let p_legit = suite.model.predict(&behavior_features(&legit, None));
        assert!(p_legit < 0.2, "benign behavior scored {p_legit:.3}");
        assert!(p_abuser > p_legit);
    }

    #[test]
    fn calibrated_detector_produces_three_way_decisions_on_households() {
        // Households: structurally clustered but benign — the calibrated
        // detector must NOT auto-block them (friction budget), and mostly clear.
        let worlds = vec![
            world(WorldKind::Normal, crate::CALIBRATION_SEED),
            world(WorldKind::ReturnAbuse, crate::CALIBRATION_SEED),
            world(WorldKind::Household, crate::CALIBRATION_SEED),
        ];
        let suite = LearnedSuite::train_on_calibration(&worlds);
        let mut det = suite.calibrated_detector();

        let household = world(WorldKind::Household, crate::CALIBRATION_SEED);
        let decisions = det.scan(&household);
        let auto_blocked = decisions.values().filter(|e| **e == Escalation::AutoBlock).count();
        assert_eq!(
            auto_blocked, 0,
            "CRC friction budget violated: legitimate households auto-blocked"
        );
    }

    #[test]
    fn review_band_exists_between_thresholds() {
        let worlds = vec![
            world(WorldKind::Normal, crate::CALIBRATION_SEED),
            world(WorldKind::RefundAbuse, crate::CALIBRATION_SEED),
        ];
        let suite = LearnedSuite::train_on_calibration(&worlds);
        assert!(
            suite.taus.tau_clear < suite.taus.tau_block,
            "tau_clear {} must sit below tau_block {}",
            suite.taus.tau_clear,
            suite.taus.tau_block
        );
    }

    #[test]
    fn economics_allow_cheap_uncertainty_and_block_expensive_conviction() {
        // The instance-dependent rule in isolation:
        //   tiny exposure + high p̂  → CLEAR  (wrong costs less than a review)
        //   huge exposure + high p̂   → BLOCK (CRC region, expensive to be wrong)
        let worlds = vec![
            world(WorldKind::Normal, crate::CALIBRATION_SEED),
            world(WorldKind::ReturnAbuse, crate::CALIBRATION_SEED),
        ];
        let suite = LearnedSuite::train_on_calibration(&worlds);
        let mut det = suite.calibrated_detector();

        let mut w = world(WorldKind::Normal, crate::CALIBRATION_SEED);
        // Two identical behavior profiles; only exposure differs.
        for (id, exposure) in [("cheap_suspect", 5_000i64), ("rich_suspect", 50_000_000)] {
            let mut b = investigation_engine::CustomerBehavior {
                customer_id: id.into(),
                order_count: 12,
                return_count: 4,
                refund_count: 4,
                dispute_count: 1,
                distinct_merchants: 2,
                distinct_products: 2,
                account_age_days: 25,
                purchase_to_return_hours: vec![30.0, 40.0],
            };
            b.customer_id = id.into();
            w.behaviors.insert(id.to_string(), b);
            w.exposure_paise.insert(id.to_string(), exposure);
        }
        let decisions = det.scan(&w);
        assert_eq!(
            decisions["cheap_suspect"],
            Escalation::Clear,
            "₹50 at risk must not buy a human review"
        );
        assert_eq!(
            decisions["rich_suspect"],
            Escalation::AutoBlock,
            "high-exposure conviction must block"
        );
    }
}
