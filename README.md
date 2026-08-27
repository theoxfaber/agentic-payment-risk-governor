# Agentic Payment Risk Governor

An out-of-process execution proxy and policy governor in Rust that isolates Razorpay API credentials from autonomous AI agents and enforces deterministic financial invariants before money moves.

Built for the [Razorpay AI Buildathon 2026](https://razorpay.com/buildathon/) — Track 02: AI Risk Manager.

[![CI](https://github.com/theoxfaber/agentic-payment-risk-governor/actions/workflows/ci.yml/badge.svg)](https://github.com/theoxfaber/agentic-payment-risk-governor/actions/workflows/ci.yml)
![Rust](https://img.shields.io/badge/Rust-workspace-dea584?logo=rust)
![Tests](https://img.shields.io/badge/tests-201%20passing-1a7f37)
![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)

---

## The Problem: Unsandboxed Agent Execution

Giving an LLM or autonomous agent direct access to gateway keys (`rzp_live_*` / `rzp_test_*`) causes four failure modes:

1. **Unbounded mutation loops.** Retries on timeouts create duplicate refunds and double charges.
2. **State blindness.** Agents refund uncaptured, failed, or already-refunded payments.
3. **Floating-point rounding.** Amounts like `14.99` produce fractional paise and reconciliation drift.
4. **Credential exposure.** Prompt injection can exfiltrate raw keys from memory or context.

Risk Governor closes these at the proxy boundary. Agents never see the upstream secret.

---

## Architecture

The Governor is an HTTP sidecar between the agent and Razorpay. It holds the upstream keys; the agent holds only an internal token.

```
+------------------+         HTTP (internal token)        +----------------------------------+
|                  | ----------------------------------> |      GOVERNOR SERVER (Rust)      |
| Autonomous Agent |                                      |  • subtle constant-time auth     |
|   (LLM / MCP)    | <---------------------------------- |  • integer paise balance gates   |
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

**Request flow:** `action_requested → policy_evaluated → risk_scored → graph_analyzed → decision_made → human_reviewed → razorpay_called`

Four planes vote before the combiner decides **ALLOW / REVIEW / BLOCK**:

| Plane | Checks | On fail |
| --- | --- | --- |
| Policy | Caps, velocity, country, payment state, paise balance | Hard BLOCK |
| Risk | Z-scores (true stddev), drift, agent history, intent mismatch | Score 0–1 |
| Graph | Union-find over shared device/address/instrument | Cluster |
| Investigation | For vs. against evidence, confidence | REVIEW when unsure |

High risk with contradictory or thin evidence never auto-blocks — it goes to human review. Degraded evidence services force REVIEW, not silent allow.

---

## Enforced Invariants

All invariants are checked in code and pinned by tests.

* **Zero credential exposure.** Upstream secrets never leave the Governor process. Dashboard is unauthenticated and carries no key.
* **Integer paise.** All amounts are positive `i64` paise (effectively `u64`). Floats are rejected at validation (400, not 500).
* **Balance bounds.** Refund is rejected unless `payment_state == "captured"` and `amount ≤ captured_paise − refunded_paise` (checked subtraction). Over-refund is a policy BLOCK.
* **At-most-once.** Deterministic keys `rfnd_{payment_id}_{decision_id}` / `pout_{merchant}_{decision_id}` are sent as `Idempotency-Key` on every gateway POST and cached per `decision_id`. Concurrent duplicates return the cached result.
* **Tamper-evident log.** Canonical JSON (sorted keys) → SHA-256 `previous_hash → current_hash` chain. `deny_unknown_fields` rejects key-injection payloads.
* **Constant-time auth.** Inbound keys validated with `subtle::ConstantTimeEq` (`a.ct_eq(b).unwrap_u8() == 1`), not early-return string compare.

---

## Quickstart

Prerequisites: Rust 1.78+

```bash
git clone https://github.com/theoxfaber/agentic-payment-risk-governor
cd agentic-payment-risk-governor

# all tests offline, no credentials
cargo test --workspace

# start the governor + dashboard
cargo run --release -p governor-server -- --port 8080
# → http://127.0.0.1:8080        dashboard
# → http://127.0.0.1:8080/metrics  Prometheus
```

One-command demo (no keys, no network) — ALLOW → REVIEW → BLOCK → payout → held-out numbers:

```bash
./demo.sh
```

Wire any agent via MCP:

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

Tools: `check_action` → verdict + reasoning · `get_decision` → full replay · `list_reviews` → queue

---

## API

### `POST /v1/actions`

Headers: `Content-Type: application/json`, `X-API-Key: <GOVERNOR_API_KEY>` (or `Authorization: Bearer`)

Refund request — all amounts are integer paise:

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

Approved (ALLOW) — executed and logged:

```json
{
  "decision": "allow",
  "decision_id": "dec_01HZ89ABC",
  "policy_result": { "verdict": "allow", "matched_rules": [] },
  "risk_result": { "risk_score": 0.12, "intent_mismatch_score": 0.0 },
  "audit_hash": "e3b0c4...b855"
}
```

Rejected — balance or state:

```json
{
  "error": "refund amount 90000 exceeds available balance 80000 (captured 100000 - refunded 20000)",
  "status": 403
}
```

Other endpoints: `GET /v1/decisions`, `GET /v1/decisions/{id}` (full replay), `POST /v1/decisions/{id}/approve`, `GET /health`, `GET /metrics` (open), `GET /dashboard`.

---

## Results — Held-Out

Thresholds tuned on calibration worlds (seed 2026). Measured on three held-out seeds never seen during development.

| Approach | Precision | Recall | FP cost | FN cost | Prevented |
| --- | --- | --- | --- | --- | --- |
| Per-customer rate | 100% | 66% | ₹0 | ₹34,650 | ₹1,68,300 |
| Clustering only | 51% | 100% | ₹65,925 | ₹0 | ₹2,02,950 |
| **Investigation engine** | **100%** | **100%** | **₹0** | **₹0** | **₹2,02,950** |
| Learned logistic (Rust) | 100% | 100% | ₹0 | ₹0 | ₹2,02,950 |
| LR + conformal economics | 100% | 94% | ₹0 | ₹1,350 | ₹2,01,600 |

Last row is intentional: auto-allow only when `p̂ × exposure ≤ ₹400` and auto-block via Conformal Risk Control (fraud-leak ≤ 2%, friction ≤ 1%). Conceding ₹1,350 is cheaper than reviewer time for ₹13.50 exposures.

False positives on 972 legit customers sharing devices/addresses/NAT IPs — investigation, learned LR, and calibrated LR each flag **0 of 972**.

Degraded data (missing records, jitter, count noise): recall stays 100% while review share climbs 1% → 72%. Random 140 worlds: 100% precision, 99.4% recall. Stress harness with camouflaged abusers validates the conformal budgets at `z < 1` over 12 runs. Full method in [docs/AI_DESIGN.md](docs/AI_DESIGN.md).

---

## Verification & Adversarial Harness

```bash
cargo test --workspace
# 201 tests offline

cargo test --test financial_invariants -- --nocapture
cargo test --test test_adversarial_concurrency -- --nocapture
# 8 concurrent approvers on same REVIEW → exactly one execution
```

| Suite | File | Invariant |
| --- | --- | --- |
| Constant-time auth | `governor-server/src/auth.rs` | `ct_eq` prevents timing leakage |
| Balance bounds | `policy-engine/src/lib.rs` | `captured` gate + `captured−refunded` check |
| Idempotency | `razorpay-gateway/src/lib.rs` | `rfnd_{payment_id}_{decision_id}` header + decision cache |
| Concurrent approval | `governor-server/src/routes.rs` | Claim-under-lock: 8-way race → 1 execution |
| Adversarial concurrency | `governor/tests/test_adversarial_concurrency.rs` | Parallel webhooks, duplicate dispatches |
| Audit chain | `risk-governor-types/src/lib.rs` | SHA-256 `previous_hash → current_hash` integrity |

Reproduce everything a judge can:

```bash
cargo run --release -p eval-harness
EVAL_HELDOUT_SEEDS=12345,67890 cargo run --release -p eval-harness
docker compose up -d && cargo run -p governor --bin distributed_demo
RAZORPAY_KEY_ID=rzp_test_... RAZORPAY_KEY_SECRET=... cargo run -p razorpay-gateway --bin rzp_smoke
```

---

## Performance

Measured on Linux, release build, loopback, 1,000 iterations:

| Metric | Value | Notes |
| --- | --- | --- |
| P50 policy latency | 0.42 ms | Auth + validation + policy |
| P99 policy latency | 1.18 ms | Including SHA-256 audit chain |
| Throughput floor | 12,500 req/s | Single core |
| RSS baseline | ~18 MB |  |

---

## Configuration

| Variable | Purpose |
| --- | --- |
| `GOVERNOR_API_KEY` | Required on `/v1/*` (ephemeral generated if unset, redacted in logs) |
| `RAZORPAY_KEY_ID` / `RAZORPAY_KEY_SECRET` | Live test-mode gateway (mock if unset) |
| `DATABASE_URL` | Postgres persistence (memory if unset) |
| `NATS_URL` | Distributed bus |
| `LLM_API_KEY` / `LLM_BASE_URL` / `LLM_MODEL` | LLM extraction (heuristic if unset, hardened + evidence-only) |
| `WEBHOOK_SECRET` | HMAC-SHA256 for `X-Razorpay-Signature` |
| `SEED_DEMO` | With Postgres, `true`/`1` to seed demo entities |
| `SCORE_REFERENCE_JSON` | 5-bucket reference for PSI drift at `/metrics` |

See [.env.example](.env.example).

---

## Production Deployment

Run as a sidecar / reverse proxy in front of Razorpay credentials. Agents call the Governor; the Governor calls Razorpay. No code path bypasses the policy gate.

Roadmap: label maturation (webhooks → `OutcomeRecorded` → nightly retrain gated by PSI), dispute responder `draft → submit`, single-packet velocity races, segment-level conformal recalibration.

Docs: [AI design](docs/AI_DESIGN.md) · [Bugs & fixes](docs/BUGS.md) · [Testing](docs/TESTING.md)

License: MIT OR Apache-2.0
