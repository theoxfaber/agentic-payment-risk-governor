//! Conformal Risk Control for the ALLOW / REVIEW / BLOCK thresholds.
//!
//! The REVIEW band stops being a magic number. Following split-conformal
//! prediction (Vovk et al. 2005; Papadopoulos et al. 2002) and Conformal
//! Risk Control (Angelopoulos, Bates et al., ICLR 2024, arXiv:2208.02814),
//! we pick the two thresholds on a CALIBRATION split so that, on future
//! exchangeable traffic:
//!
//!   • E[fraudsters silently cleared]        ≤ α_leak    (fraud-leak budget)
//!   • E[legitimate customers auto-blocked]  ≤ α_friction (friction budget)
//!
//! with finite-sample, distribution-free validity — the only assumption is
//! exchangeability between calibration and serving data. Everything between
//! the two thresholds routes to HUMAN REVIEW: uncertainty is escalated, not
//! guessed (the same safety principle as the combiner, now with statistics
//! behind it).
//!
//! Construction (monotone-loss CRC, binary case):
//!   scores s_i = p̂(x_i) ∈ [0,1], labels y_i ∈ {0,1}, calibration size n.
//!   • τ_clear = largest score with  #{y=1, s ≤ τ}      ≤ α_leak · (n+1) − 1
//!     ⇒ at most ⌊α_leak(n+1)⌋ − ... positives slip through auto-clear.
//!   • τ_block = smallest score with #{y=0, s ≥ τ}      ≤ α_friction · (n+1)
//!   Between them → REVIEW. If a budget admits zero auto-decisions on this
//!   calibration set, that side collapses to REVIEW (fail-safe direction).

use crate::lr::Sample;
use serde::Serialize;

/// Calibrated operating point.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct CalibratedThresholds {
    /// p̂ below this → CLEAR (auto-allow), bounded fraud-leak rate.
    pub tau_clear: f64,
    /// p̂ above this → AUTO-BLOCK, bounded friction rate.
    pub tau_block: f64,
    /// Budgets actually used (recorded into artifacts/audit).
    pub alpha_leak: f64,
    pub alpha_friction: f64,
    pub calibration_samples: usize,
}

/// Budgets: how much fraud may leak through auto-allow, and how much
/// legitimate traffic may be auto-blocked. Both are POLICY choices a merchant
/// makes explicitly — the whole point of conformal calibration is that these
/// are the only two numbers anyone has to defend.
pub const DEFAULT_ALPHA_LEAK: f64 = 0.02;
pub const DEFAULT_ALPHA_FRICTION: f64 = 0.01;

/// Calibrate (tau_clear, tau_block) from labeled calibration scores.
///
/// `samples` must come ONLY from worlds/thresholds the detector was developed
/// against — held-out data is never touched here.
pub fn calibrate(samples: &[Sample], alpha_leak: f64, alpha_friction: f64) -> CalibratedThresholds {
    let n = samples.len();
    assert!(n > 0, "calibration requires samples");

    let mut pos_scores: Vec<f64> = samples
        .iter()
        .filter(|s| s.label > 0.5)
        .map(|s| s.features[0])
        .collect();
    let mut neg_scores: Vec<f64> = samples
        .iter()
        .filter(|s| s.label <= 0.5)
        .map(|s| s.features[0])
        .collect();
    pos_scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    neg_scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // --- tau_clear: allow at most floor(alpha_leak*(n+1)) positives to sit
    // at or below it. q counts PERMITTED leaks under the (n+1) correction;
    // if the budget cannot even permit one leak, nothing auto-clears.
    let q = ((alpha_leak * (n as f64 + 1.0)).floor() as usize).min(pos_scores.len());
    let tau_clear = if q == 0 {
        -0.0_f64 // no auto-clear is statistically defensible → everything reviews/blocks
    } else {
        pos_scores[q - 1]
    };

    // --- tau_block: block only where fewer than floor(alpha_friction*(n+1))
    // negatives would be swept up. r = permitted friction count.
    let r = ((alpha_friction * (n as f64 + 1.0)).floor() as usize).min(neg_scores.len());
    let tau_block = if r == 0 {
        1.0 + f64::EPSILON // no auto-block is defensible → nothing auto-blocks
    } else {
        neg_scores[neg_scores.len() - r]
    };

    // Sanity: the band must not invert (clear threshold above block).
    let (tau_clear, tau_block) = if tau_clear > tau_block {
        (tau_block, tau_clear)
    } else {
        (tau_clear, tau_block)
    };

    CalibratedThresholds {
        tau_clear,
        tau_block,
        alpha_leak,
        alpha_friction,
        calibration_samples: n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(score: f64, label: f64) -> Sample {
        Sample {
            features: vec![score],
            label,
        }
    }

    #[test]
    fn tight_separation_yields_wide_review_band_edges() {
        // Perfect separation: negatives at 0.1, positives at 0.9.
        let mut samples = Vec::new();
        for i in 0..50 {
            samples.push(sample(0.10 + i as f64 * 1e-4, 0.0));
            samples.push(sample(0.90 - i as f64 * 1e-4, 1.0));
        }
        let t = calibrate(&samples, DEFAULT_ALPHA_LEAK, DEFAULT_ALPHA_FRICTION);
        assert!(t.tau_clear < 0.5 && t.tau_block > 0.5, "taus: {:?}", t);
    }

    #[test]
    fn zero_leak_budget_collapses_auto_clear() {
        // alpha_leak so small that floor(alpha*(n+1)) == 0 → nothing clears.
        let mut samples = Vec::new();
        for i in 0..100 {
            samples.push(sample(
                if i % 2 == 0 { 0.2 } else { 0.8 },
                if i % 2 == 0 { 0.0 } else { 1.0 },
            ));
        }
        let t = calibrate(&samples, 0.0, DEFAULT_ALPHA_FRICTION);
        assert!(
            t.tau_clear <= 0.0,
            "zero budget must disable auto-clear, got {}",
            t.tau_clear
        );
    }

    #[test]
    fn empirical_leak_rate_respects_budget_on_calibration_data() {
        // With overlap between classes, verify the guarantee empirically:
        // #positives cleared by tau_clear must satisfy the CRC bound.
        let mut samples = Vec::new();
        let mut rng_state = 12345u64;
        let mut next = move || {
            rng_state = rng_state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (rng_state >> 33) as f64 / (u32::MAX as f64)
        };
        for _ in 0..500 {
            let y = if next() < 0.3 { 1.0 } else { 0.0 };
            let base = if y > 0.5 {
                0.55 + next() * 0.35
            } else {
                0.25 + next() * 0.45
            };
            samples.push(sample(base.min(1.0), y));
        }
        let t = calibrate(&samples, DEFAULT_ALPHA_LEAK, DEFAULT_ALPHA_FRICTION);
        let allowed_leaks = samples
            .iter()
            .filter(|s| s.label > 0.5 && s.features[0] <= t.tau_clear)
            .count();
        let bound = (DEFAULT_ALPHA_LEAK * (samples.len() as f64 + 1.0)).floor() as usize;
        assert!(
            allowed_leaks <= bound.max(1),
            "leaks {allowed_leaks} exceed CRC bound {bound} (tau_clear={})",
            t.tau_clear
        );

        let blocked_legits = samples
            .iter()
            .filter(|s| s.label <= 0.5 && s.features[0] >= t.tau_block)
            .count();
        let bound_b = (DEFAULT_ALPHA_FRICTION * (samples.len() as f64 + 1.0)).floor() as usize;
        assert!(
            blocked_legits <= bound_b.max(1),
            "friction {blocked_legits} exceeds CRC bound {bound_b} (tau_block={})",
            t.tau_block
        );
    }

    #[test]
    fn band_never_inverts() {
        // Fully overlapping classes: clear and block must stay ordered.
        let mut samples = Vec::new();
        for i in 0..80 {
            samples.push(sample(0.3 + (i % 40) as f64 * 0.01, if i < 40 { 0.0 } else { 1.0 }));
        }
        let t = calibrate(&samples, DEFAULT_ALPHA_LEAK, DEFAULT_ALPHA_FRICTION);
        assert!(t.tau_clear <= t.tau_block);
    }
}
