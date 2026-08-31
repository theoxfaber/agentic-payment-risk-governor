<p align="center">
  <h1 align="center">Agentic Payment Risk Governor</h1>
  <p align="center">
    <strong>Razorpay AI Buildathon 2026 — Track 02: AI Risk Manager (Defense-Only)</strong><br>
    A safety gateway for AI agents that touch money. No key leaves the gateway.
  </p>
  <p align="center">
    <a href="https://github.com/theoxfaber/agentic-payment-risk-governor/actions/workflows/ci.yml"><img src="https://github.com/theoxfaber/agentic-payment-risk-governor/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
    <img src="https://img.shields.io/badge/Rust-workspace-dea584?logo=rust" alt="Rust">
    <img src="https://img.shields.io/badge/tests-205%20passing-1a7f37" alt="Tests">
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="License"></a><br>
    <a href="https://dashboard-v2-two-steel.vercel.app"><img src="https://img.shields.io/badge/demo-live%20on%20Vercel-black?logo=vercel" alt="Vercel"></a>
    <em>Live frontend → <a href="https://dashboard-v2-two-steel.vercel.app">dashboard-v2-two-steel.vercel.app</a> · Backend: <code>cargo run -p governor-server</code> serves the same build at <code>/</code></em>
  </p>
</p>

> **In one sentence:** An AI assistant wants to refund money. It has to ask this gateway first. The gateway checks the request, decides, and writes down why — then says ALLOW, REVIEW, or BLOCK.

---

### The idea in 2 minutes

Imagine you let an AI handle refunds. If it can call your payment system directly, one mistake could send real money the wrong way.

This gateway sits in between. The agent never holds your Razorpay key. It asks in plain words: “I want to refund ₹50 for order #123.” The gateway checks, decides, and records every step so you can see why later.

**How it decides:**

1. The agent asks — it describes what it wants, never touching money.
2. The gateway checks — is the payment real and is there enough left? Does the amount or frequency look odd? Are these customers linked like a fraud ring? Does the story match the numbers? If anything is missing or strange, it says “no” or “let a person check”.
3. Money moves once, with a paper trail — if the answer is yes, the gateway makes the payment and you can replay the whole decision.

Try it without code: open `http://127.0.0.1:8080` → **New Action** → try a small trusted refund (ALLOW), a large one (REVIEW → Approve), and one blocked by the limit.

*Defense-only: it can only stop or ask a human, never help an attack. That’s the Track 02 bar.*

<details>
<summary><strong>Technical details for engineers — click to expand</strong></summary>

<br>

**Architecture**

```
Agent (no keys) -- X-API-Key --> Governor Server (Rust) -- rzp_test_* --> Razorpay API
                                  ├─ login check (constant-time)
                                  ├─ money checks: integer paise, captured? balance left? (missing → BLOCK)
                                  ├─ risk + graph + investigation in parallel → learned p̂ gates before money moves
                                  ├─ hash chain of every step
                                  └─ at-most-once: unique key + pending claim + receipt probe
```

The system checks the payment, scores how unusual it is, finds linked accounts, and weighs evidence for and against. Contradiction or thin evidence goes to a human.

| Plane | Checks | If it fails |
|---|---|---|
| Policy | max refund, velocity, country, `captured`, `amount ≤ captured − refunded` | BLOCK |
| Risk | amount/velocity weirdness, drift, story vs numbers | score 0–1 |
| Graph | linked by device / address / card | cluster |
| Investigation | for / against, household defense, confidence | REVIEW |

Board: [`docs/architecture.excalidraw`](docs/architecture.excalidraw) — patterns borrowed from fraud systems, see [`docs/adasl.yaml`](docs/adasl.yaml) and [`docs/RBI_RBA.md`](docs/RBI_RBA.md).

**Guarantees**

| Guarantee | How | Test |
|---|---|---|
| No key leak | `GOVERNOR_API_KEY` only on server, dashboard via `sessionStorage` | `governor-server` |
| Paise-safe | `i64` paise, no floats | `policy-engine` |
| Balance safe | `captured` gate + `captured − refunded`, missing → BLOCK | `missing_payment_state_fails_closed` |
| At-most-once | `rfnd_{pay}_{decision}` + pending claim + receipt check | `razorpay-gateway` |
| Tamper-evident | sorted JSON → `SHA-256` chain | `audit-service` |
| One approval wins | `REVIEW` removed under lock before gateway | `concurrent_approvals_execute_exactly_once` |
| Latency | `p95 < 180ms` histogram at `/metrics` | prometheus |
| Explainable | per-feature SHAP `weight*(x-mean)/std` | `action-service/learned` |

**API**

Headers: `X-API-Key: <key>` or `Authorization: Bearer`

```bash
curl -H 'X-API-Key: demo123' -H 'Content-Type: application/json' -d '{
  "agent_id":"agent-trusted-01","merchant_id":"merchant-001","action_type":"refund",
  "amount":5000,"currency":"INR","declared_intent":"refund order #123",
  "context":{"payment_id":"pay_O9xK8w7e5Y1Z2a","payment_state":"captured","captured_paise":100000,"refunded_paise":20000}
}' http://127.0.0.1:8080/v1/actions
```

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `POST` | `/v1/actions` | yes | `ALLOW` / `REVIEW` / `BLOCK` + `learned_insight` |
| `GET` | `/v1/decisions` | yes | list |
| `GET` | `/v1/decisions/{id}` | yes | replay + audit trail |
| `POST` | `/v1/decisions/{id}/approve` | yes | human allow/block, once |
| `GET` | `/v1/audit/verify` · `/v1/audit/anchor` | yes | chain + HMAC |
| `GET` | `/v1/real/analysis?count=20` | yes | **Real Razorpay test data** — live `GET /v1/payments` + risk flags (requires `RAZORPAY_KEY_ID/SECRET`, replaces synthetic for demo) |
| `POST` | `/webhooks/razorpay` | no* | HMAC verified (*`WEBHOOK_SECRET`) |
| `GET` | `/health` · `/metrics` · `/` | no | liveness, Prometheus, console |

**Verification**

*Live smoke (test-mode):* `auth OK + order_TUyv0Ib1swX7ki`; `POST /payments/create/json` `404` (deprecated) → refund path covered by gateway tests. Repro: `RAZORPAY_KEY_ID=rzp_test_... cargo run -p razorpay-gateway --bin rzp_smoke`

*Offline:* `cargo test --workspace`, `financial_invariants`, `test_adversarial_concurrency` (8-way → one winner), `learned_escalation_blocks_before_gateway`. Coverage `cargo llvm-cov --workspace --fail-under-lines 60`.

*Held-out (synthetic):* calibration `2026`, held-out `31415/27182/16180`

| Approach | Precision | Recall | FP cost | Prevented |
|---|---:|---:|---:|---:|
| Per-customer | 100% | 66% | ₹0 | ₹1,68,300 |
| Clustering only | 51% | 100% | ₹65,925 | ₹2,02,950 |
| **Investigation / Learned** | **100%** | **100%** | **₹0** | **₹2,02,950** |
| Calibrated LR | 100% | 94% | ₹0 | ₹2,01,600 |

`972` households `0` flagged; `140` random worlds `100%/99.4%`. Full report: `docs/EVAL_REPORT_2026-08-28.md` → `cargo run -p eval-harness`.

</details>

---

### Quickstart

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

### Configuration

| Variable | Use |
|---|---|
| `GOVERNOR_API_KEY` | `/v1/*` key (ephemeral if unset) |
| `RAZORPAY_KEY_ID` / `SECRET` | live test-mode (mock if unset) |
| `DATABASE_URL` | Postgres (memory if unset) |
| `NATS_URL` | bus (in-process if unset) |
| `LLM_API_KEY` / `BASE_URL` / `MODEL` | intent claims, evidence-only |
| `WEBHOOK_SECRET` | `X-Razorpay-Signature` (constant-time) |
| `AUDIT_SIGNING_KEY` | HMAC of chain head (`docs/BUGS.md` #17) |
| `SEED_DEMO` | `true` to seed demo on Postgres |
| `SCORE_REFERENCE_JSON` | 5-bucket PSI reference |

See [`.env.example`](.env.example).

---

### Project Layout

```
governor-server/  axum + auth + learned scorer + dashboard
policy-engine/    caps, balance, velocity, country, RBI RiskTier
risk-engine/      z-scores, drift, intent mismatch + CEP typologies
risk-graph/       union-find (device → cluster)
investigation-engine/  for/against, confidence
razorpay-gateway/      at-most-once + receipt probe
eval-harness/          LR + CRC + report
dashboard-v2/          React 19 + TanStack Query
governor/              orchestration + e2e tests
```

Docs: [`DEMO.md`](DEMO.md) · [`docs/AI_DESIGN.md`](docs/AI_DESIGN.md) · [`docs/BUGS.md`](docs/BUGS.md) · [`docs/RBI_RBA.md`](docs/RBI_RBA.md) · [`docs/adasl.yaml`](docs/adasl.yaml) · [`BENCHMARK.md`](BENCHMARK.md)

---

### License

Dual-licensed: [`MIT`](LICENSE-MIT) OR [`Apache-2.0`](LICENSE-APACHE) — see [LICENSE](LICENSE). You may choose either.

