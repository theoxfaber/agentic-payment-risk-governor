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
    // Precision is pooled OVER legitimate-overlap worlds: every flag there
    // is a false positive and must count against the headline.
    let (precision, recall, worlds, legit_flagged) = eval_harness::robustness::run_randomized_sweep(6, 987_654);
    assert!(worlds >= 30, "sweep too small to mean anything: {worlds}");
    assert!(recall >= 0.95, "randomized-world recall {recall:.1}% < 95%");
    assert!(
        precision >= 0.95,
        "randomized-world precision (incl. legit FPs) {precision:.1}% < 95%"
    );
    assert_eq!(legit_flagged, 0, "randomized sweep flagged innocent customers");
}

#[test]
fn crc_budgets_hold_under_camouflage_and_drift_is_visible() {
    // Cheap, honest validation: camouflage collapses separability so the CRC
    // machinery is FORCED to spend its budgets — then we check it stays
    // inside them. This is the only test that FAILS if the quantile math is
    // wrong by one (the off-by-one shows up as z ≫ 2). Kept small (6 runs)
    // so CI stays fast; the full 12-run table ships via the harness binary.
    let suite = eval_harness::learned_suite();
    let runs = eval_harness::stress::run_crc_stress(6, &suite);
    let v = eval_harness::stress::summarize(
        &runs,
        eval_harness::conformal::DEFAULT_ALPHA_LEAK,
        eval_harness::conformal::DEFAULT_ALPHA_FRICTION,
    );
    assert!(
        v.leak_budget_holds,
        "leak budget violated: mean {:.2}% vs {:.0}% budget, z={:.2} over {} customers (failed runs leaked {}/{})",
        v.mean_leak_rate * 100.0,
        v.alpha_leak * 100.0,
        v.leak_z,
        v.n_tested,
        v.total_leaked,
        v.total_leaked
    );
    assert!(
        v.friction_budget_holds,
        "friction budget violated: mean {:.3}% vs {:.0}% budget, z={:.2}",
        v.mean_friction_rate * 100.0,
        v.alpha_friction * 100.0,
        v.friction_z
    );
    assert!(
        v.mean_psi_camouflage > 0.20,
        "camouflage must drift score histogram (PSI {:.3})",
        v.mean_psi_camouflage
    );
    assert!(
        v.mean_review_share > 0.15,
        "overlap should route uncertainty to review (review share {:.1}%)",
        v.mean_review_share * 100.0
    );
    for r in &runs {
        assert!(
            r.tau_clear < 0.15,
            "run {} tau_clear {:.4} should collapse from clean ~0.23 under overlap",
            r.run,
            r.tau_clear
        );
    }
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
