# Demo — Judge Path (10 minutes, no credentials)

**What this is:** defense-only verifier for **agent-initiated refund abuse**. The agent never holds the Razorpay secret — it posts an intent to the governor, the governor decides `ALLOW / REVIEW / BLOCK` before Razorpay is called.

## One command

```bash
git clone https://github.com/theoxfaber/agentic-payment-risk-governor
cd agentic-payment-risk-governor
cargo test --workspace        # 201 tests offline
./demo.sh                     # ALLOW → REVIEW → BLOCK → payout + held-out summary
```

`demo.sh` needs no keys, no network, no Postgres — it drives `cargo run -p governor-server` and prints decisions + audit hashes.

## Manual triage (dashboard)

```bash
GOVERNOR_API_KEY=demo123 cargo run -p governor-server -- --port 8080
# http://127.0.0.1:8080/dashboard  (or http://127.0.0.1:5173 if running dashboard-v2 dev)
# API key: demo123  (page is unauthenticated — key lives in browser storage, never in HTML)
```

**Verify invariants live:**
```bash
# 1. ALLOW — small refund within balance
curl -H 'X-API-Key: demo123' -H 'Content-Type: application/json' \
  -d '{"agent_id":"agent-trusted-01","merchant_id":"merchant-001","action_type":"refund","amount":5000,"currency":"INR","declared_intent":"refund order #123","context":{"payment_id":"pay_demo","payment_state":"captured","captured_paise":100000,"refunded_paise":20000}}' \
  http://127.0.0.1:8080/v1/actions

# 2. BLOCK — over-refund
curl -H 'X-API-Key: demo123' -H 'Content-Type: application/json' \
  -d '{"agent_id":"agent-trusted-01","merchant_id":"merchant-001","action_type":"refund","amount":90000,"currency":"INR","declared_intent":"refund order #999","context":{"payment_id":"pay_demo","payment_state":"captured","captured_paise":100000,"refunded_paise":20000}}' \
  http://127.0.0.1:8080/v1/actions
# → 403 "exceeds available balance 80000 (captured 100000 - refunded 20000)"

# 3. REVIEW → approve exactly once (8 concurrent approvers → 1 execution)
curl -H 'X-API-Key: demo123' http://127.0.0.1:8080/v1/decisions | jq '.[] | select(.decision=="review") | .decision_id'
curl -X POST -H 'X-API-Key: demo123' -H 'Content-Type: application/json' \
  -d '{"approved":true,"reviewer_id":"analyst-7","notes":"approved via demo"}' \
  http://127.0.0.1:8080/v1/decisions/{id}/approve
cargo test --test concurrent_approvals_execute_exactly_once -- --nocapture

# Audit replay + metrics
curl -H 'X-API-Key: demo123' http://127.0.0.1:8080/v1/decisions/{id} | jq
curl http://127.0.0.1:8080/metrics | grep risk_governor
```

## Held-out evaluation (synthetic)

```bash
cargo run --release -p eval-harness
# calibration seed 2026, held-out 31415,27182,16180 — see docs/EVAL_REPORT_2026-08-28.md
EVAL_HELDOUT_SEEDS=12345,67890 cargo run --release -p eval-harness  # externally supplied seeds
cat BENCHMARK.md  # canonical table — source of truth, do not use stale PITCH/BENCHMARK numbers
```

Numbers are **synthetic** (proves train→calibrate→guarantee→monitor machinery, not production fraud performance). Chart: `docs/eval-results.svg`.

## Live Razorpay (partial, test-mode)

```bash
RAZORPAY_KEY_ID=rzp_test_... RAZORPAY_KEY_SECRET=... cargo run -p razorpay-gateway --bin rzp_smoke
# 2026-08-28 live: auth OK, order_TUyv0Ib1swX7ki created, /payments/create/json 404 (deprecated) → partial pass
# Refund path is covered by HttpGateway idempotency + receipt-probe tests (mocked payment_id, no live captured payment)
```

## Architecture

`action_requested → policy_evaluated → risk_scored → graph_analyzed → decision_made → human_reviewed → razorpay_called`
Docs: `README.md` · `docs/AI_DESIGN.md` §5 (learned is evaluation plane today) · `docs/TESTING.md` · `BENCHMARK.md`

## Known limitations

- Learned logistic + CRC runs in `eval-harness`; live `/v1/actions` uses hand-tuned combiner + risk features (model_version `1.1.0-investigated` / `1.2.0-intent-heuristic`). Wiring is next milestone — see `docs/AI_DESIGN.md` §5.
- Data is synthetic, worlds are small (300 bg + 6–8 rings), 100% precision on synthetic invites scrutiny — see `BENCHMARK.md` §1 for FP/FN costs and household checks.
- Live smoke is partial (no live refund of captured payment) — Razorpay deprecated legacy payment simulation endpoint.
- In-memory audit/decisions unless `DATABASE_URL` is set; `SEED_DEMO` gates demo seeding on Postgres.

## Troubleshooting

- `401 missing or invalid API key` → set `GOVERNOR_API_KEY` env or use `demo123` as above.
- `cargo test` fails with `database` → you ran `--ignored` without `docker compose up -d` — plain `cargo test --workspace` needs no infra.
- Dashboard asks for key repeatedly → browser stored wrong key: DevTools → `localStorage.setItem('rgov_key','demo123'); location.reload()`.
