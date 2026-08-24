//! Regression guards for the two hardest worlds:
//!   - AdversarialEvasion: structural linkage must escalate even when
//!     behavioral signals are defeated (recall >= 90%)
//!   - Household: counter-evidence must exonerate (FP == 0)

use dataset_gen::{generate_world, WorldKind, WorldSpec};
use eval_harness::{evaluate, InvestigationEngineDetector, PerCustomerRateRule, StructuralClusterOnly};

fn evasion_world() -> dataset_gen::World {
    generate_world(WorldSpec {
        kind: WorldKind::AdversarialEvasion,
        n_background: 300,
        n_rings: 6,
        ring_size: 3,
        seed: 2026,
    })
}

fn household_world() -> dataset_gen::World {
    generate_world(WorldSpec {
        kind: WorldKind::Household,
        n_background: 300,
        n_rings: 8,
        ring_size: 3,
        seed: 2026,
    })
}

/// Under adversarial evasion the investigator loses behavioral confirmation —
/// that must route to HUMAN REVIEW, never to a silent clear.
#[test]
fn evasion_recall_at_least_90_percent() {
    let mut det = InvestigationEngineDetector;
    let m = evaluate(&evasion_world(), &mut det);

    assert!(
        m.recall >= 0.90,
        "evasion recall {}% < 90% — structurally-linked abusers were cleared",
        m.recall * 100.0
    );
    // The escalation path should be predominantly human review, not auto-block:
    // we KNOW behavior is inconclusive there.
    assert!(
        m.human_reviewed > 0 || m.auto_blocked > 0,
        "no escalation recorded at all"
    );
}

/// Households are the FP trap: shared devices + addresses but genuinely
/// legitimate. Counter-evidence must exonerate them completely.
#[test]
fn household_false_positives_stay_zero() {
    let mut det = InvestigationEngineDetector;
    let m = evaluate(&household_world(), &mut det);

    assert_eq!(m.fp, 0, "investigation engine flagged innocent household members");
}

/// Sanity: the naive rule still misses most evasion abuse (this is WHY the
/// graph exists) and structural-only still torches households (this is why
/// investigation exists).
#[test]
fn detector_contrast_is_preserved() {
    let w = household_world();
    let mut rules = PerCustomerRateRule;
    let mut structural = StructuralClusterOnly::default();
    let mut inv = InvestigationEngineDetector;

    let r = evaluate(&w, &mut rules);
    let s = evaluate(&w, &mut structural);
    let i = evaluate(&w, &mut inv);

    assert_eq!(i.fp, 0);
    assert!(
        s.fp > i.fp,
        "structural-only ({}) must be worse than investigation ({}) on households",
        s.fp,
        i.fp
    );
    let _ = r;

    let w2 = evasion_world();
    let mut rules2 = PerCustomerRateRule;
    let mut inv2 = InvestigationEngineDetector;
    let rr = evaluate(&w2, &mut rules2);
    let ii = evaluate(&w2, &mut inv2);
    assert!(
        ii.recall > rr.recall,
        "investigation recall {}% must beat per-customer rules {}% on evasion",
        ii.recall * 100.0,
        rr.recall * 100.0
    );
}

// ---------------------------------------------------------------------------
// HELD-OUT seeds — the detector never saw these worlds during development.
// These guards make the README's headline numbers a claim about generalization,
// not memorization.
// ---------------------------------------------------------------------------

fn world_at(kind: WorldKind, rings: usize, seed: u64) -> dataset_gen::World {
    generate_world(WorldSpec {
        kind,
        n_background: 300,
        n_rings: rings,
        ring_size: 3,
        seed,
    })
}

#[test]
fn held_out_evasion_recall_holds_on_every_unseen_seed() {
    for seed in eval_harness::HELDOUT_SEEDS {
        let mut det = InvestigationEngineDetector;
        let m = evaluate(&world_at(WorldKind::AdversarialEvasion, 6, *seed), &mut det);
        assert!(
            m.recall >= 0.90,
            "seed {seed}: evasion recall {}% < 90% — structurally-linked abusers were cleared",
            m.recall * 100.0
        );
    }
}

#[test]
fn held_out_household_false_positives_stay_zero_on_every_unseen_seed() {
    for seed in eval_harness::HELDOUT_SEEDS {
        let mut det = InvestigationEngineDetector;
        let m = evaluate(&world_at(WorldKind::Household, 8, *seed), &mut det);
        assert_eq!(m.fp, 0, "seed {seed}: innocent household members flagged");
    }
}

#[test]
fn held_out_coincidental_sharing_precision_stays_perfect() {
    // NAT-style overlap is all legitimate: flagging ANY of it burns real money.
    for seed in eval_harness::HELDOUT_SEEDS {
        let mut det = InvestigationEngineDetector;
        let m = evaluate(&world_at(WorldKind::CoincidentalSharing, 8, *seed), &mut det);
        assert_eq!(
            m.fp, 0,
            "seed {seed}: coincidental-sharing customers flagged — FP precision broken"
        );
    }
}

// ---------------------------------------------------------------------------
// ROBUSTNESS GATES — performance under messy data, not just clean sweeps.
// Real evidence pipelines lose records, drift timestamps, and miscount.
// The safety property being pinned: degradation routes uncertainty to HUMANS
// (review share rises), never to silent clears (recall holds).
// ---------------------------------------------------------------------------

#[test]
fn degradation_recall_holds_and_uncertainty_routes_to_humans() {
    // One seed keeps CI fast while still covering every mess level x every
    // world kind; the full 3-seed sweep runs in `cargo run -p eval-harness`.
    let rows = eval_harness::robustness::run_degradation_sweep(&eval_harness::HELDOUT_SEEDS[0..1]);
    assert_eq!(rows.len(), 3, "clean/mild/heavy");

    for r in &rows {
        assert!(
            r.recall >= 0.90,
            "mess level {}: pooled recall {}% < 90% — abusers were silently cleared",
            r.level,
            r.recall * 100.0
        );
    }

    // Review share must RISE with degradation: falling confidence escalates
    // to humans instead of guessing.
    let (clean, mild, heavy) = (&rows[0], &rows[1], &rows[2]);
    assert!(
        heavy.human_review_share > clean.human_review_share,
        "human-review share {}% did not rise from clean to heavy",
        heavy.human_review_share * 100.0
    );
    assert!(
        mild.human_review_share > clean.human_review_share,
        "even mild degradation must shift decisions toward human review"
    );
}

#[test]
fn randomized_world_shapes_recall_stays_above_95_percent() {
    // Reduced draws for CI runtime; the full 140-world sweep runs via
    // `cargo run --release -p eval-harness`.
    let (precision, recall, worlds) = eval_harness::robustness::run_randomized_sweep(6, 987_654);
    assert!(worlds >= 30, "sweep too small to mean anything: {worlds}");
    assert!(recall >= 0.95, "randomized-world recall {recall:.1}% < 95%");
    assert!(precision >= 0.95, "randomized-world precision {precision:.1}% < 95%");
}

#[test]
fn degrade_world_actually_degrades() {
    // Guard against a vacuous harness: perturbation must visibly change the data.
    use dataset_gen::{generate_world, WorldKind, WorldSpec};
    use eval_harness::robustness::{degrade_world, MessLevel};

    let w = generate_world(WorldSpec {
        kind: WorldKind::ReturnAbuse,
        n_background: 300,
        n_rings: 6,
        ring_size: 3,
        seed: 31_415,
    });
    let degraded = degrade_world(&w, MessLevel::Heavy, 42);
    assert!(
        degraded.behaviors.len() < w.behaviors.len(),
        "heavy mess must drop behavioral records"
    );
    let jittered_differs = w.behaviors.iter().any(|(id, b)| {
        degraded
            .behaviors
            .get(id)
            .map(|d| d.purchase_to_return_hours != b.purchase_to_return_hours)
            .unwrap_or(false)
    });
    assert!(jittered_differs, "heavy mess must jitter timing");
}
