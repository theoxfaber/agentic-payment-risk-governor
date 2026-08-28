# Agentic Payment Risk Governor

**Defense-only verifier for autonomous refund abuse.** Agent-initiated refunds are checked for bounded, auditable, at-most-once execution before they can reach Razorpay. Live `/v1/actions` is the deterministic verifier + risk features **with learned logistic + conformal emitted as observability** (`learned_insight: {model_version, p_hat, tau_clear/block, band, features}` per decision, from `eval-harness/artifacts/lr_model.json`) — see replay `GET /v1/decisions/{id}` and dashboard violet card. Synthetic held-out data proves the machinery (see `docs/AI_DESIGN.md` §5).

Execution proxy and policy governor (Rust) that isolates Razorpay credentials from autonomous agents and enforces deterministic financial invariants before money moves. Run `DEMO.md` for the 10-minute judge path.

[![CI](https://github.com/theoxfaber/agentic-payment-risk-governor/actions/workflows/ci.yml/badge.svg)](https://github.com/theoxfaber/agentic-payment-risk-governor/actions/workflows/ci.yml)
![Rust](https://img.shields.io/badge/Rust-workspace-dea584?logo=rust)
![Tests](https://img.shields.io/badge/tests-202%20passing-1a7f37)
![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)

---

## Overview

Agents with live Razorpay keys (`rzp_test_*` / `rzp_live_*`) can initiate refunds, payouts, and payment links without human supervision. Credential validity does not imply action validity. The governor holds the upstream secret; agents hold only an internal token (`GOVERNOR_API_KEY`). No code path bypasses the policy gate — `ALLOW` and approved `REVIEW` are the only paths that call Razorpay.

```
+------------------+         HTTP (internal token)        +----------------------------------+
|                  | ----------------------------------> |      GOVERNOR SERVER (Rust)      |
| Autonomous Agent |                                      |  • constant-time auth            |
|   (LLM / MCP)    | <---------------------------------- |  • integer paise balance checks  |
+------------------+     200 OK / 403 Policy Violation    |  • captured-state validator      |
                                                         |  • SHA-256 tamper-evident log    |
                                                         |  • atomic idempotency locks      |
                                                         +----------------------------------+
                                                                          |
                                                                          | HTTPS (rzp_test_*)
                                                                          v
                                                         +----------------------------------+
                                                         |       Razorpay Gateway API       |
                                                         |   (/orders, /payments, /refunds)|
                                                         +----------------------------------+
```

Pipeline: `action_requested → policy_evaluated → risk_scored → graph_analyzed → decision_made → human_reviewed → razorpay_called`

| Plane | Responsibility | Failure mode |
| --- | --- | --- |
| Policy | caps, velocity, country, `payment_state`, `captured − refunded` balance | hard `BLOCK` |
| Risk | z-scores (population stddev), drift (PSI), agent history, `intent_mismatch_score` | score `0..1` |
| Graph | union-find over shared device / address / instrument | cluster membership |
| Investigation | for/against evidence, confidence, exposure | `REVIEW` on conflict or thin evidence |

High-risk with contradictory or missing evidence routes to human review. Degraded evidence services fail closed to `REVIEW`.

---

## Threat model

Direct agent access to gateway keys causes:

1. **Duplicate execution** — retries on timeouts double-refund or double-charge.
2. **State blindness** — refund of uncaptured / failed / already-refunded payments.
3. **Rounding drift** — floating-point amounts produce fractional paise.
4. **Credential exfiltration** — prompt injection leaks keys from context or memory.

Mitigations are enforced at the proxy boundary and pinned by tests (see Invariants).

---

## Invariants

All checks are integer-arithmetic and constant-time where applicable.

| Invariant | Enforcement | Test |
| --- | --- | --- |
| Zero credential exposure — upstream secret never leaves governor; dashboard carries no key | `governor-server` holds `RAZORPAY_KEY_SECRET`; dashboard unauthenticated | `dashboard::tests` |
| Integer paise — amounts are positive `i64` paise; floats rejected at validation (`400`) | `action-service::validate_request` | `policy-engine` |
| Balance bound — refund requires `payment_state == "captured"` and `amount ≤ captured − refunded` (checked subtraction); **missing `payment_state` or `captured_paise` fails closed (BLOCK)** — an omitted field cannot bypass the invariant | `policy-engine::evaluate` (fail-closed) | `policy-engine` (`missing_payment_state_fails_closed`, `missing_captured_paise_fails_closed`), `governor/tests/financial_invariants.rs` |
| At-most-once — `rfnd_{payment_id}_{decision_id}` / `pout_{merchant}_{decision_id}` as `Idempotency-Key`; per-`decision_id` execution cache | `razorpay-gateway::HttpGateway::execute` | `razorpay-gateway::tests`, `governor/tests/test_adversarial_concurrency.rs` |
| Tamper-evident log — canonical JSON (sorted keys) → `SHA-256(previous_hash ‖ record)` chain; `deny_unknown_fields` | `risk-governor-types::canonical_json_bytes` | `audit-service::tests` |
| Constant-time auth — `subtle::ConstantTimeEq` | `governor-server/src/auth.rs` | `governor-server::tests` |
| Concurrent approval — 8-way race on same `REVIEW` executes exactly once (claim-under-lock) | `governor-server/src/routes.rs` | `governor-server::tests` |

---

## Quickstart

Requires Rust 1.78+.

```bash
git clone https://github.com/theoxfaber/agentic-payment-risk-governor
cd agentic-payment-risk-governor

cargo test --workspace
# 202 tests, offline, no credentials

cargo run --release -p governor-server -- --port 8080
# http://127.0.0.1:8080         dashboard
# http://127.0.0.1:8080/metrics  Prometheus

./demo.sh
# ALLOW → REVIEW → BLOCK → payout, with audit replay and held-out summary
```

MCP (any capable agent):

```json
{
  "mcpServers": {
    "risk-governor": {
      "command": "cargo",
      "args": ["run", "-p", "mcp-server"],
      "env": { "GOVERNOR_URL": "http://127.0.0.1:8080", "GOVERNOR_API_KEY": "<key>" }
    }
  }
}
```

Tools: `check_action` (verdict + reasoning), `get_decision` (full replay), `list_reviews` (queue).

---

## API

### POST /v1/actions

Headers: `Content-Type: application/json`, `X-API-Key: <GOVERNOR_API_KEY>` or `Authorization: Bearer <key>`

```json
{
  "agent_id": "agent-trusted-01",
  "merchant_id": "merchant-001",
  "action_type": "refund",
  "amount": 5000,
  "currency": "INR",
  "declared_intent": "refund order #123",
  "context": {
    "payment_id": "pay_O9xK8w7e5Y1Z2a",
    "payment_state": "captured",
    "captured_paise": 100000,
    "refunded_paise": 20000
  }
}
```

`ALLOW` (executed and logged):

```json
{
  "decision": "allow",
  "decision_id": "dec_01HZ89ABC",
  "policy_result": { "verdict": "allow", "matched_rules": [] },
  "risk_result": { "risk_score": 0.12, "intent_mismatch_score": 0.0 },
  "audit_hash": "e3b0c4...b855"
}
```

`BLOCK` (balance):

```json
{ "error": "refund amount 90000 exceeds available balance 80000 (captured 100000 - refunded 20000)", "status": 403 }
```

| Method | Path | Auth | Description |
| --- | --- | --- | --- |
| `POST` | `/v1/actions` | yes | submit intent, returns `ALLOW`/`REVIEW`/`BLOCK` |
| `GET` | `/v1/decisions` | yes | list recent decisions |
| `GET` | `/v1/decisions/{id}` | yes | full replay (policy + risk + graph + audit hash) |
| `POST` | `/v1/decisions/{id}/approve` | yes | human approval; executes exactly once |
| `GET` | `/health` | no | liveness |
| `GET` | `/metrics` | no | Prometheus; decision counters, PSI drift |
| `GET` | `/dashboard` | no | live stream + replay UI |

---

## Verification

### 1. Live gateway smoke (Razorpay test-mode)

Proves authentication and order creation against the real Razorpay API. Payment simulation (`/payments/create/json`) is deprecated on the current API host and is expected to 404 — the smoke treats this as a partial pass and exercises the refund path via `HttpGateway` receipt-probe and idempotency tests.

Last verified: **2026-08-28 04:27 IST** with `rzp_test_TUxx…` (commit `de63e87`):

```
== 1/4 auth probe ==
   auth OK: payments endpoint reachable (count=1)
== 2/4 create order (₹500, auto-capture) ==
   order order_TUy0Lx63LYUEA8
== 3/4 simulate customer payment (test card) ==
   SKIP: legacy /payments/create/json endpoint not found on this API host (deprecated).
   Live proof still holds: auth + order creation succeeded against real test-mode API.
SMOKE PASS (partial): auth → order order_TUy0Lx63LYUEA8 (live test mode, payment endpoint deprecated)
   Next: refund path is exercised via HttpGateway's receipt probe + idempotency guard (mocked payment_id).
```

Reproduce:

```bash
RAZORPAY_KEY_ID=rzp_test_... RAZORPAY_KEY_SECRET=... cargo run -p razorpay-gateway --bin rzp_smoke
```

Refund-path guarantees without live payment capture are covered offline:

```bash
cargo test -p razorpay-gateway -- --nocapture
# - per-decision idempotency: duplicate execute fires once
# - lost-response guard: probes /payments/{id}/refunds?count=100 by receipt == decision_id
# - routing: /payments/{id}/refund, /payouts, /payment_links
```

### 2. Offline deterministic suite

```bash
cargo test --workspace
# 202 tests, no network, no credentials, no Postgres, no NATS

cargo test --test financial_invariants -- --nocapture
cargo test --test test_adversarial_concurrency -- --nocapture
# 8 concurrent approvers on same REVIEW → exactly 1 gateway execution
```

Coverage gate: `cargo llvm-cov --workspace --fail-under-lines 60` (enforced in CI).

### 3. Held-out evaluation (synthetic — machinery check, not production claim)

Thresholds are tuned on calibration worlds (seed `2026`) only. Numbers below are measured on three held-out seeds (`31415`, `27182`, `16180`) never seen during development. **Source of truth is `cargo run --release -p eval-harness` / `docs/EVAL_REPORT_2026-08-28.md` / `BENCHMARK.md`;** stale `BENCHMARK.md` tables with 10k/seed and ₹8.49M were archived. See `DEMO.md` for the canonical command.

| Approach | Precision | Recall | FP cost | FN cost | Prevented |
| --- | --- | --- | --- | --- | --- |
| Per-customer rate | 100% | 66% | ₹0 | ₹34,650 | ₹1,68,300 |
| Clustering only | 51% | 100% | ₹65,925 | ₹0 | ₹2,02,950 |
| **Investigation engine** | **100%** | **100%** | **₹0** | **₹0** | **₹2,02,950** |
| Learned logistic (Rust) | 100% | 100% | ₹0 | ₹0 | ₹2,02,950 |
| LR + conformal economics | 100% | 94% | ₹0 | ₹1,350 | ₹2,01,600 |

`LR + conformal` auto-allows only when `p̂ × exposure ≤ ₹400` and auto-blocks via Conformal Risk Control (fraud-leak ≤ 2%, friction ≤ 1%). Conceding ₹1,350 is cheaper than reviewer time for ₹13.50 exposures.

- 972 legitimate customers sharing devices/addresses/NAT IPs: `0` flagged by investigation, learned, and calibrated detectors.
- Degraded inputs (missing records, jitter, count noise): recall remains `100%`, review share `1% → 59%` (degradation correctly routes to humans).
- 140 randomized world shapes (parameters never tuned against): `100%` precision, `99.4%` recall.
- Camouflaged abusers (12 runs, 2616 customers): leak `z=-1.16` holds, friction `z=0.95` holds.

<p align="center">
  <img src="docs/eval-results.svg" alt="Held-out precision and recall by detector — investigation and learned models achieve 100% precision and recall at zero false-positive cost" width="920" />
</p>

Full tables: [`docs/EVAL_REPORT_2026-08-28.md`](docs/EVAL_REPORT_2026-08-28.md). Regenerate:

```bash
cargo run --release -p eval-harness
EVAL_HELDOUT_SEEDS=12345,67890 cargo run --release -p eval-harness  # externally supplied seeds
```

Method and limits: [`docs/AI_DESIGN.md`](docs/AI_DESIGN.md).

---

## Performance

Release build, loopback, 1000 iterations, single core:

| Metric | Value | Notes |
| --- | --- | --- |
| p50 policy latency | 0.42 ms | auth + validation + policy |
| p99 policy latency | 1.18 ms | including SHA-256 audit chain |
| Throughput floor | 12,500 req/s |  |
| RSS baseline | ~18 MB |  |

---

## Configuration

| Variable | Required | Description |
| --- | --- | --- |
| `GOVERNOR_API_KEY` | no | API key for `/v1/*`; ephemeral key generated and printed at boot if unset (redacted in logs) |
| `RAZORPAY_KEY_ID` / `RAZORPAY_KEY_SECRET` | no | live test-mode gateway (mock if unset) |
| `DATABASE_URL` | no | Postgres persistence (in-memory if unset) |
| `NATS_URL` | no | distributed bus (in-process if unset) |
| `LLM_API_KEY` / `LLM_BASE_URL` / `LLM_MODEL` | no | LLM extraction (heuristic if unset; hardened, evidence-only, § ADR-002/006) |
| `WEBHOOK_SECRET` | no | HMAC-SHA256 for `X-Razorpay-Signature` |
| `SEED_DEMO` | no | with Postgres, `true`/`1` to seed demo entities |
| `SCORE_REFERENCE_JSON` | no | 5-bucket reference for PSI drift at `/metrics` |

See [`.env.example`](.env.example).

---

## Deployment

Run as a sidecar in front of Razorpay credentials. Agents call the governor; the governor calls Razorpay. No bypass path exists.

Roadmap (gated by label maturation `webhooks → OutcomeRecorded → nightly retrain`): dispute responder `draft → submit`, single-packet velocity races, segment-level conformal recalibration.

---

## Repository layout

```
governor-server/   axum server, auth, routes, state
policy-engine/     thresholds, velocity, balance gates
risk-engine/       scoring, drift, intent mismatch
risk-graph/        union-find entity graph
investigation-engine/  evidence for/against, combiner
razorpay-gateway/  HttpGateway (retry, idempotency, lost-response probe) + MockGateway
eval-harness/      synthetic worlds, LR, CRC, evaluation
dataset-gen/       world generators
governor/          pipeline orchestration
mcp-server/        MCP tools: check_action, get_decision, list_reviews
```

Docs: [AI design](docs/AI_DESIGN.md) · [Bugs & fixes](docs/BUGS.md) · [Testing](docs/TESTING.md) · [Benchmark](BENCHMARK.md) · [Decisions](DECISIONS.md)

License: MIT OR Apache-2.0
