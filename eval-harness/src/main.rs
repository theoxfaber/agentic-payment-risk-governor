//! Prints the honest comparison table. Run: cargo run -p eval-harness
//! Output feeds the README verbatim.

fn main() {
    let results = eval_harness::run_all();
    println!("{}", eval_harness::render_markdown(&results));

    // Aggregate per detector across abuse worlds (exclude Normal/Household
    // from recall aggregates; they have no abusers)
    println!("\n=== Aggregate across abuse worlds (C/D/E/F/G) ===");
    for det in ["per_customer_rate_rule", "structural_cluster_only", "investigation_engine"] {
        let rows: Vec<_> = results
            .iter()
            .filter(|m| m.detector == det && m.world.contains("abuse") || m.detector == det && (m.world.contains("ring") || m.world.contains("collusion") || m.world.contains("evasion")))
            .collect();
        if rows.is_empty() {
            continue;
        }
        let tp: u32 = rows.iter().map(|m| m.tp).sum();
        let fp: u32 = rows.iter().map(|m| m.fp).sum();
        let fnn: u32 = rows.iter().map(|m| m.fn_count).sum();
        let fp_cost: i64 = rows.iter().map(|m| m.fp_cost_paise).sum();
        let fn_cost: i64 = rows.iter().map(|m| m.fn_cost_paise).sum();
        let prevented: i64 = rows.iter().map(|m| m.prevented_paise).sum();
        let precision = if tp + fp == 0 { 1.0 } else { tp as f64 / (tp + fp) as f64 };
        let recall = if tp + fnn == 0 { 1.0 } else { tp as f64 / (tp + fnn) as f64 };
        println!(
            "{det}: precision={:.0}% recall={:.0}% | FP cost ₹{:.0} | FN cost ₹{:.0} | prevented ₹{:.0}",
            precision * 100.0, recall * 100.0,
            fp_cost as f64 / 100.0, fn_cost as f64 / 100.0, prevented as f64 / 100.0
        );
    }

    println!("\n=== Household FP check (World B — all customers legitimate) ===");
    for det in ["per_customer_rate_rule", "structural_cluster_only", "investigation_engine"] {
        if let Some(m) = results.iter().find(|m| m.detector == det && m.world.contains("household")) {
            println!("{det}: FP={} of {} customers (₹{:.0} friction cost)", m.fp, m.tp + m.fp + m.tn + m.fn_count, m.fp_cost_paise as f64 / 100.0);
        }
    }
}
