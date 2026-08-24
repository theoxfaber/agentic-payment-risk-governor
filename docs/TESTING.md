# Testing Guide

Every test in this workspace runs **fully offline**: no network, no Razorpay
credentials, no Postgres, no NATS, no LLM API keys. A fresh clone needs only
a Rust toolchain:

```bash
cargo test --workspace        # 154 tests, all green, zero environment setup
```

## Test inventory by crate

Counts generated from `cargo test --workspace` output (integration binaries
attributed to their crate):

| Crate | Tests | What's covered |
|---|---:|---|
| `action-service` | 12 | Combiner safety property (high risk + contradicted evidence → REVIEW, never BLOCK), validation, correlation-ID threading, degraded-evidence fail-safe |
| `audit-service` | 4 | In-memory store append/query, replay reconstruction |
| `razorpay-gateway` | 10 | Per-decision idempotency (duplicate execute fires once), refund lost-response guard against a local mock server, payout/payment-link endpoint routing, constant-time webhook signature verification |
| `governor-server` | 12 | Full production router through auth middleware (401 without/wrong key, Bearer accepted), handler behavior: decision counters, approval flow, double-approval rejection, replay with complete audit trail, 404s |
| `risk-engine` | 12 | Feature extraction bounds, intent mismatch (keywords + extracted claims), declared-vs-actual amount contradiction |
| `policy-engine` | 10 | Threshold/velocity/country/custom-rule evaluation |
| `investigation-engine` | 11 | Verdict asymmetry (Supported / Conflicted / Unsupported), adversarial-evasion hold rule, household exoneration, partial-visibility confidence dampening, exposure accounting |
| `risk-graph` | 16 | Entity graph construction, abuse-ring clustering, shared-resource linkage |
| `intent-engine` | 8 | Heuristic claim extraction (amounts, order refs, urgency), LLM client against a local OpenAI-compatible mock, degraded fallback on failure/timeout |
| `mcp-server` | 10 | MCP handshake, tools/list schema, tool calls end-to-end against a fake governor, JSON-RPC error paths |
| `evidence-service` | 7 | History accumulation, velocity stats, gather semantics |
| `eval-harness` | 6 | **Held-out regression gates:** evasion recall ≥90% on every unseen seed, zero false positives on households and coincidental sharing across all held-out seeds, detector contrast preserved |
| `dataset-gen` | 6 | Adversarial world generators produce non-degenerate fixtures |
| `evaluation-service` | 4 | Labeled-dataset pipeline with cost accounting |
| `webhook-consumer` | 5 | Integration tests over a real axum router |
| `risk-governor-correlation` | 6 | Correlation-ID task-local scoping across bus calls |
| `risk-governor-replay` | 3 | Decision reconstruction from audit trail |
| `governor` | 8 | End-to-end pipeline scenarios incl. investigated decisions; chaos/distributed suites are `#[ignore]`-tagged (need live infra) |
| `pg-store` | 2 | Seed JSON parsing (live-DB paths are `#[ignore]`-tagged) |
| `dashboard` | 2 | Auth-header wiring in served page, key escaping |

**Total: 154.**

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
cargo run --release -p eval-harness    # calibration + held-out metric tables
```
