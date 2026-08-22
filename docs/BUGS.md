# Failure Stories — Real Bugs Hit & Fixed

Running ledger. Section 22 material: every entry here is a bug we actually
hit during development, caught by a test, not staged examples. Each entry
names the detection mechanism, because "how did you catch it" is the part
judges probe.

---

## 1. Pre-decision audit records were unlinkable to their decision

**Phase 1 · caught by:** `allow_review_block_end_to_end` failing on trail assertion

Audit records emitted *before* the decision existed (ActionRequested,
PolicyEvaluated, RiskScored) carried `decision_id: None`. `trail_for(id)`
filtered on `Some(id)` — so replay could never see how a decision was
reached, only the outcome. Exactly the bug that stays invisible until
someone asks "show me why" under pressure.

**Fix:** decision_id generated at *request arrival* and threaded through
every subsequent record. The ID now exists before the decision does.

## 2. Correlation-ID leak between request and bus envelope

**Phase 2 step 3 · caught by:** comparing demo stdout vs worker logs manually

After splitting policy-engine over NATS, the envelope's correlation_id was a
*fresh fallback UUID*, silently diverging from the request's own ID — two
IDs floating around for one logical action. Root cause: nothing set the
task-local in the action-service path, so `current_correlation_id()` fell
back to generating a new one.

**Fix:** `process_action` wraps the entire pipeline in
`scope_correlation(request.correlation_id, …)` — downstream bus calls
inherit the request's ID automatically. Verified end-to-end: identical IDs
now appear in both processes' structured logs.

## 3. NATS "no responders" race delivering garbage to reply inboxes

**Phase 2 step 3 · caught by:** `remote_policy_evaluation_allow` failing with
`EOF while parsing a value at line 1 column 0`

If a request publishes before the worker's subscription registers server-
side, NATS (with no-responders enabled) delivers an **empty message with a
503-style status** to the reply inbox — not silence, not an error. Our
client tried to JSON-decode it and blew up. Startup-timing dependent: the
kind of thing that passes every test and then detonates during a live demo
restart.

**Fix:** client checks `msg.status` before decoding; any status message maps
to the fail-safe path (`policy_engine_unavailable:no_responders_NNN`) →
human review. Locked in by `policy_engine_down_fails_safe_to_review`.

## 4. Silent-Allow hazard hiding behind evidence degradation (found by design review during the evidence-service split)

Transport-level evidence failures (timeout/no-responders) initially looked
like they'd fall back to *benign default evidence* → policy evaluates
Allow → risk scores ~0 → **decision Allow**. A downed evidence service
would have produced exactly the silent-allow outcome the architecture
exists to prevent.

**Fix:** `GatheredEvidence.degraded_reason` flows explicitly through the
trait; the combiner injects an `evidence_service_unavailable:*` rule into
the policy result → forced Review, visible in the audit trail.

## 5. Synthetic fixtures with degenerate timing data (`vec!` + RNG)

**Dataset generation · caught by:** `adversarial_evasion_still_clusters_but_with_weaker_behavior`
failing with `spread = 0`

`vec![rng.random_range(12.0..140.0); n]` evaluates the expression **once**
and clones it — every synthetic customer had perfectly identical
purchase→return gaps. Consequence if uncaught: `synchronized_returns` fires
trivially on every ring, the investigator looks brilliant, and the
precision/recall table in the README is fiction. The adversarial-evasion
world exists precisely to measure detection under jittered timing — it can't
do that with zero-variance fixtures.

**Fix:** `(0..n).map(|_| rng.random_range(a..b)).collect()` in all five
generators. Determinism preserved via the seeded StdRng.

---

## Known limitations (honest list)

| Limitation | Why | Plan |
|---|---|---|
| Evidence store is in-memory per-process | Postgres-backed `EvidenceStore` not built yet | Next phase |
| Workers seeded via JSON file, not DB | Same root cause as above | Replaced by Postgres store |
| Audit writes still in-process | Fire-and-forget bus pattern deliberately scheduled last | Phase 2 final step |
