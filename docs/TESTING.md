# Testing Guide

Every test in this workspace runs **fully offline**: no network, no Razorpay
credentials, no Postgres, no NATS, no LLM API keys. A fresh clone needs only
a Rust toolchain:

```bash
cargo test --workspace        # 184 tests, all green, zero environment setup
```

## Test inventory by crate

Counts generated from `cargo test --workspace` output (integration binaries
attributed to their crate):

| Crate | Tests | What's covered |
|---|---:|---|
| `governor-server` | 16 | Full production router through auth middleware (401 without/wrong key, Bearer accepted), handler behavior: decision counters, approval flow, **8-way concurrent approval executes exactly once**, replay with complete audit trail, 404s, caller-validation → 400, PSI/score-bucket metrics |
| `action-service` | 14 | Combiner safety property (high risk + contradicted evidence → REVIEW, never BLOCK), **intent-contradiction forces review / stays silent when mild**, validation as pipeline step zero, correlation-ID threading, degraded-evidence fail-safe |
| `risk-engine` | 12 | Feature extraction bounds, intent mismatch (keywords + extracted claims), declared-vs-actual amount contradiction |
| `intent-engine` | 12 | Heuristic claim extraction (amounts, order refs, urgency), LLM client against a local OpenAI-compatible mock, degraded fallback on failure/timeout, **prompt-injection sanitization** (role openers, fences, control chars, length cap) |
| `eval-harness` | 21 | **Learned layer:** LR separates classes deterministically, round-trips through JSON, CRC budgets hold empirically on calibration data, economics rule blocks high-exposure convictions while clearing cheap uncertainty, households never auto-blocked. **Held-out regression gates:** evasion recall ≥90% on every unseen seed, zero false positives on households/coincidental sharing across all held-out seeds; **robustness gates:** recall holds under degradation with review share rising, randomized-world precision/recall ≥95% *with legit FPs pooled in*, perturbation harness non-vacuous |
| `policy-engine` | 11 | Threshold/velocity/country/custom-rule evaluation, **unknown custom-rule conditions fail closed** |
| `investigation-engine` | 11 | Verdict asymmetry (Supported / Conflicted / Unsupported), adversarial-evasion hold rule, household exoneration, partial-visibility confidence dampening, exposure accounting |
| `risk-graph` | 10 | Entity graph construction, abuse-ring clustering, transitive merge keeps all link kinds, deterministic output |
| `mcp-server` | 10 | MCP handshake, tools/list schema, tool calls end-to-end against a fake governor, JSON-RPC error paths |
| `evidence-service` | 7 | History accumulation, velocity stats, gather semantics |
| `razorpay-gateway` | 12 | Per-decision idempotency (duplicate execute fires once), refund lost-response guard against a local mock server, payout/payment-link endpoint routing, constant-time webhook signature verification |
| `webhook-consumer` | 5 | Integration tests over a real axum router |
| `audit-service` | 4 | In-memory store append/query, replay reconstruction |
| `evaluation-service` | 4 | Labeled-dataset pipeline with cost accounting |
| `dataset-gen` | 6 | Adversarial world generators produce non-degenerate fixtures; label-free trimmed baseline estimation |
| `governor` | 8 | End-to-end pipeline scenarios incl. investigated decisions; chaos/distributed suites are `#[ignore]`-tagged (need live infra) |
| `dashboard` | 3 | **Page carries no credential**, auth-header wiring, escaped id interpolation |
| `risk-governor-replay` | 3 | Decision reconstruction from audit trail |
| `risk-governor-correlation` | 6 | Correlation-ID task-local scoping across bus calls |
| `pg-store` | 2 | Seed JSON parsing (live-DB paths are `#[ignore]`-tagged) |

**Total: 184.**

## Which tests touch external infrastructure

None by default. The only exceptions are opt-in, explicitly tagged so they
never run unless you ask for them:

| Suite | Requires | How to run |
|---|---|---|
| Distributed pipeline (`chaos`, `distributed`) | `docker compose up -d` (NATS + Postgres) | `cargo test -p governor -- --ignored` |
| Live Razorpay smoke (`rzp_smoke`) | `RAZORPAY_KEY_ID` + `RAZORPAY_KEY_SECRET` (test mode) | `cargo run -p razorpay-gateway --bin rzp_smoke` |

CI runs plain `cargo test --workspace` with **no credentials set**, which is
exactly what a fresh clone gets.

## Reproducing the numbers

```bash
cargo test --workspace                 # full suite
cargo llvm-cov --workspace --text      # line coverage (CI fails below 60%)
cargo run --release -p eval-harness    # calibration + held-out tables,
                                       # learned-layer artifact export
```
