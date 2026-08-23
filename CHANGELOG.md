# Changelog

All notable changes to Risk Governor are documented here.
Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

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
