# Risk Governor

**A safety and governance layer for autonomous financial agents.**

> AI agents can now execute financial actions — refunds, payouts, orders — on
> payment platforms like Razorpay. That creates a new authorization gap:
> **valid API credentials don't mean a valid financial action.** Risk Governor
> is a decision layer that scores every agent-initiated action *before* it
> reaches an execution API, and returns ALLOW / REVIEW / BLOCK.

This is not a fraud rules engine. It sits above/beside existing fraud models
and answers a different question: *should this autonomous agent be allowed to
move this money, right now?*

---

## Why now

Razorpay has called 2026 the "Age of Agentic Payments": AI agents that initiate
payouts, a CLI built for the AI-agent era, engineering teams building
infrastructure so AI agents can transact securely at scale. When software
agents hold live credentials and act while humans sleep, credential validity
stops being the security boundary. Action validity does.

Risk Governor is built for exactly that gap.

## Architecture

```text
AI Agent (refund/payout request)
   │
   ▼
┌─────────────────────────────────────────────┐
│                RISK GOVERNOR                │
│                                             │
│  Policy Engine ── hard boundaries           │
│  Risk Engine ──── scoring                   │
│  Entity Graph ─── coordinated-abuse structure│
│  Investigation Engine ─ evidence reasoning  │
│  Evidence Service ─ agent history + quality │
│  Audit Service ── full decision trail       │
└─────────────────────────────────────────────┘
   │
   ▼
ALLOW ──────► executes against Razorpay test-mode API
REVIEW ─────► held for human approval endpoint
BLOCK ─────► never touches the API
```

Every stage runs per-action; every decision is logged with its full feature
breakdown so any ALLOW/REVIEW/BLOCK can be replayed and explained after the
fact.

## The core safety idea

**High risk cannot automatically mean BLOCK when the evidence is
contradictory or low-confidence.**

The investigation engine weighs evidence for and against abuse
(`Direction::{Supports, Contradicts}`), computes an `evidence_confidence`
score, and escalates to human review when a high risk score lacks behavioral
confirmation. A false BLOCK is not a safe failure — it's money movement
denied to a legitimate user. The system is engineered to be *sure before it
acts*, and humble when it can't be sure.

## Evaluation

Synthetic labeled dataset across adversarial worlds (coincidental sharing,
merchant collusion, adaptive evasion). Aggregate over all abuse worlds:

| Approach | Precision | Recall | FP cost | FN cost | Prevented |
|---|---|---|---|---|---|
| Rules only (per-customer rate) | 100% | 58% | ₹0 | ₹11,700 | ₹46,800 |
| Structural clustering only | 51% | 100% | ₹22,950 | ₹0 | ₹58,500 |
| **Investigation engine (hybrid)** | **100%** | **100%** | **₹0** | **₹0** | **₹58,500** |

False-positive check on an all-legitimate household world: rules-only and
clustering-only each flag innocent customers; the investigation engine flags
0 of 324 legitimate customers (₹0 friction cost).

Read honestly: rules-only misses 42% of abuse; clustering-only burns real
money on false positives. The reasoning layer gets both because it refuses to
act on structure alone — it demands behavioral confirmation first.

## Repo layout

Single-thesis Rust workspace; each crate is a clean module of one pipeline:

| Crate | Role |
|---|---|
| `action-service` | Pipeline orchestrator: policy → risk → evidence → decision → gateway |
| `policy-engine` | Hard boundaries (thresholds, scope) |
| `risk-engine` | Behavioral scoring |
| `risk-graph` / `risk-governor-correlation` | Entity graph + coordinated-abuse structure |
| `investigation-engine` | Evidence reasoning: confidence, contradiction, verdicts |
| `evidence-service` | Agent history store |
| `audit-service` / `risk-governor-replay` | Decision trail + replay |
| `dashboard` | One-page live decision stream, replay viewer, human-approval UI (vanilla JS, no build step) |
| `razorpay-gateway` | Test-mode HTTP client (basic auth, retry/backoff on 429/5xx) + mock for offline runs |
| `nats-link` | Distributed mode (pipeline split across processes via NATS) |
| `dataset-gen` / `eval-harness` | Labeled synthetic worlds + precision/recall/cost eval |
| `governor-server` | Unified axum binary: decision API + replay + approval + dashboard at `/` |
| `webhook-consumer`, `dashboard`, `evaluation-service`, `infra-probe` | Supporting services |

## Run it

### 1. The API server + live dashboard (flagship)

```bash
cargo run -p governor-server --bin governor-server
# → open http://127.0.0.1:8080
```

The dashboard at the root URL is a real-time view of the governor in action:
a live decision stream (polls every 2s), aggregate stats including **blocked
value prevented**, and click-through to full decision replay — what the
governor saw, why it decided, the complete audit-trail timeline. REVIEW
decisions surface an inline human-approval box; approving executes the held
action against the gateway, right there.

Drive it from another terminal (or watch the stream fill up):

```bash
# Normal refund → ALLOW
curl -s -X POST localhost:8080/v1/actions \
  -H 'content-type: application/json' \
  -d '{"agent_id":"agent-trusted-01","merchant_id":"merchant-001",
       "action_type":"refund","amount":50000,
       "declared_intent":"refund for order #123"}'

# Above approval threshold → REVIEW (approve it in the dashboard)
curl -s -X POST localhost:8080/v1/actions \
  -H 'content-type: application/json' \
  -d '{"agent_id":"agent-trusted-01","merchant_id":"merchant-001",
       "action_type":"refund","amount":150000,
       "declared_intent":"refund for order #456"}'

# Over hard cap → BLOCK
curl -s -X POST localhost:8080/v1/actions \
  -H 'content-type: application/json' \
  -d '{"agent_id":"agent-trusted-01","merchant_id":"merchant-001",
       "action_type":"refund","amount":600000,
       "declared_intent":"refund order #789"}'
```

Gateway auto-selects: set `RAZORPAY_KEY_ID`/`RAZORPAY_KEY_SECRET` for live
test-mode calls, otherwise a mock gateway records intent and moves no money.

The audit trail for one decision reads as the full lifecycle:

```
action_requested → policy_evaluated → risk_scored → graph_analyzed
→ decision_made → human_reviewed → razorpay_called
```

### 2. Tests, eval, demos

```bash
# Tests (full workspace)
cargo test --workspace

# Eval: baselines vs investigation engine across abuse worlds
cargo run --release -p eval-harness

# In-process demo pipeline (4 scripted scenarios)
cargo run -p governor --bin governor

# Distributed demo — requires infra first:
docker compose up -d        # NATS + Postgres + policy/evidence workers
cargo run -p governor --bin distributed_demo

# Live Razorpay test-mode smoke test (needs keys)
RAZORPAY_KEY_ID=rzp_test_... RAZORPAY_KEY_SECRET=... cargo run -p razorpay-gateway --bin rzp_smoke
```

## What's real vs simulated

Said plainly, because credibility matters more than impressions:

- **Real:** the decision pipeline, entity graph, investigation/reasoning
  logic, audit/replay, NATS distribution, chaos tests, and live Razorpay
  test-mode API calls (order creation, refunds, retry handling).
- **Simulated:** the agent-behavior dataset is synthetic and labeled by
  construction. No claim of access to Razorpay's production risk systems or
  internal models.
- **Not claimed:** production integration, Vulcan access, or that the ML here
  replaces platform-side fraud models — this layer composes with them.

## Future work

With more time: payout scenarios as a first-class path alongside refunds,
LLM-assisted extraction of unstructured declared intent (never as the
decision-maker itself), single-packet race testing for velocity controls, and
broader detection classes (OAuth flows, deserialization, request smuggling)
at the gateway boundary.
