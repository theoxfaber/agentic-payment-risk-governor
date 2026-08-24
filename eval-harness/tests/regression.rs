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
