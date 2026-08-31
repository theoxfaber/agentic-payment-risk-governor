# Razorpay AI Buildathon 2026 — Track 02: AI Risk Manager (Defense-Only) — Agentic Payment Risk Governor

[![CI](https://github.com/theoxfaber/agentic-payment-risk-governor/actions/workflows/ci.yml/badge.svg)](https://github.com/theoxfaber/agentic-payment-risk-governor/actions/workflows/ci.yml)
![Rust](https://img.shields.io/badge/Rust-workspace-dea584?logo=rust)
![Tests](https://img.shields.io/badge/tests-205%20passing-1a7f37)
![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)

> **One line for everyone:** An AI agent wants to refund money. It must ask this safety gateway first. The gateway checks the rules, the risk, and whether the customers are linked — then says `ALLOW` (pay), `REVIEW` (human checks), or `BLOCK` (no). No secret ever leaves the gateway.

---

## For non-engineers — 2 minutes

Imagine you let an AI assistant handle refunds. If it can call your payment system directly, a single mistake or tricked instruction could send real money the wrong way.

This gateway sits in between. The agent never holds your Razorpay key. It has to ask first, in plain words: “I want to refund ₹50 for order #123.” The gateway checks the request, decides, and writes down why.

How it decides, in three steps:

1. **The agent asks, it doesn’t act.** It describes what it wants to do, never touching money itself.
2. **The gateway checks.** Is the payment real and is there enough money left to refund? Does the amount or frequency look unusual? Are these customers linked together in a way that looks like a fraud ring? Does the agent’s story match the numbers? If anything is missing or odd, it says “no” or “let a person check” — it never quietly says “yes”.
3. **Money only moves once, and you can see why.** If the answer is yes, the gateway makes the payment and records every step so you can replay the decision later.

You can try it without code: open the dashboard, try a small trusted refund, a large one that needs approval, and one that is blocked.

> This is defense-only: it can only stop or ask a human, never help an attack. That’s the requirement for Track 02.

Technical details below — the next sections are for engineers and can be as dense as needed.

---

## Quickstart (engineers)

```bash
git clone https://github.com/theoxfaber/agentic-payment-risk-governor
cd agentic-payment-risk-governor

cargo test --workspace          # offline, no credentials
cargo run --release -p governor-server -- --port 8080
# http://127.0.0.1:8080         triage console
# http://127.0.0.1:8080/metrics  Prometheus

./demo.sh                       # runs the three cases with replay
```

MCP for any agent:

```json
{ "mcpServers": { "risk-governor": { "command": "cargo", "args": ["run","-p","mcp-server"], "env": { "GOVERNOR_URL": "http://127.0.0.1:8080", "GOVERNOR_API_KEY": "<key>" } } } }
```

`check_action` · `get_decision` · `list_reviews`

---

## Architecture (still the same system, easier words)

```
Agent (no keys) -- HTTP + X-API-Key --> Governor Server (Rust) -- HTTPS + rzp_test_* --> Razorpay API
                                         ├─ login check that doesn’t leak timing (constant-time)
                                         ├─ money checks: integer paise + “is captured?” + “balance left?” (if missing → BLOCK)
                                         ├─ risk + graph + investigation run in parallel → learned p̂ + tau_clear/block (gates BEFORE money moves)
                                         ├─ SHA-256 chain of every step (see /v1/audit/verify)
                                         └─ at-most-once: unique key + “was I pending?” + receipt probe
```

The system checks the payment, scores how unusual it looks, looks for linked accounts, and investigates the evidence. If anything is missing or contradictory it asks a human instead of quietly allowing.

| Plane | What it checks | If it fails |
|---|---|---|
| Policy | max refund, velocity, country, `payment_state == captured`, `amount ≤ captured − refunded` | `BLOCK` |
| Risk | how weird is amount/velocity, drift, does story match numbers | score 0–1 |
| Graph | are customers linked by device / address / card | cluster |
| Investigation | evidence for vs against (household defense), confidence | `REVIEW` if conflict/thin |

Visual board: [`docs/architecture.excalidraw`](docs/architecture.excalidraw) — top flow for non-technical, bottom layer for engineers. Architecture patterns are borrowed from known fraud-detection systems — see `docs/adasl.yaml` and `docs/RBI_RBA.md` for details.

---

## Guarantees (what you can trust)

| Guarantee | How it works | Test that proves it |
|---|---|---|
| No key leak | `GOVERNOR_API_KEY` only on server, dashboard uses `sessionStorage` | `governor-server` |
| Money in paise (no float bugs) | `i64` paise, `400.00` rejected | `policy-engine` |
| Balance safe | `captured` gate + `captured − refunded`, missing → `BLOCK` | `missing_payment_state_fails_closed` |
| At-most-once | `rfnd_{pay}_{decision}` + `_pending` claim + `receipt == decision_id` probe | `razorpay-gateway` |
| Tamper-evident | sorted JSON → `SHA-256(previous‖record)` + `deny_unknown_fields` | `audit-service` |
| Login safe | constant-time compare | `governor-server` |
| One approval wins | `REVIEW` removed from map under lock before gateway | `concurrent_approvals_execute_exactly_once` (8-way → 1) |
| Latency SLO | `risk_governor_request_duration_ms` histogram, p95 <180ms (Thirdwatch <200ms, ADA <30s, Vulcan <50ms) | `/metrics` |
| Explainability | `learned_insight.contributions` per-feature SHAP (`weight*(x-mean)/std`) | `action-service/learned` |

---

## API (same as before)

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
| `POST` | `/v1/actions` | yes | `ALLOW`/`REVIEW`/`BLOCK` + `learned_insight` (with per-feature contributions) |
| `GET` | `/v1/decisions` | yes | list (with `learned_p_hat`/`band`) |
| `GET` | `/v1/decisions/{id}` | yes | full replay + `audit_trail` + `audit_verified` + `audit_anchor` |
| `POST` | `/v1/decisions/{id}/approve` | yes | human `allow`/`block`, exactly once |
| `GET` | `/v1/audit/verify` + `/v1/audit/anchor` | yes | hash-chain + HMAC head |
| `POST` | `/webhooks/razorpay` | no* | HMAC-verified (`*requires WEBHOOK_SECRET`) |
| `GET` | `/health` `/metrics` `/` | no | liveness, Prometheus, console |

---

## Verification

**Live smoke (test-mode, partial):** `auth OK + order_TUyv0Ib1swX7ki` live; `/payments/create/json` is `404` (deprecated) → refund path covered by `HttpGateway` tests. Reproduce: `RAZORPAY_KEY_ID=rzp_test_... cargo run -p razorpay-gateway --bin rzp_smoke`

**Offline suite:** `cargo test --workspace`, `financial_invariants`, `test_adversarial_concurrency` (8-way race with one winner), `learned_escalation_blocks_before_gateway`. Coverage `cargo llvm-cov --workspace --fail-under-lines 60`.

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
| `GOVERNOR_API_KEY` | `/v1/*` key (ephemeral if unset) |
| `RAZORPAY_KEY_ID` / `SECRET` | live test-mode (mock if unset) |
| `DATABASE_URL` | Postgres (memory if unset) |
| `NATS_URL` | bus (in-process if unset) |
| `LLM_API_KEY` / `BASE_URL` / `MODEL` | intent claims, evidence-only |
| `WEBHOOK_SECRET` | `X-Razorpay-Signature` (constant-time) |
| `AUDIT_SIGNING_KEY` | HMAC-SHA256 of chain head (`docs/BUGS.md` #17, `docs/RBI_RBA.md`) |
| `SEED_DEMO` | `true` to seed demo on Postgres |
| `SCORE_REFERENCE_JSON` | 5-bucket PSI reference |

See `.env.example`.

---

## Layout

```
governor-server/  axum + auth + learned scorer + ServeDir dashboard-v2/dist
policy-engine/    caps, balance (fail-closed), velocity, country, RBI RiskTier/FRI/PMLA
risk-engine/      z-scores, drift, intent mismatch + card-testing/velocity-spike/RTO CEP
risk-graph/       union-find (device fingerprint → cluster via UsesDevice)
investigation-engine/ for/against, combiner (now with RTO impulse)
razorpay-gateway/ HttpGateway (retry, idempotency, receipt probe) + MockGateway
eval-harness/     LR (lr-1.0.0-calib-0.1.0, tau 0.2345/0.99995) + CRC + report
dashboard-v2/     Vite + React 19 + Tailwind + TanStack Query (threshold slider → live precision/recall)
governor/         pipeline orchestration + e2e tests
```

Docs: `DEMO.md` (10-min judge path) · `docs/AI_DESIGN.md` · `docs/BUGS.md` · `docs/RBI_RBA.md` · `docs/adasl.yaml` (AdaDSL) · `docs/TESTING.md` · `BENCHMARK.md` · `docs/PITCH_SCRIPT.md` · `docs/architecture.excalidraw`

License: MIT OR Apache-2.0
