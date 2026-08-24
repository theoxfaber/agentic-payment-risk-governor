# Changelog

All notable changes to Risk Governor are documented here.
Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added
- API-key auth on every `/v1/*` route (`GOVERNOR_API_KEY`, or an ephemeral
  key generated and printed at boot) — constant-time comparison; the
  dashboard authenticates with the server's own key
- `intent-engine`: AI-assisted declared-intent understanding. Deterministic
  heuristic extractor always on; LLM-backed (OpenAI-compatible, env-gated)
  when configured. Extracted claims (amount, order ref, action type, urgency)
  are checked against request hard fields as risk evidence — never the
  decision-maker, can only raise a score
- Gateway idempotency: one decision_id executes exactly once (duplicate
  execution returns the cached response); refunds get a lost-response guard
  that probes the payment's refund list after an ambiguous 5xx and refuses
  to double-fire a refund that already landed

### Changed
- Evaluation protocol: thresholds tuned on calibration worlds (seed 2026)
  only; headline precision/recall/cost numbers now measured on held-out
  worlds (three unseen seeds). Regression tests enforce recall ≥90% and zero
  false positives on every held-out seed
- Webhook signature verification uses constant-time `verify_slice` instead
  of hex string equality

## [0.1.0] - 2026-08-23

First coherent release of the decision pipeline.

### Added
- Decision pipeline: policy → risk scoring → entity-graph investigation →
  evidence-weighted combiner → ALLOW / REVIEW / BLOCK
- Investigation engine with evidence directionality (Supports / Contradicts /
  Missing), confidence weighting, and the adversarial-evasion hold rule:
  structurally-linked clusters without behavioral confirmation go to human
  review, never auto-cleared
- Unified axum server (`governor-server`): decision API, full replay,
  human-review approval endpoint, live dashboard, Prometheus `/metrics`
- Razorpay test-mode gateway: basic auth, 429/5xx retry with Retry-After,
  order/payment/refund helpers, offline mock gateway
- Distributed mode: NATS pub/sub split of policy/evidence planes with
  correlation-ID propagation and fail-safe REVIEW on transport degradation
- Audit trail + replay engine reconstructing any decision after the fact
- Evaluation harness over synthetic adversarial worlds (coincidental sharing,
  merchant collusion, adaptive evasion, household false-positive check):
  investigation engine 100% precision/recall vs baselines missing 42% of abuse
  or flagging innocent households
- CI: build/test/clippy(-D warnings)/fmt gates, cargo-audit dependency scan,
  enforced line coverage; Dependabot for Cargo + Actions
