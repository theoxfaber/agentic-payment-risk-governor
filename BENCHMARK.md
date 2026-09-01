# Benchmark — Canonical Held-Out Evaluation

> **Source of truth:** `cargo run --release -p eval-harness` (or `EVAL_HELDOUT_SEEDS=...` for externally supplied seeds). Numbers below are generated from `docs/EVAL_REPORT_2026-08-28.md` — do not edit manually. Data is **synthetic** (proves the train → calibrate → guarantee → monitor machinery, not production fraud performance).

**Calibration:** seed `2026` only. **Held-out:** seeds `31415`, `27182`, `16180` (never seen during tuning). Worlds: `normal`, `household`, `coincidental_sharing`, `return_abuse`, `refund_abuse`, `distributed_ring`, `merchant_collusion`, `adversarial_evasion` (300 background for CI, `EVAL_SCALE=large` → 10,000+ customers per world, 80k+ per seed).

---

## 1. Held-out aggregate (abuse worlds, 3 seeds)

| Approach | Precision | Recall | FP cost | FN cost | Prevented |
|---|---:|---:|---:|---:|---:|
| Per-customer rate | 100% | 66% | ₹0 | ₹34,650 | ₹1,68,300 |
| Clustering only | 51% | 100% | ₹65,925 | ₹0 | ₹2,02,950 |
| **Investigation engine** | **100%** | **100%** | **₹0** | **₹0** | **₹2,02,950** |
| Learned logistic (Rust) | 100% | 100% | ₹0 | ₹0 | ₹2,02,950 |
| LR + conformal economics (`p̂×exposure ≤ ₹400`, fraud-leak ≤2%, friction ≤1%) | 100% | 94% | ₹0 | ₹1,350 | ₹2,01,600 |

`LR + conformal` concedes ₹1,350 vs investigation engine — cheaper than spending ₹400 of reviewer time to prevent ₹13.50 exposures per instance. Chart: `docs/eval-results.svg`.

Household false-positive check (972 legitimate customers across 3 held-out seeds sharing devices/addresses/NAT IPs): investigation, learned, and calibrated each flag **0 of 972** (clustering flags 72, per-customer flags 3).

---

## 2. Robustness

**Degradation sweep (held-out):** missing records + jitter + count noise → recall stays **100%**, review share **1.1% → 35.6% (mild) → 58.9% (heavy)**, legit flagged `0 → 0 → 14 of 1507`. Degradation correctly routes to humans.

**Randomized worlds (140 worlds, parameters never tuned against):** investigation engine `100.0%` precision, `99.4%` recall, `0` legit customers flagged.

**Camouflaged abusers (forced overlap, 12 runs, 2616 customers):** CRC budgets hold — leaked `44 vs 52` (`z=-1.16`), blocked-legit `31 vs 26` (`z=0.95`), worst-run leak `3.21%` friction `2.75%`, mean review share `36.5%`, `tau_clear` collapses `0.23 → ~0.04` (designed conservative response), mean PSI `0.98`.

---

## 3. Reproduce

```bash
cargo run --release -p eval-harness
EVAL_HELDOUT_SEEDS=12345,67890 cargo run --release -p eval-harness
cat docs/EVAL_REPORT_2026-08-28.md
```

Regenerates `docs/eval-results.svg` + `eval-harness/artifacts/lr_model.json`. Previous `BENCHMARK.md` tables with 10k/seed and ₹8.49M were from an earlier synthetic scale and are archived — current canonical surface is the held-out report above.

---

## 4. Live gateway (separate from synthetic)

Test-mode smoke `2026-08-28T04:27 IST` with `rzp_test_TUxx…`: `GET /payments?count=1` auth OK, `POST /orders` created `order_TUyv0Ib1swX7ki` live. `POST /payments/create/json` returned `404` (deprecated on current Razorpay host) — smoke treats as partial pass; refund path is covered by `HttpGateway` idempotency + receipt-probe tests (`cargo test -p razorpay-gateway`). Not a live refund of a captured payment.

---

## 5. Production safety & compliance (not a benchmark)

Latency: `GET /metrics` now exposes `risk_governor_request_duration_ms` histogram (buckets 20/50/100/200ms) — p50 `0.42 ms`, p95 `<180ms` (<200ms Thirdwatch SLO), p99 `1.18 ms` (incl. SHA-256 chain), `12,500 req/s` single-core, `~18 MB` RSS. Invariants: integer paise, `captured` gate + `captured−refunded` checked subtraction, `rfnd_{payment_id}_{decision_id}` idempotency, `previous_hash→current_hash` chain, `subtle::ct_eq`, claim-under-lock 8-way race → 1 execution. RBI: `risk_tier` SDD/CDD/EDD + DoT FRI `Medium/High/VeryHigh` → `policy-engine` fail-closed + 1825d PMLA retention; DPDP Act: audit redacts `email/phone/payment_id→sha256` recursively (incl. nested `payment.*`). Explainability: `learned_insight.contributions` per-feature SHAP (`weight*(x-mean)/std`) — `action-service` sorts by `|contribution|` before `HashMap` insert; `/v1/decisions/{id}` caller sorts for display.
