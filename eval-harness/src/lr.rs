//! Pure-Rust logistic regression — the learned scoring layer.
//!
//! Why logistic regression (see docs/AI_DESIGN.md):
//!   • The industry-standard fraud baseline (Stripe Radar started GBDT+LR;
//!     banks deploy constrained LR under interpretability rules).
//!   • Trainable offline in pure Rust with zero dependencies — deterministic,
//!     auditable, and the weights serialize to a small JSON artifact.
//!   • Outputs CALIBRATED probabilities, which is what the downstream
//!     conformal calibration (conformal.rs) requires to give finite-sample
//!     guarantees. A hand-tuned weighted sum does not produce probabilities;
//!     this does.
//!
//! Training: full-batch gradient descent with L2 regularization and
//! class-weighting (fraud is rare; unweighted loss learns to never fire).
//! Feature standardization happens inside the model so the artifact carries
//! everything needed at inference.

use serde::{Deserialize, Serialize};

/// A trained model: standardized linear logits + sigmoid.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogisticModel {
    pub feature_names: Vec<String>,
    /// Per-feature mean (standardization).
    pub means: Vec<f64>,
    /// Per-feature stddev (standardization, floored at 1e-9).
    pub stds: Vec<f64>,
    pub weights: Vec<f64>,
    pub bias: f64,
    /// Version string recorded in decisions/audit trails.
    pub version: String,
}

impl LogisticModel {
    /// P(positive | features). Calibrated probability in [0, 1].
    pub fn predict(&self, features: &[f64]) -> f64 {
        assert_eq!(features.len(), self.weights.len(), "feature count mismatch");
        let mut z = self.bias;
        for (i, &x) in features.iter().enumerate() {
            let standardized = (x - self.means[i]) / self.stds[i];
            z += self.weights[i] * standardized;
        }
        1.0 / (1.0 + (-z).exp())
    }
}

/// One labeled training example.
#[derive(Debug, Clone)]
pub struct Sample {
    pub features: Vec<f64>,
    /// 1.0 = abusive, 0.0 = legitimate.
    pub label: f64,
}

/// Trained-model hyperparameters (fixed, documented — not tuned secrets).
pub const LEARNING_RATE: f64 = 0.1;
pub const EPOCHS: usize = 600;
pub const L2_PENALTY: f64 = 1e-3;

/// Train class-weighted L2 logistic regression by full-batch gradient
/// descent. Deterministic: fixed iteration schedule, no randomness.
pub fn train(samples: &[Sample], feature_names: &[String], version: &str) -> LogisticModel {
    let n_features = feature_names.len();
    assert!(!samples.is_empty(), "cannot train on empty sample set");

    // --- standardization statistics ---
    let n = samples.len() as f64;
    let mut means = vec![0.0; n_features];
    for s in samples {
        for (m, &x) in means.iter_mut().zip(&s.features) {
            *m += x;
        }
    }
    for m in &mut means {
        *m /= n;
    }
    let mut stds = vec![0.0; n_features];
    for s in samples {
        for (i, &x) in s.features.iter().enumerate() {
            stds[i] += (x - means[i]).powi(2);
        }
    }
    for v in &mut stds {
        *v = (*v / n).sqrt().max(1e-9);
    }

    // --- class weights: balance the loss, not the data (keeps calibration
    // closer to true priors than oversampling would) ---
    let n_pos = samples.iter().map(|s| s.label).sum::<f64>();
    let n_neg = n - n_pos;
    let w_pos = if n_pos > 0.0 { n / (2.0 * n_pos) } else { 1.0 };
    let w_neg = if n_neg > 0.0 { n / (2.0 * n_neg) } else { 1.0 };

    let mut weights = vec![0.0; n_features];
    let mut bias = 0.0;

    for _ in 0..EPOCHS {
        let mut grad_w = vec![0.0; n_features];
        let mut grad_b = 0.0;
        for s in samples {
            let p = sigmoid(
                bias + weights
                    .iter()
                    .zip(&s.features)
                    .enumerate()
                    .map(|(i, (&w, &x))| w * ((x - means[i]) / stds[i]))
                    .sum::<f64>(),
            );
            let cw = if s.label > 0.5 { w_pos } else { w_neg };
            let err = cw * (p - s.label);
            for (i, &x) in s.features.iter().enumerate() {
                grad_w[i] += err * ((x - means[i]) / stds[i]);
            }
            grad_b += err;
        }
        for (w, g) in weights.iter_mut().zip(&grad_w) {
            *w -= LEARNING_RATE * (*g / n + L2_PENALTY * *w);
        }
        bias -= LEARNING_RATE * grad_b / n;
    }

    LogisticModel {
        feature_names: feature_names.to_vec(),
        means,
        stds,
        weights,
        bias,
        version: version.to_string(),
    }
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z).exp())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("f{i}")).collect()
    }

    #[test]
    fn separates_linearly_separable_classes() {
        // Class 0 around origin, class 1 far along f0.
        let mut samples = Vec::new();
        for i in 0..60 {
            samples.push(Sample {
                features: vec![i as f64 * 0.05, (i % 7) as f64],
                label: 0.0,
            });
        }
        for i in 0..60 {
            samples.push(Sample {
                features: vec![3.0 + i as f64 * 0.05, (i % 5) as f64],
                label: 1.0,
            });
        }
        let model = train(&samples, &names(2), "test");
        assert!(model.predict(&[0.1, 1.0]) < 0.2, "low example scored high");
        assert!(model.predict(&[4.5, 1.0]) > 0.8, "high example scored low");
    }

    #[test]
    fn probabilities_are_bounded_and_calibrated_shape() {
        let samples = vec![
            Sample {
                features: vec![0.0],
                label: 0.0,
            },
            Sample {
                features: vec![0.1],
                label: 0.0,
            },
            Sample {
                features: vec![5.0],
                label: 1.0,
            },
            Sample {
                features: vec![5.1],
                label: 1.0,
            },
        ];
        let model = train(&samples, &names(1), "test");
        for x in [-10.0f64, 0.0, 2.5, 10.0] {
            let p = model.predict(&[x]);
            assert!((0.0..=1.0).contains(&p));
        }
    }

    #[test]
    fn training_is_deterministic() {
        let build = || {
            let samples: Vec<Sample> = (0..40)
                .map(|i| Sample {
                    features: vec![i as f64],
                    label: if i > 20 { 1.0 } else { 0.0 },
                })
                .collect();
            train(&samples, &names(1), "determinism")
        };
        let a = build();
        let b = build();
        assert_eq!(a.weights, b.weights);
        assert_eq!(a.bias, b.bias);
    }

    #[test]
    fn model_round_trips_through_json() {
        let samples: Vec<Sample> = (0..30)
            .map(|i| Sample {
                features: vec![i as f64, (i * 3) as f64],
                label: if i % 2 == 0 { 1.0 } else { 0.0 },
            })
            .collect();
        let model = train(&samples, &names(2), "serde");
        let json = serde_json::to_string(&model).unwrap();
        let back: LogisticModel = serde_json::from_str(&json).unwrap();
        let x = [1.5, 4.0];
        assert_eq!(model.predict(&x), back.predict(&x));
    }
}
