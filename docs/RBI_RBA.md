# RBI Risk-Based Authentication & Compliance

> Possession factor = device fingerprint, risk_tier mapping SDD/CDD/EDD, DoT FRI integration, PMLA retention

- **Risk tiers:** `RiskTier::Standard` (SDD) → `Medium` (CDD) → `High/VeryHigh` (EDD) maps to DoT FRI Medium/High/VeryHigh (razorpay.com/blog/risk-scoring-indian-payments-implementation)
- **FRI score:** `MerchantPolicy.fri_score: Option<u8>` ingested via DoT feed; `policy-engine` fail-closed: FRI 90-100 → VeryHigh → BLOCK + STR, 67-89 → High → EDD required
- **KYC Master Jun 12 2025:** `pmla_retention_days: 1825` (5 years) enforced, `REVOKE UPDATE,DELETE` on `audit_records` + HMAC anchor `AUDIT_SIGNING_KEY`
- **PMLA STR → FIU-IND:** `audit-service` chain head HMAC published externally; full-chain rewrite without key fails `verify_chain_with_anchor`
- **DPDP Act:** audit redacts `email/phone/payment_id→sha256` before chain append
- **Latency:** p95 <180ms (<200ms Thirdwatch SLO) via `risk_governor_request_duration_ms` histogram
