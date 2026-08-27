    Finished `release` profile [optimized] target(s) in 0.74s
     Running `target/release/eval-harness`
=== HELD-OUT SEEDS: committed defaults [31415, 27182, 16180] (override with EVAL_HELDOUT_SEEDS=a,b,c) ===
=== LEARNED LAYER (trained on calibration seed 2026 ONLY) ===
model: lr-1.0.0-calib-0.1.0 | features: return_refund_rate, log_account_age_days, distinct_merchants_norm, distinct_products_norm, dispute_ratio, sync_share_72h, cluster_size_norm, cluster_pooled_return_rate
CRC budgets: fraud-leak ≤ 2.0%, friction ≤ 1.0% → tau_clear=0.2345, tau_block=1.0000

artifact written: /Users/apple/risk-governor/eval-harness/artifacts/lr_model.json
=== CALIBRATION worlds (seed 2026 — what thresholds were tuned on; NOT headline) ===
| Split | Seed | World | Detector | Precision | Recall | F1 | FP cost | FN cost | Prevented | Auto | Review |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| calibration | 2026 | normal_bg300_r0x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | normal_bg300_r0x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | normal_bg300_r0x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | normal_bg300_r0x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | normal_bg300_r0x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | household_bg300_r8x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | household_bg300_r8x3 | structural_cluster_only | 0% | 100% | 0.00 | ₹5625 | ₹0 | ₹0 | 24 | 0 |
| calibration | 2026 | household_bg300_r8x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | household_bg300_r8x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | household_bg300_r8x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | coincidentalsharing_bg300_r8x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | coincidentalsharing_bg300_r8x3 | structural_cluster_only | 0% | 100% | 0.00 | ₹22950 | ₹0 | ₹0 | 88 | 0 |
| calibration | 2026 | coincidentalsharing_bg300_r8x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | coincidentalsharing_bg300_r8x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | coincidentalsharing_bg300_r8x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| calibration | 2026 | returnabuse_bg300_r6x3 | per_customer_rate_rule | 100% | 61% | 0.76 | ₹0 | ₹1800 | ₹9450 | 11 | 0 |
| calibration | 2026 | returnabuse_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11250 | 18 | 0 |
| calibration | 2026 | returnabuse_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11250 | 18 | 0 |
| calibration | 2026 | returnabuse_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11250 | 18 | 0 |
| calibration | 2026 | returnabuse_bg300_r6x3 | calibrated_lr_crc | 100% | 83% | 0.91 | ₹0 | ₹0 | ₹11250 | 14 | 1 |
| calibration | 2026 | refundabuse_bg300_r6x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹18450 | 18 | 0 |
| calibration | 2026 | refundabuse_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹18450 | 18 | 0 |
| calibration | 2026 | refundabuse_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹18450 | 18 | 0 |
| calibration | 2026 | refundabuse_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹18450 | 18 | 0 |
| calibration | 2026 | refundabuse_bg300_r6x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹18450 | 1 | 17 |
| calibration | 2026 | distributedring_bg300_r6x3 | per_customer_rate_rule | 100% | 44% | 0.62 | ₹0 | ₹3150 | ₹5400 | 8 | 0 |
| calibration | 2026 | distributedring_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹8550 | 18 | 0 |
| calibration | 2026 | distributedring_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹8550 | 18 | 0 |
| calibration | 2026 | distributedring_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹8550 | 18 | 0 |
| calibration | 2026 | distributedring_bg300_r6x3 | calibrated_lr_crc | 100% | 83% | 0.91 | ₹0 | ₹0 | ₹8550 | 8 | 7 |
| calibration | 2026 | merchantcollusion_bg300_r6x3 | per_customer_rate_rule | 100% | 61% | 0.76 | ₹0 | ₹1800 | ₹9450 | 11 | 0 |
| calibration | 2026 | merchantcollusion_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11250 | 18 | 0 |
| calibration | 2026 | merchantcollusion_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11250 | 18 | 0 |
| calibration | 2026 | merchantcollusion_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11250 | 18 | 0 |
| calibration | 2026 | merchantcollusion_bg300_r6x3 | calibrated_lr_crc | 100% | 83% | 0.91 | ₹0 | ₹0 | ₹11250 | 14 | 1 |
| calibration | 2026 | adversarialevasion_bg300_r6x3 | per_customer_rate_rule | 100% | 28% | 0.43 | ₹0 | ₹4500 | ₹4500 | 5 | 0 |
| calibration | 2026 | adversarialevasion_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹9000 | 18 | 0 |
| calibration | 2026 | adversarialevasion_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹9000 | 12 | 6 |
| calibration | 2026 | adversarialevasion_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹9000 | 18 | 0 |
| calibration | 2026 | adversarialevasion_bg300_r6x3 | calibrated_lr_crc | 100% | 72% | 0.84 | ₹0 | ₹900 | ₹8100 | 0 | 13 |


=== HELD-OUT worlds (seeds [31415, 27182, 16180] — detector never saw these) ===
| Split | Seed | World | Detector | Precision | Recall | F1 | FP cost | FN cost | Prevented | Auto | Review |
|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| held-out | 31415 | normal_bg300_r0x3 | per_customer_rate_rule | 0% | 100% | 0.00 | ₹788 | ₹0 | ₹0 | 2 | 0 |
| held-out | 31415 | normal_bg300_r0x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 31415 | normal_bg300_r0x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 31415 | normal_bg300_r0x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 31415 | normal_bg300_r0x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 31415 | household_bg300_r8x3 | per_customer_rate_rule | 0% | 100% | 0.00 | ₹1575 | ₹0 | ₹0 | 3 | 0 |
| held-out | 31415 | household_bg300_r8x3 | structural_cluster_only | 0% | 100% | 0.00 | ₹4612 | ₹0 | ₹0 | 24 | 0 |
| held-out | 31415 | household_bg300_r8x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 31415 | household_bg300_r8x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 31415 | household_bg300_r8x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 31415 | coincidentalsharing_bg300_r8x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 31415 | coincidentalsharing_bg300_r8x3 | structural_cluster_only | 0% | 100% | 0.00 | ₹23962 | ₹0 | ₹0 | 88 | 0 |
| held-out | 31415 | coincidentalsharing_bg300_r8x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 31415 | coincidentalsharing_bg300_r8x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 31415 | coincidentalsharing_bg300_r8x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 31415 | returnabuse_bg300_r6x3 | per_customer_rate_rule | 100% | 61% | 0.76 | ₹0 | ₹2700 | ₹9000 | 11 | 0 |
| held-out | 31415 | returnabuse_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11700 | 18 | 0 |
| held-out | 31415 | returnabuse_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11700 | 18 | 0 |
| held-out | 31415 | returnabuse_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11700 | 18 | 0 |
| held-out | 31415 | returnabuse_bg300_r6x3 | calibrated_lr_crc | 100% | 94% | 0.97 | ₹0 | ₹0 | ₹11700 | 14 | 3 |
| held-out | 31415 | refundabuse_bg300_r6x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹16650 | 18 | 0 |
| held-out | 31415 | refundabuse_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹16650 | 18 | 0 |
| held-out | 31415 | refundabuse_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹16650 | 18 | 0 |
| held-out | 31415 | refundabuse_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹16650 | 18 | 0 |
| held-out | 31415 | refundabuse_bg300_r6x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹16650 | 0 | 18 |
| held-out | 31415 | distributedring_bg300_r6x3 | per_customer_rate_rule | 100% | 61% | 0.76 | ₹0 | ₹2700 | ₹9000 | 11 | 0 |
| held-out | 31415 | distributedring_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11700 | 18 | 0 |
| held-out | 31415 | distributedring_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11700 | 18 | 0 |
| held-out | 31415 | distributedring_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11700 | 18 | 0 |
| held-out | 31415 | distributedring_bg300_r6x3 | calibrated_lr_crc | 100% | 94% | 0.97 | ₹0 | ₹0 | ₹11700 | 10 | 7 |
| held-out | 31415 | merchantcollusion_bg300_r6x3 | per_customer_rate_rule | 100% | 61% | 0.76 | ₹0 | ₹2700 | ₹9000 | 11 | 0 |
| held-out | 31415 | merchantcollusion_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11700 | 18 | 0 |
| held-out | 31415 | merchantcollusion_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11700 | 18 | 0 |
| held-out | 31415 | merchantcollusion_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11700 | 18 | 0 |
| held-out | 31415 | merchantcollusion_bg300_r6x3 | calibrated_lr_crc | 100% | 94% | 0.97 | ₹0 | ₹0 | ₹11700 | 14 | 3 |
| held-out | 31415 | adversarialevasion_bg300_r6x3 | per_customer_rate_rule | 100% | 56% | 0.71 | ₹0 | ₹3150 | ₹9000 | 10 | 0 |
| held-out | 31415 | adversarialevasion_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹12150 | 18 | 0 |
| held-out | 31415 | adversarialevasion_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹12150 | 18 | 0 |
| held-out | 31415 | adversarialevasion_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹12150 | 18 | 0 |
| held-out | 31415 | adversarialevasion_bg300_r6x3 | calibrated_lr_crc | 100% | 89% | 0.94 | ₹0 | ₹450 | ₹11700 | 0 | 16 |
| held-out | 27182 | normal_bg300_r0x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | normal_bg300_r0x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | normal_bg300_r0x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | normal_bg300_r0x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | normal_bg300_r0x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | household_bg300_r8x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | household_bg300_r8x3 | structural_cluster_only | 0% | 100% | 0.00 | ₹5850 | ₹0 | ₹0 | 24 | 0 |
| held-out | 27182 | household_bg300_r8x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | household_bg300_r8x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | household_bg300_r8x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | coincidentalsharing_bg300_r8x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | coincidentalsharing_bg300_r8x3 | structural_cluster_only | 0% | 100% | 0.00 | ₹22162 | ₹0 | ₹0 | 88 | 0 |
| held-out | 27182 | coincidentalsharing_bg300_r8x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | coincidentalsharing_bg300_r8x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | coincidentalsharing_bg300_r8x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 27182 | returnabuse_bg300_r6x3 | per_customer_rate_rule | 100% | 78% | 0.88 | ₹0 | ₹1800 | ₹13500 | 14 | 0 |
| held-out | 27182 | returnabuse_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹15300 | 18 | 0 |
| held-out | 27182 | returnabuse_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹15300 | 18 | 0 |
| held-out | 27182 | returnabuse_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹15300 | 18 | 0 |
| held-out | 27182 | returnabuse_bg300_r6x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹15300 | 18 | 0 |
| held-out | 27182 | refundabuse_bg300_r6x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹19800 | 18 | 0 |
| held-out | 27182 | refundabuse_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹19800 | 18 | 0 |
| held-out | 27182 | refundabuse_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹19800 | 18 | 0 |
| held-out | 27182 | refundabuse_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹19800 | 18 | 0 |
| held-out | 27182 | refundabuse_bg300_r6x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹19800 | 0 | 18 |
| held-out | 27182 | distributedring_bg300_r6x3 | per_customer_rate_rule | 100% | 44% | 0.62 | ₹0 | ₹4050 | ₹5850 | 8 | 0 |
| held-out | 27182 | distributedring_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹9900 | 18 | 0 |
| held-out | 27182 | distributedring_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹9900 | 18 | 0 |
| held-out | 27182 | distributedring_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹9900 | 18 | 0 |
| held-out | 27182 | distributedring_bg300_r6x3 | calibrated_lr_crc | 100% | 94% | 0.97 | ₹0 | ₹0 | ₹9900 | 8 | 9 |
| held-out | 27182 | merchantcollusion_bg300_r6x3 | per_customer_rate_rule | 100% | 78% | 0.88 | ₹0 | ₹1800 | ₹13500 | 14 | 0 |
| held-out | 27182 | merchantcollusion_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹15300 | 18 | 0 |
| held-out | 27182 | merchantcollusion_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹15300 | 18 | 0 |
| held-out | 27182 | merchantcollusion_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹15300 | 18 | 0 |
| held-out | 27182 | merchantcollusion_bg300_r6x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹15300 | 18 | 0 |
| held-out | 27182 | adversarialevasion_bg300_r6x3 | per_customer_rate_rule | 100% | 72% | 0.84 | ₹0 | ₹1800 | ₹13950 | 13 | 0 |
| held-out | 27182 | adversarialevasion_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹15750 | 18 | 0 |
| held-out | 27182 | adversarialevasion_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹15750 | 18 | 0 |
| held-out | 27182 | adversarialevasion_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹15750 | 18 | 0 |
| held-out | 27182 | adversarialevasion_bg300_r6x3 | calibrated_lr_crc | 100% | 94% | 0.97 | ₹0 | ₹0 | ₹15750 | 1 | 16 |
| held-out | 16180 | normal_bg300_r0x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | normal_bg300_r0x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | normal_bg300_r0x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | normal_bg300_r0x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | normal_bg300_r0x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | household_bg300_r8x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | household_bg300_r8x3 | structural_cluster_only | 0% | 100% | 0.00 | ₹6525 | ₹0 | ₹0 | 24 | 0 |
| held-out | 16180 | household_bg300_r8x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | household_bg300_r8x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | household_bg300_r8x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | coincidentalsharing_bg300_r8x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | coincidentalsharing_bg300_r8x3 | structural_cluster_only | 0% | 100% | 0.00 | ₹19800 | ₹0 | ₹0 | 88 | 0 |
| held-out | 16180 | coincidentalsharing_bg300_r8x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | coincidentalsharing_bg300_r8x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | coincidentalsharing_bg300_r8x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹0 | 0 | 0 |
| held-out | 16180 | returnabuse_bg300_r6x3 | per_customer_rate_rule | 100% | 50% | 0.67 | ₹0 | ₹3600 | ₹9000 | 9 | 0 |
| held-out | 16180 | returnabuse_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹12600 | 18 | 0 |
| held-out | 16180 | returnabuse_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹12600 | 18 | 0 |
| held-out | 16180 | returnabuse_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹12600 | 18 | 0 |
| held-out | 16180 | returnabuse_bg300_r6x3 | calibrated_lr_crc | 100% | 94% | 0.97 | ₹0 | ₹0 | ₹12600 | 11 | 6 |
| held-out | 16180 | refundabuse_bg300_r6x3 | per_customer_rate_rule | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹18450 | 18 | 0 |
| held-out | 16180 | refundabuse_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹18450 | 18 | 0 |
| held-out | 16180 | refundabuse_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹18450 | 18 | 0 |
| held-out | 16180 | refundabuse_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹18450 | 18 | 0 |
| held-out | 16180 | refundabuse_bg300_r6x3 | calibrated_lr_crc | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹18450 | 0 | 18 |
| held-out | 16180 | distributedring_bg300_r6x3 | per_customer_rate_rule | 100% | 33% | 0.50 | ₹0 | ₹4050 | ₹4050 | 6 | 0 |
| held-out | 16180 | distributedring_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹8100 | 18 | 0 |
| held-out | 16180 | distributedring_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹8100 | 18 | 0 |
| held-out | 16180 | distributedring_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹8100 | 18 | 0 |
| held-out | 16180 | distributedring_bg300_r6x3 | calibrated_lr_crc | 100% | 83% | 0.91 | ₹0 | ₹0 | ₹8100 | 7 | 8 |
| held-out | 16180 | merchantcollusion_bg300_r6x3 | per_customer_rate_rule | 100% | 50% | 0.67 | ₹0 | ₹3600 | ₹9000 | 9 | 0 |
| held-out | 16180 | merchantcollusion_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹12600 | 18 | 0 |
| held-out | 16180 | merchantcollusion_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹12600 | 18 | 0 |
| held-out | 16180 | merchantcollusion_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹12600 | 18 | 0 |
| held-out | 16180 | merchantcollusion_bg300_r6x3 | calibrated_lr_crc | 100% | 94% | 0.97 | ₹0 | ₹0 | ₹12600 | 11 | 6 |
| held-out | 16180 | adversarialevasion_bg300_r6x3 | per_customer_rate_rule | 100% | 50% | 0.67 | ₹0 | ₹2700 | ₹8550 | 9 | 0 |
| held-out | 16180 | adversarialevasion_bg300_r6x3 | structural_cluster_only | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11250 | 18 | 0 |
| held-out | 16180 | adversarialevasion_bg300_r6x3 | investigation_engine | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11250 | 15 | 3 |
| held-out | 16180 | adversarialevasion_bg300_r6x3 | learned_logistic | 100% | 100% | 1.00 | ₹0 | ₹0 | ₹11250 | 18 | 0 |
| held-out | 16180 | adversarialevasion_bg300_r6x3 | calibrated_lr_crc | 100% | 72% | 0.84 | ₹0 | ₹900 | ₹10350 | 1 | 12 |


=== HEADLINE: aggregate across HELD-OUT abuse worlds ===
per_customer_rate_rule: precision=100% recall=66% | FP cost ₹0 | FN cost ₹34650 | prevented ₹168300
structural_cluster_only: precision=51% recall=100% | FP cost ₹65925 | FN cost ₹0 | prevented ₹202950
investigation_engine: precision=100% recall=100% | FP cost ₹0 | FN cost ₹0 | prevented ₹202950
learned_logistic: precision=100% recall=100% | FP cost ₹0 | FN cost ₹0 | prevented ₹202950
calibrated_lr_crc: precision=100% recall=94% | FP cost ₹0 | FN cost ₹1350 | prevented ₹201600

=== Household FP check across ALL held-out seeds (all customers legitimate) ===
per_customer_rate_rule: FP=3 of 972 customers across 3 seeds (₹1575 friction cost)
structural_cluster_only: FP=72 of 972 customers across 3 seeds (₹16988 friction cost)
investigation_engine: FP=0 of 972 customers across 3 seeds (₹0 friction cost)
learned_logistic: FP=0 of 972 customers across 3 seeds (₹0 friction cost)
calibrated_lr_crc: FP=0 of 972 customers across 3 seeds (₹0 friction cost)

=== ROBUSTNESS: degradation sweep over held-out seeds ===
mess      precision     recall     FP     FN      FN cost    legit flagged  review share
clean        100.0%     100.0%      0      0 ₹          0         0 of 2136           1.1%
mild         100.0%     100.0%      0      0 ₹          0         0 of 1926          35.6%
heavy        100.0%     100.0%      0      0 ₹          0        14 of 1507          58.9%

=== ROBUSTNESS: randomized world shapes (parameters never tuned against) ===
investigation_engine across 140 randomly-drawn worlds: precision=100.0% recall=99.4% | legit customers flagged: 0

=== CRC GUARANTEE VALIDATION: forced class overlap (camouflaged abusers) ===
 run  tau_clear  tau_block     leak friction  missed@r  review%     psi
   0     0.0474     0.2734  1.83%  1.83%     22.2%   26.6%   1.024
   1     0.0412     0.2662  1.38%  0.92%     16.7%   31.7%   0.957
   2     0.0414     0.2721  3.21%  0.92%     38.9%   33.0%   0.982
   3     0.0404     0.2750  1.83%  2.29%     22.2%   30.3%   1.027
   4     0.0394     0.2737  1.83%  1.38%     22.2%   40.8%   0.988
   5     0.0375     0.2625  1.83%  0.92%     22.2%   39.0%   0.944
   6     0.0426     0.2769  0.92%  0.46%     11.1%   37.6%   0.957
   7     0.0340     0.3010  0.46%  0.00%      5.6%   43.6%   0.988
   8     0.0279     0.2744  0.92%  0.46%     11.1%   48.2%   0.944
   9     0.0511     0.2666  3.21%  0.46%     38.9%   32.6%   0.955
  10     0.0361     0.2539  2.29%  2.75%     27.8%   42.2%   0.943
  11     0.0439     0.2938  0.46%  1.83%      5.6%   32.6%   1.064

verdict over 12 runs (2616 customers scored): leaked 44 vs α·N=52 (z=-1.16, budget HOLDS) | blocked-legit 31 vs α·N=26 (z=0.95, budget HOLDS) | worst-run leak 3.21% friction 2.75% | mean review share 36.5% | missed-recall under camouflage 20.4% | mean PSI 0.98
note: leak/friction are UNCONDITIONAL rates — the quantity the CRC bound governs; 'missed@r' is leaks-per-abuser (context only). tau_clear collapsing from ~0.23 to ~0.04 is the designed response to overlap: uncertainty routes to review.
