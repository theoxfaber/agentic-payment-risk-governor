#!/usr/bin/env bash
# High-throughput simulation — proves the proxy judges every payment at line rate.
# Usage: GOVERNOR_API_KEY=demo123 ./scripts/load_test.sh [count]
set -e
KEY="${GOVERNOR_API_KEY:-demo123}"
BASE="${BASE:-http://127.0.0.1:8080}"
COUNT="${1:-50}"
for i in $(seq 1 "$COUNT"); do
  AGENT=$([ $((RANDOM%2)) -eq 0 ] && echo "agent-trusted-01" || echo "agent-sketchy-99")
  TYPE=$([ $((RANDOM%3)) -eq 0 ] && echo "refund" || ([ $((RANDOM%2)) -eq 0 ] && echo "payout" || echo "payment_link"))
  AMT=$(( (RANDOM%6000 + 10)*100 ))
  if [ "$TYPE" = "refund" ]; then CTX="{\"payment_id\":\"pay_load_$i\",\"payment_state\":\"captured\",\"captured_paise\":600000,\"refunded_paise\":0}"
  elif [ "$TYPE" = "payout" ]; then CTX="{\"fund_account_id\":\"fa_demo_$i\",\"mode\":\"IMPS\"}"
  else CTX="{\"reference_id\":\"ref_$i\"}"; fi
  curl -s -X POST "$BASE/v1/actions" -H "Content-Type: application/json" -H "X-API-Key: $KEY" -d "{\"agent_id\":\"$AGENT\",\"merchant_id\":\"merchant-001\",\"action_type\":\"$TYPE\",\"amount\":$AMT,\"currency\":\"INR\",\"declared_intent\":\"refund order #$i\",\"context\":$CTX}" > /dev/null &
  [ $((i%10)) -eq 0 ] && wait
done
wait
echo "burst $COUNT done"
curl -s "$BASE/v1/decisions" -H "X-API-Key: $KEY" | python3 -c "import json,sys; d=json.load(sys.stdin); print(len(d), 'total'); from collections import Counter; print(Counter(x['decision'] for x in d))"
curl -s "$BASE/metrics" | grep -E "decisions_total|gateway_executions|risk_score_bucket"
