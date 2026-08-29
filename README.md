# Agentic Payment Risk Governor

Defense-only verifier for autonomous refund abuse. Agents never hold the Razorpay secret — they post an intent, the governor decides `ALLOW / REVIEW / BLOCK` before money moves.

[![CI](https://github.com/theoxfaber/agentic-payment-risk-governor/actions/workflows/ci.yml/badge.svg)](https://github.com/theoxfaber/agentic-payment-risk-governor/actions/workflows/ci.yml)
![Rust](https://img.shields.io/badge/Rust-workspace-dea584?logo=rust)
![Tests](https://img.shields.io/badge/tests-209%20passing-1a7f37)
![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)

> **Scope:** `POST /v1/actions → ALLOW / REVIEW / BLOCK → at-most-once Razorpay → audited`. Live is the deterministic verifier + risk features with learned `p̂` + conformal band **gating before execution** (`p̂·amount > ₹400` escalates `ALLOW→REVIEW/BLOCK` before `DecisionMade`/gateway; see `docs/BUGS.md` #19). Synthetic held-out data proves the machinery — see `docs/AI_DESIGN.md` §5.

---

## Quickstart

```bash
git clone https://github.com/theoxfaber/agentic-payment-risk-governor
cd agentic-payment-risk-governor

cargo test --workspace          # 209 tests, offline, no credentials
cargo run --release -p governor-server -- --port 8080
# http://127.0.0.1:8080         triage console
# http://127.0.0.1:8080/metrics  Prometheus

./demo.sh                       # ALLOW → REVIEW (+ approve) → BLOCK → payout, with replay
```

MCP for any agent:

```json
{ "mcpServers": { "risk-governor": { "command": "cargo", "args": ["run","-p","mcp-server"], "env": { "GOVERNOR_URL": "http://127.0.0.1:8080", "GOVERNOR_API_KEY": "<key>" } } } }
```

`check_action` · `get_decision` · `list_reviews`

---

## Architecture

```
Agent (no keys) -- HTTP + X-API-Key --> Governor Server (Rust) -- HTTPS + rzp_test_* --> Razorpay API
                                         ├─ constant-time auth (subtle ct_eq)
                                         ├─ integer paise + captured-state + balance (fail-closed)
                                         ├─ risk + graph + investigation → combiner → learned p̂ + tau_clear/block (gate before DecisionMade/gateway, HMAC-anchored)
                                         ├─ SHA-256 previous_hash → current_hash (canonical JSON, /v1/audit/verify)
                                         └─ rfnd_{pay}_{decision} idempotency + _pending claim + receipt probe
```

`action_requested → policy_evaluated → risk_scored → graph_analyzed → decision_made (+ learned_insight) → human_reviewed → razorpay_called`

| Plane | Checks | On failure |
|---|---|---|
| Policy | `max_refund`, velocity, country, `payment_state == captured`, `amount ≤ captured − refunded` (checked) | `BLOCK` (missing `payment_state`/`captured` also `BLOCK`) |
| Risk | z-scores, drift (PSI), `intent_mismatch` | score `0–1` |
| Graph | union-find on device / address / instrument | cluster |
| Investigation | for / against, confidence | `REVIEW` on conflict or thin evidence |

High-risk + contradiction → human. Degraded evidence → `REVIEW`.

Visual board: [`docs/architecture.excalidraw`](docs/architecture.excalidraw) — simple top flow for non-technical, deep bottom layer for engineers (invariants, CRC, audit chain). Open in https://excalidraw.com.

---

## Invariants

| Guarantee | How | Tests |
|---|---|---|
| No credential exposure — dashboard unauthenticated, no key in HTML | `GOVERNOR_API_KEY` in server only, `sessionStorage` in browser | `governor-server` |
| Integer paise — `i64` paise, floats `400` | `validate_request` | `policy-engine` |
| Balance — `captured` gate + `captured − refunded` checked, missing fields fail-closed | `policy-engine::evaluate` | `missing_payment_state_fails_closed` |
| At-most-once — `rfnd_{pay}_{decision}` + per-decision cache + `receipt == decision_id` probe | `HttpGateway::execute` | `razorpay-gateway` |
| Tamper-evident — sorted canonical JSON → `SHA-256(previous‖record)` + `deny_unknown_fields` | `canonical_json_bytes` | `audit-service` |
| Constant-time auth — `subtle::ct_eq` | `auth::require_api_key` | `governor-server` |
| One approval wins — 8-way `REVIEW` → 1 execution (claim-under-lock) | `routes::approve_decision` | `concurrent_approvals_execute_exactly_once` |

---

## API

Headers: `X-API-Key: <GOVERNOR_API_KEY>` or `Authorization: Bearer`

```bash
curl -H 'X-API-Key: demo123' -H 'Content-Type: application/json' -d '{
  "agent_id":"agent-trusted-01","merchant_id":"merchant-001","action_type":"refund",
  "amount":5000,"currency":"INR","declared_intent":"refund order #123",
  "context":{"payment_id":"pay_O9xK8w7e5Y1Z2a","payment_state":"captured","captured_paise":100000,"refunded_paise":20000}
}' http://127.0.0.1:8080/v1/actions
```

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `POST` | `/v1/actions` | yes | `ALLOW`/`REVIEW`/`BLOCK` + `learned_insight` |
| `GET` | `/v1/decisions` | yes | list (adds `learned_p_hat`/`band`) |
| `GET` | `/v1/decisions/{id}` | yes | full replay + `audit_trail` + `audit_verified` + `audit_anchor` |
| `POST` | `/v1/decisions/{id}/approve` | yes | human `allow`/`block`, exactly once |
| `GET` | `/v1/audit/verify` + `/v1/audit/anchor` | yes | hash-chain verification + HMAC head |
| `POST` | `/webhooks/razorpay` | no* | HMAC-verified, audited (`*requires WEBHOOK_SECRET`) |
| `GET` | `/health` `/metrics` `/` | no | liveness, Prometheus, console |

---

## Verification

**Live smoke (test-mode, partial):** `auth OK + order_TUyv0Ib1swX7ki` live; `/payments/create/json` is deprecated `404` → refund path covered by `HttpGateway` tests (idempotency, receipt probe). Reproduce: `RAZORPAY_KEY_ID=rzp_test_... cargo run -p razorpay-gateway --bin rzp_smoke`

**Offline suite:** `cargo test --workspace` (209), `cargo test --test financial_invariants`, `cargo test --test test_adversarial_concurrency` (8-way race → 1), `learned_escalation_blocks_before_gateway` (0 calls on BLOCK). Coverage `cargo llvm-cov --workspace --fail-under-lines 60`.

**Held-out (synthetic, machinery check):** calibration `2026`, held-out `31415/27182/16180`.

| Approach | Precision | Recall | FP cost | Prevented |
|---|---:|---:|---:|---:|
| Per-customer | 100% | 66% | ₹0 | ₹1,68,300 |
| Clustering only | 51% | 100% | ₹65,925 | ₹2,02,950 |
| **Investigation / Learned** | **100%** | **100%** | **₹0** | **₹2,02,950** |
| Calibrated LR | 100% | 94% | ₹0 | ₹2,01,600 |

`972` households `0` flagged; degradation `1%→59%` review; `140` random worlds `100%/99.4%`; camouflage 12 runs `z=-1.16/0.95`. Source: `cargo run --release -p eval-harness` → `docs/EVAL_REPORT_2026-08-28.md` + `BENCHMARK.md`.

---

## Configuration

| Variable | Use |
|---|---|
| `GOVERNOR_API_KEY` | `/v1/*` key (ephemeral if unset, redacted) |
| `RAZORPAY_KEY_ID` / `SECRET` | live test-mode (mock if unset) |
| `DATABASE_URL` | Postgres (memory if unset) |
| `NATS_URL` | bus (in-process if unset) |
| `LLM_API_KEY` / `BASE_URL` / `MODEL` | intent claims, evidence-only, hardened |
| `WEBHOOK_SECRET` | `X-Razorpay-Signature` (ct `verify_slice`) — also enables `POST /webhooks/razorpay` |
| `AUDIT_SIGNING_KEY` | HMAC-SHA256 of chain head for `/v1/audit/verify` + `/v1/audit/anchor` (see `docs/BUGS.md` #17) |
| `SEED_DEMO` | `true` to seed demo on Postgres |
| `SCORE_REFERENCE_JSON` | 5-bucket PSI reference |

See `.env.example`.

---

## Layout

```
governor-server/  axum + auth + learned scorer + ServeDir dashboard-v2/dist
policy-engine/    caps, balance (fail-closed), velocity, country
risk-engine/      z-scores, drift, intent mismatch
risk-graph/       union-find
investigation-engine/ for/against, combiner
razorpay-gateway/ HttpGateway (retry, idempotency, receipt probe) + MockGateway
eval-harness/     LR (lr-1.0.0-calib-0.1.0, tau 0.2345/0.99995) + CRC + report
dashboard-v2/     Vite + React 19 + Tailwind + TanStack Query (triage console)
governor/         pipeline orchestration + e2e tests
```

Docs: `DEMO.md` (10-min judge path) · `docs/AI_DESIGN.md` · `docs/BUGS.md` (#15 balance fail-closed) · `docs/TESTING.md` · `BENCHMARK.md` · `docs/PITCH_SCRIPT.md` · `docs/architecture.excalidraw`

License: MIT OR Apache-2.0
