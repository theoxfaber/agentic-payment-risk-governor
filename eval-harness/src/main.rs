//! Prints the honest comparison table. Run: cargo run -p eval-harness
//! Output feeds the README verbatim.
//!
//! Protocol: thresholds were tuned on CALIBRATION worlds (seed 2026) only.
//! Headline numbers come from HELD-OUT worlds (three unseen seeds), which is
//! the only evaluation that predicts anything.

use eval_harness::DETECTORS;

fn aggregate(rows: &[&eval_harness::WorldMetrics], label: &str) {
    if rows.is_empty() {
        return;
    }
    let tp: u32 = rows.iter().map(|m| m.tp).sum();
    let fp: u32 = rows.iter().map(|m| m.fp).sum();
    let fnn: u32 = rows.iter().map(|m| m.fn_count).sum();
    let fp_cost: i64 = rows.iter().map(|m| m.fp_cost_paise).sum();
    let fn_cost: i64 = rows.iter().map(|m| m.fn_cost_paise).sum();
    let prevented: i64 = rows.iter().map(|m| m.prevented_paise).sum();
    let precision = if tp + fp == 0 {
        1.0
    } else {
        tp as f64 / (tp + fp) as f64
    };
    let recall = if tp + fnn == 0 {
        1.0
    } else {
        tp as f64 / (tp + fnn) as f64
    };
    println!(
        "{label}: precision={:.0}% recall={:.0}% | FP cost ₹{:.0} | FN cost ₹{:.0} | prevented ₹{:.0}",
        precision * 100.0,
        recall * 100.0,
        fp_cost as f64 / 100.0,
        fn_cost as f64 / 100.0,
        prevented as f64 / 100.0
    );
}

fn main() {
    let results = eval_harness::run_all();

    println!(
        "=== CALIBRATION worlds (seed {} — what thresholds were tuned on; NOT headline) ===",
        eval_harness::CALIBRATION_SEED
    );
    let calib: Vec<_> = results.iter().filter(|m| m.split == "calibration").collect();
    println!("{}", eval_harness::render_markdown(&calib));

    println!(
        "\n=== HELD-OUT worlds (seeds {:?} — detector never saw these) ===",
        eval_harness::HELDOUT_SEEDS
    );
    let heldout: Vec<_> = results.iter().filter(|m| m.split == "held-out").collect();
    println!("{}", eval_harness::render_markdown(&heldout));

    // Aggregate per detector across held-out abuse worlds ONLY — this is the
    // honest headline (Normal/Household have no abusers, excluded from recall).
    println!("\n=== HEADLINE: aggregate across HELD-OUT abuse worlds ===");
    for det in DETECTORS {
        let rows: Vec<_> = heldout
            .iter()
            .copied()
            .filter(|m| {
                m.detector == *det
                    && (m.world.contains("abuse")
                        || m.world.contains("ring")
                        || m.world.contains("collusion")
                        || m.world.contains("evasion"))
            })
            .collect();
        aggregate(&rows, det);
    }

    println!("\n=== Household FP check across ALL held-out seeds (all customers legitimate) ===");
    for det in DETECTORS {
        let rows: Vec<_> = heldout
            .iter()
            .copied()
            .filter(|m| m.detector == *det && m.world.contains("household"))
            .collect();
        let fp: u32 = rows.iter().map(|m| m.fp).sum();
        let customers: u32 = rows.iter().map(|m| m.tp + m.fp + m.tn + m.fn_count).sum();
        let friction: i64 = rows.iter().map(|m| m.fp_cost_paise).sum();
        println!(
            "{det}: FP={fp} of {customers} customers across {} seeds (₹{:.0} friction cost)",
            rows.len(),
            friction as f64 / 100.0
        );
    }
}
