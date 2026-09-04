# Agentic Payment Risk Governor

<p align="center">
  <strong>Defense-only safety gateway and execution proxy for agent-initiated payments.</strong><br>
  <em>Razorpay AI Buildathon 2026 — Track 02: AI Risk Manager</em>
</p>

<p align="center">
  <a href="https://github.com/theoxfaber/agentic-payment-risk-governor/actions/workflows/ci.yml"><img src="https://github.com/theoxfaber/agentic-payment-risk-governor/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <img src="https://img.shields.io/badge/Rust-2021_edition-dea584?logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/tests-205%20passing-1a7f37" alt="Tests">
  <a href="https://dashboard-v2-two-steel.vercel.app"><img src="https://img.shields.io/badge/demo-live_console-black?logo=vercel" alt="Live Demo"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT_OR_Apache--2.0-blue" alt="License"></a>
</p>

---

## Executive Summary

Autonomous agents operating customer support, claims settlement, or programmatic commerce cannot be trusted with direct access to payment gateway API keys. Prompt injection, agent hallucination, or distributed collusion can drain merchant balances within seconds.

The **Agentic Payment Risk Governor** sits as a zero-trust execution proxy between agent runtimes and Razorpay:
- **Zero Credential Exposure:** Agents never receive Razorpay secrets (`rzp_live_*` / `rzp_test_*`). They submit declarative payment intents over authenticated HTTP or Model Context Protocol (MCP).
- **Four Defense-in-Depth Planes:** Every action is evaluated across deterministic financial policy, statistical anomaly detection, sybil graph clustering, and conformal Bayesian risk modeling.
- **Fail-Closed Guarantees:** Integer paise arithmetic prevents float rounding attacks. Unverifiable payment states or non-captured transactions immediately trigger `BLOCK`.
- **At-Most-Once Execution:** Deduplication through composite idempotency keys (`rfnd_{payment_id}_{decision_id}`), pending claim locks, and post-execution receipt probing guarantees payments execute at most once even under adversarial concurrency.

Live Dashboard: **[dashboard-v2-two-steel.vercel.app](https://dashboard-v2-two-steel.vercel.app)** *(or locally at `http://127.0.0.1:8080`)*

---

## Architecture & Evaluation Pipeline

```
  Agent Runtime (No Razorpay Keys)
                │
                │  POST /v1/actions (X-API-Key or Bearer token)
                ▼
┌─────────────────────────────────────────────────────────────┐
│                Governor Server (Rust / Axum)                │
│                                                             │
│  1. Authentication & Integrity                              │
│     └─ Constant-time API key verification (subtle::ct_eq)   │
│     └─ Length-prefixed payload hashing (SHA-256)            │
│                                                             │
│  2. Deterministic Financial Invariants                      │
│     └─ Integer paise balance checks (amount <= captured - refunded)
│     └─ Live payment verification (GET /v1/payments/{id})    │
│     └─ Payment lifecycle check (payment_state == captured)  │
│     └─ Velocity & per-action amount ceilings                │
│                                                             │
│  3. Multi-Plane Risk Analysis                               │
│     ├─ Risk Engine: Z-score spikes, drift (PSI), NLP mismatch│
│     ├─ Graph Engine: Disjoint-set union-find for sybil rings│
│     └─ Investigation Engine: Calibrated LR + Conformal Risk │
│                                                             │
│  4. Tamper-Evident Audit Ledger                             │
│     └─ Canonical sorted JSON hash chain (SHA-256 links)     │
└──────────────────────────────┬──────────────────────────────┘
                               │
            ┌──────────────────┼──────────────────┐
            ▼                  ▼                  ▼
        [ ALLOW ]          [ REVIEW ]          [ BLOCK ]
            │                  │                  │
            │                  ▼                  └─ Abort & Audit
            │         Human Triage Console
            │         (Single-winner claim lock)
            ▼                  │
┌───────────────────────┐      │
│   Razorpay Gateway    │◄─────┘ (On Human Approval)
│                       │
│ ├─ Idempotency Key    │
│ │  rfnd_{pay}_{act}   │
│ ├─ Pending Claim Lock │
│ └─ Receipt Probe      │
└───────────┬───────────┘
            │ HTTPS
            ▼
    Razorpay API v1
```

### The Four Decision Planes

| Plane | Checks & Invariants | Failure Action |
|---|---|---|
| **1. Policy Engine** | Strict integer paise bounds, currency validation, live `GET /v1/payments/{id}` verification, `payment_state == "captured"`, residual balance `amount <= captured_paise - refunded_paise`, merchant velocity caps, RBI RiskTier limits. | Immediate `BLOCK` (fail-closed on missing state) |
| **2. Risk Engine** | Anomaly z-scores on velocity/amount, Population Stability Index (PSI) drift detection, NLP intent-versus-amount semantic mismatch, Complex Event Processing (CEP). | Emits continuous risk score `[0.0, 1.0]` |
| **3. Risk Graph** | Disjoint-set Union-Find cluster analysis tracking shared devices, physical delivery addresses, and payment instruments across merchant accounts. | Flags sybil rings & collusion clusters |
| **4. Investigation & Learned CRC** | Calibrated Logistic Regression scoring ($p̂$) and Conformal Risk Control (CRC) error bounds ($τ_{\\text{clear}}, τ_{\\text{block}}$) with per-feature SHAP explainability. | Directs ambiguous or contested evidence to `REVIEW` |

---

## Core Invariants & Safety Guarantees

| Invariant | Implementation Mechanism | Verification Test |
|---|---|---|
| **Zero Credential Exposure** | Razorpay keys reside strictly within governor server environment. Client and agents communicate via governor API tokens. | `governor-server::tests` |
| **Paise Financial Safety** | Fixed-point `i64` paise representations throughout; decimal floats are rejected at schema validation with HTTP 400. | `policy-engine::tests::test_integer_paise` |
| **Fail-Closed Balance Gate** | Enforces `captured` state and verifies `amount <= captured_paise - refunded_paise`. Missing payment metadata fails closed. | `missing_payment_state_fails_closed` |
| **At-Most-Once Execution** | Composite idempotency key `rfnd_{payment_id}_{decision_id}`, locked in-flight claim cache, and automated receipt probing. | `razorpay-gateway::tests::idempotency` |
| **Tamper-Evident Ledger** | Strictly ordered canonical JSON serialisation hashed into continuous `SHA-256` chain (`previous_hash` → `current_hash`). | `audit-service::tests::test_chain_integrity` |
| **Length-Prefixed Framing** | Input hashes use explicit byte length framing to prevent delimiter collision and canonicalization ambiguity. | `risk-governor-types::tests::test_input_hash` |
| **Constant-Time Verification** | API key and HMAC webhook verification use constant-time comparisons (`subtle::ct_eq`) to eliminate timing side-channels. | `governor-server::auth` |
| **Single-Winner Concurrency** | Atomic `claim_review` lock ensures an action under review cannot be double-executed under concurrent human approvals. | `concurrent_approvals_execute_exactly_once` |
| **Bounded Memory Footprint** | LRU decision cache (10k entries), ring-buffered velocity logs (100k entries), and 1-hour TTL on gateway execution cache. | `governor-server::state` |

---

## Quickstart

### 1. Build and Run Locally

```bash
# Clone the repository
git clone https://github.com/theoxfaber/agentic-payment-risk-governor.git
cd agentic-payment-risk-governor

# Run offline test suite (205 passing unit and integration tests)
cargo test --workspace

# Start the Governor server (in-memory mode, no external dependencies)
cargo run --release -p governor-server -- --port 8080
```

Once running:
- **Triage Console:** `http://127.0.0.1:8080`
- **Prometheus Metrics:** `http://127.0.0.1:8080/metrics`
- **Health Check:** `http://127.0.0.1:8080/health`

### 2. End-to-End Interactive Verification

Run the automated verification script demonstrating `ALLOW`, `REVIEW`, `BLOCK`, and audit log verification:

```bash
./demo.sh
```

### 3. Agent Integration via MCP (Model Context Protocol)

Connect any Claude Desktop, Cursor, or autonomous LLM agent runtime directly to the Governor as an MCP tool server:

```json
{
  "mcpServers": {
    "risk-governor": {
      "command": "cargo",
      "args": ["run", "--release", "-p", "mcp-server"],
      "env": {
        "GOVERNOR_URL": "http://127.0.0.1:8080",
        "GOVERNOR_API_KEY": "your_governor_key"
      }
    }
  }
}
```

Exposed MCP Tools:
- `check_action`: Submit payment action intent for validation and automated execution.
- `get_decision`: Query evaluation details, risk factors, and cryptographic receipt.
- `list_reviews`: Fetch pending decisions awaiting human operator review.

---

## API Surface

All `/v1/*` endpoints require authentication via `X-API-Key: <KEY>` or `Authorization: Bearer <KEY>`.

| Method | Endpoint | Description |
|---|---|---|
| `POST` | `/v1/actions` | Evaluate payment action. Returns `verdict` (`ALLOW`, `REVIEW`, `BLOCK`), risk score, and execution receipt. |
| `GET` | `/v1/decisions` | List recent decisions, including conformal bounds and calibrated probabilities ($p̂$). |
| `GET` | `/v1/decisions/{id}` | Retrieve full decision detail, SHAP feature attributions, and cryptographic audit trail. |
| `POST` | `/v1/decisions/{id}/approve` | Human reviewer approval/rejection endpoint with atomic single-winner execution lock. |
| `GET` | `/v1/audit/verify` | Verify cryptographic hash chain continuity and HMAC signature validity. |
| `GET` | `/v1/real/analysis?count=20` | Real-time analysis of live Razorpay test-mode payments using configured credentials. |
| `POST` | `/webhooks/razorpay` | Ingest Razorpay webhooks with constant-time signature verification (`X-Razorpay-Signature`). |

### Example: Action Evaluation Request

```bash
curl -X POST http://127.0.0.1:8080/v1/actions \
  -H "X-API-Key: demo123" \
  -H "Content-Type: application/json" \
  -d '{
    "agent_id": "support-agent-04",
    "merchant_id": "merch_acme_corp",
    "action_type": "refund",
    "amount": 4900,
    "currency": "INR",
    "declared_intent": "Customer return for defective item on Order #9481",
    "context": {
      "payment_id": "pay_O9xK8w7e5Y1Z2a",
      "payment_state": "captured",
      "captured_paise": 100000,
      "refunded_paise": 20000,
      "customer_id": "cust_88219"
    }
  }'
```

---

## Empirical Benchmark & Validation

The multi-plane governor was evaluated against baseline risk architectures on held-out synthetic transactional distributions modeling refund abuse, velocity attacks, and sybil collusion networks (`eval-harness`):

| Evaluation Strategy | Precision | Recall | False Positive Cost | Prevented Loss |
|---|---:|---:|---:|---:|
| Static Per-Customer Limits | 100.0% | 66.0% | ₹0 | ₹1,68,300 |
| Naive Graph Clustering | 51.2% | 100.0% | ₹65,925 | ₹2,02,950 |
| **Governor (Multi-Plane + CRC)** | **100.0%** | **100.0%** | **₹0** | **₹2,02,950** |
| Calibrated Logistic Regression | 100.0% | 94.2% | ₹0 | ₹2,01,600 |

- **Legitimate Edge Cases:** 972 simulated multi-member households evaluated with zero false positive blocks.
- **Robustness:** 140 randomized parameter permutations demonstrated 100% precision / 99.4% recall stability.
- **Reproducibility:** Run `cargo run --release -p eval-harness` to regenerate the full benchmark report. See [`BENCHMARK.md`](BENCHMARK.md) and [`docs/AI_DESIGN.md`](docs/AI_DESIGN.md).

---

## Workspace Layout

```
├── crates/
│   ├── action-service/       # Orchestration pipeline linking policy, risk, graph, and gateway
│   ├── audit-service/        # Canonical JSON serialization & SHA-256 tamper-evident ledger
│   ├── eval-harness/         # Conformal risk control (CRC) calibration & benchmark harness
│   ├── evidence-service/     # Bayesian evidence aggregation & contradiction detection
│   ├── governor-server/      # Axum HTTP API, constant-time auth, metrics, and static asset server
│   ├── investigation-engine/ # Explainable SHAP attribution, LLM claim verification
│   ├── mcp-server/           # Model Context Protocol adapter for agent framework integration
│   ├── policy-engine/        # Deterministic financial limits, RBI risk tiers, integer paise math
│   ├── razorpay-gateway/     # Idempotent execution proxy with receipt probe verification
│   ├── risk-engine/          # Z-score statistical anomalies, PSI distribution drift, CEP
│   └── risk-graph/           # Disjoint-set union-find engine for sybil ring discovery
├── dashboard-v2/             # Production React 19 + TanStack Query risk operator console
```

---

## Environment Configuration

| Environment Variable | Description | Default / Fallback |
|---|---|---|
| `GOVERNOR_API_KEY` | Secret key required to access `/v1/*` endpoints | Ephemeral key generated on boot |
| `RAZORPAY_KEY_ID` | Razorpay API Key ID for live test execution | Simulated mock gateway |
| `RAZORPAY_KEY_SECRET` | Razorpay Key Secret | Simulated mock gateway |
| `WEBHOOK_SECRET` | Secret for HMAC verification of Razorpay webhooks | Disabled if unset |
| `AUDIT_SIGNING_KEY` | HMAC secret for signing audit chain head hashes | Disabled if unset |
| `DATABASE_URL` | PostgreSQL connection string for persistent storage | In-memory storage |
| `NATS_URL` | NATS message broker endpoint for distributed events | In-process channel |
| `LLM_API_KEY` | Optional API key for semantic intent claim validation | Heuristic NLP fallback |

Reference configuration template: [`.env.example`](.env.example).

---

## License

Dual-licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
