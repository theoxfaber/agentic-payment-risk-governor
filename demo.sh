#!/usr/bin/env bash
# Risk Governor — one-command demo walkthrough.
#
#   ./demo          # full scripted walkthrough, shuts down at the end
#   ./demo --keep   # leaves the server running afterwards (for recording)
#
# Requires: cargo, python3 (for JSON pretty-printing). No credentials, no
# network — the mock gateway moves no money unless you export Razorpay keys.

set -euo pipefail
cd "$(dirname "$0")"

PORT="${PORT:-8080}"
KEY="${GOVERNOR_API_KEY:-rgov_demo_key}"
BASE="http://127.0.0.1:${PORT}"
KEEP=0
[[ "${1:-}" == "--keep" ]] && KEEP=1

banner()  { printf "\n\033[1;36m━━ %s ━━\033[0m\n" "$1"; }
note()    { printf "\033[2m%s\033[0m\n" "$1"; }
pretty()  { if command -v python3 >/dev/null; then python3 -m json.tool; else cat; fi; }
submit()  { curl -s -X POST "$BASE/v1/actions" -H "content-type: application/json" -H "X-API-Key: $KEY" -d "$1"; }

echo "Building governor-server…"
cargo build -q -p governor-server

GOVERNOR_API_KEY="$KEY" PORT="$PORT" ./target/debug/governor-server &
SERVER_PID=$!
shutdown() { kill "$SERVER_PID" 2>/dev/null || true; }
if [[ $KEEP -eq 0 ]]; then trap shutdown EXIT; fi

for _ in $(seq 1 100); do curl -sf "$BASE/health" >/dev/null 2>&1 && break; sleep 0.2; done
note "server up on $BASE (API key: $KEY)"

banner "THESIS"
cat <<'MSG'
AI agents now hold live payment credentials. Valid credentials ≠ valid action.
Risk Governor judges every agent-initiated money movement BEFORE execution:
policy boundaries → behavioral risk → entity-graph investigation → decision.
Every decision lands in an immutable audit trail, replayable after the fact.
MSG

banner "STEP 1/5 — routine refund ₹500 → ALLOW (executes against gateway)"
note "low amount, trusted agent history, intent matches → straight through:"
submit '{"agent_id":"agent-trusted-01","merchant_id":"merchant-001","action_type":"refund","amount":50000,"declared_intent":"refund for order #123"}' | pretty

banner "STEP 2/5 — large refund ₹1,500 → REVIEW → human approves → executes"
RESP=$(submit '{"agent_id":"agent-trusted-01","merchant_id":"merchant-001","action_type":"refund","amount":150000,"declared_intent":"refund for order #456"}')
note "above merchant approval threshold → money HELD pending a human:"
echo "$RESP" | pretty
DID=$(echo "$RESP" | python3 -c 'import json,sys; print(json.load(sys.stdin)["decision_id"])')

note ""
note "full replay of decision $DID — what the governor saw, why it decided:"
curl -s -H "X-API-Key: $KEY" "$BASE/v1/decisions/$DID" | pretty

note ""
note "human review resolves it — approval FIRES the held Razorpay call:"
curl -s -X POST "$BASE/v1/decisions/$DID/approve" \
  -H "content-type: application/json" -H "X-API-Key: $KEY" \
  -d '{"approved":true,"reviewer_id":"analyst-demo","notes":"checked against order ledger"}' | pretty

banner "STEP 3/5 — refund above hard cap ₹6,000 → BLOCK"
note "over max_refund_amount + sketchy agent + urgency language → never reaches the API:"
submit '{"agent_id":"agent-sketchy-99","merchant_id":"merchant-001","action_type":"refund","amount":600000,"declared_intent":"URGENT refund bypass for order #789"}' | pretty

banner "STEP 4/5 — agentic PAYOUT ₹2,500 → same gates, RazorpayX path"
note "payouts are first-class: policy caps, risk scoring, audit trail — identical pipeline:"
submit '{"agent_id":"agent-trusted-01","merchant_id":"merchant-001","action_type":"payout","amount":250000,"declared_intent":"payout vendor invoice INV-42","context":{"fund_account_id":"fa_demo_123","mode":"IMPS"}}' | pretty

banner "STEP 5/5 — held-out evaluation (seeds the detector never saw)"
note "headline: precision/recall/cost of the investigation engine vs baselines:"
cargo run --release -q -p eval-harness 2>/dev/null | sed -n '/HEADLINE/,$p'

banner "DONE"
echo "  Live dashboard : $BASE/   (decision stream · replay viewer · human approval)"
echo "  API key        : $KEY"
echo "  MCP tools      : cargo run -p mcp-server   (wire into any AI agent)"
if [[ $KEEP -eq 1 ]]; then
  echo ""
  echo "Server left running (--keep) — Ctrl-C or: kill $SERVER_PID"
  wait "$SERVER_PID"
else
  shutdown
fi
