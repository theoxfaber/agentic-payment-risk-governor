# Changelog

All notable changes to Risk Governor are documented here.
Format based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added — learned intelligence layer (docs/AI_DESIGN.md)
- Pure-Rust logistic regression (`eval-harness::lr`): class-weighted, L2,
  deterministic training on CALIBRATION worlds only; ships as a versioned
  JSON artifact (`eval-harness/artifacts/lr_model.json`). Held-out:
  100% precision / 100% recall at 0.5 cut — the feature set carries the signal
- Conformal Risk Control calibration (`eval-harness::conformal`): the REVIEW
  band's thresholds are derived from two explicit policy budgets (fraud-leak
  α=2%, friction α=1%) with finite-sample distribution-free validity instead
  of magic numbers (Vovk 2005; Angelopoulos & Bates ICLR 2024)
- Instance-dependent economics in `calibrated_lr_crc` (Elkan 2001; 2-D (p̂,
  amount) threshold region): auto-allow only when p̂×exposure costs less than
  one ₹400 human review → held-out 100% precision, 94% recall, ₹0 friction,
  and a bounded ₹13.50 concession that is cheaper than reviewing would have
  been. The honest cost of automation, per instance
- PSI drift monitoring: governor-server exports a risk-score histogram and
  Population Stability Index vs `SCORE_REFERENCE_JSON` — prediction drift is
  the earliest concept-drift signal while labels mature (Dal Pozzolo TNNLS 2017)
- Prompt-injection hardening for LLM intent extraction: agent-controlled text
  sanitized (control chars, fences, case-insensitive role-injection openers,
  length cap) and delimited as untrusted data inside `<declared_intent>` tags;
  claims remain evidence-only downstream
- `docs/AI_DESIGN.md`: full research grounding — why an LLM never decides,
  why logistic regression, conformal guarantees, cost-sensitive thresholds,
  drift monitoring, and the production label-maturation path

### Fixed — audit-driven hardening pass (docs/BUGS.md §6–14)
- **Approval TOCTOU**: concurrent reviews could double-execute a payment.
  Claim-under-lock protocol before any await; pinned by an 8-way concurrent
  test asserting exactly one gateway execution
- **Dashboard credential leak**: served HTML embedded the live API key to
  unauthenticated callers; page now carries no secret (browser-held key)
- **Label leakage**: population baseline excluded abusers via ground truth;
  replaced with label-free trimmed-pool estimation (honest side effect visible
  in the eval table)
- **Metric inflation**: randomized-world sweep dropped legitimate-world FPs
  from pooled precision despite a comment claiming otherwise; now pooled and
  reported explicitly
- **Fail-open custom rules**: unknown rule conditions silently evaluated
  false; now fail closed with a violation record
- **Dead validation**: `validate_request` was tested but never called inside
  the pipeline; now step zero of every entry path, with caller mistakes
  mapped to HTTP 400 (was 500)
- **Inert intent score**: `intent_mismatch_score` was computed end-to-end and
  consumed by nothing; combiner now forces REVIEW on contradiction ≥0.5
- **Zombie workers**: NATS workers exited 0 after failed subscription; binaries
  now exit non-zero so orchestrators can see the failure
- **Demo data in prod DBs**: demo seeding on Postgres now requires SEED_DEMO=true

### Changed
- Eval harness grows from 3 to 5 detectors (`learned_logistic`,
  `calibrated_lr_crc` join the comparison table); suite trains once per run,
  calibration worlds only

### Added (earlier this release)
- Robustness evaluation (`eval-harness::robustness`): degradation sweep over
  held-out worlds (missing behavioral records, timing jitter, count noise) —
  recall holds at 100% while human-review share rises and legitimate friction
  stays bounded; randomized-parameter sweep across 140 never-tuned world
  shapes lands 100% precision / 99.4% recall with FPs from legitimate worlds
  now pooled into precision
- Three robustness regression gates: recall holds + uncertainty routes to
  humans under degradation; randomized-world precision/recall ≥95%;
  perturbation harness proven non-vacuous
- README "What breaks first when the data gets messy" section with the
  degradation table and honest read of where the cost lands
- `docs/TESTING.md`: per-crate test inventory (157 tests), the offline-run
  guarantee, and the explicit list of opt-in infrastructure-tagged suites
- Fresh-clone verification: clean clone + stripped env (no credentials)
  passes the full suite and `./demo.sh` end-to-end

### Changed
- `governor-server` split into focused modules (`auth`, `backends`,
  `bootstrap`, `routes`, `state`) — main.rs down from 952 to 260 LOC;
  handler/replay/auth coverage grew from 6 to 12 tests in the process
- Dashboard API-key placeholder renamed to a self-evidently non-secret token;
  README test counts reconciled with the actual suite (154)

### Added (earlier this release)
- `./demo.sh` — one-command scripted walkthrough: thesis, ALLOW/REVIEW/BLOCK
  with live audit replay and human approval, agentic payout, held-out eval
  headline. `--keep` leaves the server running (for recording)
- `mcp-server` crate: Model Context Protocol server over stdio exposing the
  governor as agent tools — `check_action`, `get_decision`, `list_reviews`.
  Any MCP-capable AI agent now goes through the same policy/risk/investigation
  gates and audit trail as every other client
- Payout flow end-to-end: RazorpayX `/payouts` routing (fund account, mode,
  purpose from context, 30-char narration cap), `/payment_links` routing;
  identical policy caps, risk scoring, and audit trail

### Changed
- README: one-command demo quick-start, MCP integration guide, payout example

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
