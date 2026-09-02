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

## Audit-driven hardening pass (pre-submission review)

The following were found by a line-by-line audit of the whole tree.

## 6. Concurrent approvals could double-execute a payment (TOCTOU)

**Review resolution · caught by:** manual concurrency analysis; pinned by the
new `concurrent_approvals_execute_exactly_once` test

`approve_decision` read the decision (read lock), checked `human_review.is_none()`,
then awaited the gateway call, then wrote back. Two simultaneous approvers
both passed the check during the await window → both executed the payment.
In a system whose entire job is bounded money movement, this was the worst
bug in the repo.

**Fix:** claim protocol — the decision is *removed* from the map under the
write lock before any await point; concurrent approvers now see 404. The
claim is restored on every failure path (already reviewed, wrong state,
gateway error), so the queue can never silently lose a held payment, and a
failed execution restores it UNRESOLVED for retry with the failure in the
audit trail.

## 7. The dashboard served the server's API key to unauthenticated callers

**Dashboard · caught by:** security review of `routes::dashboard_page`

The HTML page embedded the live API key so its fetches would authenticate.
Anyone who could reach `/` or `/dashboard` owned full `/v1/*` access,
including approving payments. Safe only while bound to 127.0.0.1 — one
`BIND_HOST=0.0.0.0` away from game over.

**Fix:** page carries no secret. Reviewers paste their key once in the
browser (sessionStorage); every fetch attaches it like any other client; a
401 evicts the stored key and re-prompts.

## 8. Population baseline was sanitized with ground-truth labels

**Dataset generation · caught by:** label-leakage review of `baseline_of`

The "population baseline" excluded abusers using ground truth — information
no production baseline can have. Effect size was small but it is label
information flowing into detection, which poisons the meaning of held-out
numbers.

**Fix:** label-free robust estimation — pooled per-customer rates over ALL
customers with the top decile trimmed, so a small abusive minority cannot
inflate its own hiding place. Honest side effect visible in the eval table:
the rules-only strawman now flags a few household customers it previously
"magically" missed.

## 9. Randomized-sweep precision silently dropped legitimate-world FPs

**Robustness harness · caught by:** comment-vs-code mismatch (`// FP tracked
separately below` — nothing tracked them)

The randomized-world sweep skipped household/coincidental-sharing worlds when
pooling precision. Those worlds contain zero abusers, so every flag there is
a false positive — excluding them structurally inflated the headline
precision of a report whose thesis is honest FP accounting.

**Fix:** legit-world FPs are pooled INTO precision and reported explicitly
(`legit customers flagged: N`). Regression test now asserts zero.

## 10. Misconfigured custom rules failed OPEN

**Policy engine · caught by:** fail-open pattern review (`_ => false`)

An unrecognized condition string evaluated to false → a typo'd BLOCK rule
silently allowed everything it was written to stop.

**Fix:** tri-state rule outcome; unknown conditions are reported as threshold
violations ("fail-closed") and block the action.

## 11. Validation existed, tested… and not called by the pipeline

**Action service · caught by:** call-graph audit

`validate_request` had seven passing unit tests but nothing invoked it inside
the pipeline; negative/zero amounts flowed through policy, risk, and into
combiner logic. (The HTTP layer did validate — mapping the error to HTTP 500,
which misclassified a caller mistake as a server fault.)

**Fix:** validation runs as step zero of `process_inner` (every entry path:
HTTP, NATS, demos), and the route maps `Validation` errors to 400.

## 12. The intent-contradiction score was computed end-to-end and ignored

**Combiner · caught by:** data-flow audit

`intent_mismatch_score` was calculated, audited, serialized into every
decision… and read by nobody. The "detects lying agents" story was inert.

**Fix:** combiner consumes it — mismatch ≥ 0.5 injects an `intent_contradiction`
rule → forced human REVIEW (never auto-block; too gameable). Pinned by tests
in both directions.

## 13. NATS workers exited 0 when they couldn't subscribe

**Workers · caught by:** orchestration review (`.ok()` on join handles)

A worker that failed to subscribe logged an error and returned; the binary
then exited SUCCESSFULLY. Under systemd/K8s it looks healthy while serving
nothing.

**Fix:** `run_*_worker` functions return the subscription result; binaries
propagate it and exit non-zero on failure.

## 14. Demo entities seeded unconditionally into production databases

**Bootstrap · caught by:** deployment-path review

Every boot wrote demo agents + a default merchant policy (with hardcoded ₹
limits) into whatever backend was configured — including Postgres.

**Fix:** in-memory keeps seeding for dev ergonomics; Postgres requires
explicit `SEED_DEMO=true`.

## 15. Refund balance invariant was soft — missing fields bypassed the check

**Policy engine · caught by:** pre-submission audit of `README` "Balance bound" vs `policy-engine::evaluate` — the `payment_state` gate and `captured_paise` balance check only ran when the fields were present (`if let Some(...)`), so an agent omitting `payment_state` or `captured_paise` skipped the invariant entirely (still bounded by `max_refund_amount`, but not by actual payment state/balance — same fail-open category as #10 and #7).

**Fix:** fail-closed: missing `payment_state` → `BLOCK "missing payment_state — refund requires captured"`; missing `captured_paise` → `BLOCK "missing captured_paise — refund requires captured amount"`. `eq_ignore_ascii_case("captured")` replaces manual lowercasing. Helpers and tests updated to include `payment_state: captured, captured_paise: 500000, refunded_paise: 0`; new tests `missing_payment_state_fails_closed` / `missing_captured_paise_fails_closed`; `evaluation-service` and `governor` E2E helpers fixed.

## 16. Audit chain was never verified outside unit tests (dead-code verifier)

**Audit service · caught by:** `grep -r verify_chain -- workspace` — implemented, tested, never called

`AuditService::verify_chain` correctly detects payload tampering and broken `previous_hash` linkage, but no HTTP route, replay engine, or startup check invoked it. A "tamper-evident audit log" that no code path verifies is a claim, not a capability — exactly the class #11/#12 brags about catching.

**Fix:** `risk-governor-replay::ReplayEngine::replay` now verifies the per-decision trail before reconstruction (`ChainTampered` error). `governor-server` exposes `GET /v1/audit/verify` (full-chain verification + record count + head) and `GET /v1/audit/anchor` (head + HMAC), and `GET /v1/decisions/{id}` now returns `audit_verified` + `audit_anchor` (HMAC of trail head when `AUDIT_SIGNING_KEY` is set). Wiring is covered by `replay_returns_decision_with_full_trail` asserting `audit_verified == true`.

## 17. Hash-chaining alone doesn't stop a full-chain rewrite

**Audit service · caught by:** threat-model review ("what stops recompute with process access?")

Hash-chaining detects partial tampering, not a determined rewrite — anyone who can write the DB and recompute every `current_hash` can forge a clean chain.

**Fix:** external anchor. `AUDIT_SIGNING_KEY` (HMAC-SHA256 of chain head, key out-of-process, e.g. KMS/env on a different host) is computed on `GET /v1/audit/verify` and `GET /v1/audit/anchor`. An attacker without the key cannot produce a valid anchor for a rewritten chain; publish the anchor periodically to an external immutable sink (log aggregator, SIEM, or even stdout captured by the orchestrator). Postgres is documented as append-only: `REVOKE UPDATE, DELETE ON audit_records FROM app_role` + `pg_advisory_xact_lock` serializes appends, so a compromised app process still cannot silently mutate history without also compromising the key.

## 18. Single-instance in-memory decision/idempotency state

**Governor server + gateway · caught by:** scaling review

`decisions: RwLock<HashMap>` and `HttpGateway/MockGateway executed: Arc<Mutex<HashMap>>` are single-process. Horizontal scaling would diverge, and a crash mid-approval loses the in-memory idempotency cache.

**Fix (honest mitigation, not hand-wave):** decisions are *already* hydrated from Postgres on boot and `upsert_decision` survives restarts; crash recovery replays the persisted map. Gateway at-most-once does **not** rely solely on the in-memory cache: layer 2 is the deterministic Razorpay idempotency key `rfnd_{payment_id}_{decision_id}` (Razorpay server-side dedup) + the `receipt == decision_id` refund-landed probe on 5xx, both of which survive a process loss. The in-memory cache is now atomic via `_pending` claim-before-network with `pg_advisory_xact_lock`-style busy-wait ( `HttpGateway`/`MockGateway` insert `_pending` under lock before the network call; concurrent duplicate waits or reuses the final result, never double-fires — `duplicate_decision_id_executes_exactly_once` pinned). Horizontal scaling requires replacing the `RwLock` with `SELECT ... FOR UPDATE` on a `decisions` row and moving the gateway cache to Postgres — documented as the next scaling step, not pretended away.

## 19. Learned escalation happened after money had moved (critical ordering)

**Governor server · caught by:** execution-order audit (`routes.rs:78–141` vs `action-service:264–278`)

`submit_action` called `svc.process_action()` first, which executed the Razorpay gateway for every pre-learned `ALLOW`; only afterward did the route compute `p̂`, attach `learned_insight`, and mutate `ALLOW → REVIEW/BLOCK` in the HTTP response. A request could return `decision: "block"` while a gateway call had already fired — the response, metrics, and audit no longer described the action that controlled execution.

**Fix:** promote the learned gate into the pipeline. `action-service::learned::DefaultLearnedScorer` is now injected into `ActionService` (`with_learned_scorer`) and scored *before* `DecisionMade` and before the `ALLOW` branch. The final outcome is computed once, audited once, and only then passed to `RazorpayGateway`. The HTTP layer no longer rewrites decisions post-execution; it only records metrics from `decision.learned_insight`. Wired through `governor-server` production `wire()` and `bootstrap::test_state`; pinned by `learned_escalation_blocks_before_gateway` (high `p̂` + expensive amount → `BLOCK` with zero gateway calls) and `learned_review_escalation_blocks_allow_but_still_no_gateway`.

## 20. Refund balance check trusted the agent's claimed payment state (no Razorpay lookup)

**Policy + gateway · caught by:** external pre-demo audit — `policy-engine::evaluate` read `captured_paise / refunded_paise / payment_state` only from `request.context` (agent-supplied JSON). `BUGS #15` made missing fields fail-closed, but *present and false* fields sailed through. No code path called `GET /v1/payments/{id}`. An agent could POST `{"payment_id":"pay_real","payment_state":"captured","captured_paise":99999999}` for any `payment_id` and pass the README's "Balance bound" invariant. Real money was still capped because Razorpay would reject the over-refund as a side effect — but the governor itself wasn't the enforcer it claimed to be, violating the pitch "valid credentials don't mean a valid action."

**Fix:** `RazorpayGateway::verify_payment(payment_id) -> Option<VerifiedPayment>` — `HttpGateway` now `GET /v1/payments/{id}` (BasicAuth, same creds as refunds) *before* policy evaluation. Verified `status / amount / amount_refunded` overwrites the claimed context so `policy-engine` evaluates against ground truth, and the original claimed vs verified values plus a `mismatch` flag are appended to the audit trail (`payment_verified: true`). `MockGateway` returns `None` → audit marks `mode: mock_unverified` and trusts claimed context for offline demos. If Razorpay returns non-`captured`, 404, or any fetch error, `ActionService` injects a `violated_thresholds: "payment verification failed — refusing to trust claimed captured state: ..."` and forces `PolicyVerdict::Block` fail-closed *before* the combiner. The verified available balance `amount - amount_refunded` is re-checked regardless of what policy saw. Judge answer: "What stops the agent from lying?" → "We fetch the payment live; here's the audit record showing claimed vs Razorpay."

---

## Known limitations (honest list)

| Limitation | Why | Plan |
|---|---|---|
| Synthetic eval is honestly synthetic | Held-out numbers are machinery checks; not production fraud rates | Labels from `dataset-gen` by construction — discounted accordingly in README scope line |
| Verified payment requires live keys | `MockGateway` (no `RAZORPAY_KEY_ID`) cannot verify — offline/CI trusts claimed context, audited as `mock_unverified` | Set `RAZORPAY_KEY_ID/SECRET` for live demo; verification is enforced whenever the gateway is live |

| Limitation | Why | Plan |
|---|---|---|
| Evidence store is hybrid (PG exists, workers still file-seeded in compose) | Postgres-backed `EvidenceStore` built, JSON seeding retained for demo ergonomics | Seed via DB in prod profile |
| Single decision-owner process | `RwLock<HashMap>` + advisory lock for audit; gateway dedup has durable layer but not DB-backed | `FOR UPDATE` + Postgres idempotency table for multi-replica |
| Audit anchor requires external publishing | HMAC computed and exposed; caller must ship `audit_anchor.head_hash + hmac` to immutable sink | Sidecar that POSTs anchor to external log every N records |
