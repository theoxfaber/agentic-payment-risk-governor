# AI Design — why the intelligence layer is built this way

This document records the research and reasoning behind the learned scoring
layer, the conformal calibration of its thresholds, and the deliberate role
boundaries between deterministic code, statistical guarantees, LLMs, and
humans. It exists because "we called an LLM" is not an AI strategy for a
system that moves money.

---

## 1. The problem with "let the model decide"

A risk governor makes irreversible financial decisions. Three failure modes
disqualify the naive approach (prompt an LLM, act on its answer):

1. **No calibration.** An LLM's self-assessed confidence is not a probability.
   Fraud decisions need calibrated scores — the downstream economics (see §4)
   are meaningless otherwise.
2. **Unfaithful explanations.** Measured work on LLM self-explanation vs SHAP
   on tabular financial classification shows the model's stated reasons diverge
   from its actual drivers (AlMarri et al., arXiv:2512.00163). An audit trail
   that quotes unfaithful reasoning is worse than none.
3. **Silent drift on upgrade.** Swap the model behind an API and your decision
   boundary changes invisibly. A versioned weight artifact changes it *visibly*,
   in a diff you can roll back.

Industry practice agrees: Stripe Radar is a gradient-boosted ensemble over
engineered features with nightly retraining; banks deploy constrained logistic
regression under interpretability rules; the operational pattern for regulated
finance is **"predict with the model, calculate with code, decide by policy."**

## 2. What we actually built

### 2.1 Learned scorer: pure-Rust logistic regression (`eval-harness/src/lr.rs`)

- Full-batch gradient descent, L2-regularized, class-weighted (fraud is rare;
  class weighting beats oversampling for calibration).
- Deterministic: no randomness anywhere in training; identical inputs produce
  byte-identical weights (pinned by test).
- Trains on **calibration worlds only** (seed 2026). Held-out seeds are scored
  once by code that never saw them.
- Ships as a small JSON artifact (`eval-harness/artifacts/lr_model.json`)
  containing weights + standardization stats + version string — the deployable,
  diffable, rollbackable unit.

Why logistic regression and not a deep net: the fraud literature is consistent
that **features carry most of the value** (IEEE-CIS winners' writeups; Stripe
Radar engineering blog), LR stays competitive after good features, and a
calibrated probability is exactly what §4's economics require. The interface
(`predict(&[f64]) -> f64`) is deliberately the same one a monotonic-constraint
GBDT would expose if recall ever plateaus.

### 2.2 Features (`eval-harness/src/learned.rs`)

Chosen from the consensus of IEEE-CIS Fraud Detection winners, Stripe Radar,
and PayPal risk-platform writeups:

| Feature | Why |
|---|---|
| `return_refund_rate` | max() not sum(): returns/refunds usually describe one money event; naive summation torches precision on normal customers |
| `log_account_age_days` | new-account + high velocity = the classic abuse signature |
| `distinct_merchants_norm`, `breadth_norm` | ring members concentrate on few merchants; narrow activity breadth |
| `dispute_ratio` | direct loss signal |
| `sync_share_72h` | synchronized purchase→return timing — the ring tell |
| `cluster_size_norm`, `cluster_pooled_return_rate` | structural context from the entity graph — the graph's entire value, as a number the model can weigh |

Raw identifiers are never features — only windowed aggregates (the IEEE-CIS
"UID" lesson: unseen entities at serving time).

### 2.3 Thresholds with statistics behind them (`eval-harness/src/conformal.rs`)

The REVIEW band is no longer a magic number. Following split conformal
prediction (Vovk et al. 2005; Papadopoulos et al. 2002) and Conformal Risk
Control (Angelopoulos & Bates, arXiv:2107.07511; arXiv:2208.02814, ICLR 2024),
two budgets are declared as **policy**, and the thresholds are derived:

- `α_leak` — acceptable fraud leaking through auto-allow (default 2%)
- `α_friction` — acceptable legitimate traffic auto-blocked (default 1%)

Under exchangeability between calibration and serving data, these bounds hold
**with finite-sample, distribution-free validity** — no distributional or
model assumptions. The only thing anyone has to defend in an audit is two
business numbers.

### 2.4 Instance-dependent economics (Elkan / Bahnsen / Carbajal)

Pure global thresholds waste money in both directions. Following Elkan's
foundational result (IJCAI 2001) and cost-sensitive thresholding work
(Bahnsen et al.; Höppner et al. EJOR 2022; Carbajal et al.'s 2-D `(p̂, amount)`
decision region), the calibrated detector decides per-instance:

```
expected_loss = p̂ × exposure          # what allowing costs if we're wrong
review_cost   = ₹400                  # what certainty costs

expected_loss ≤ review_cost           → CLEAR    (cheap to be wrong)
expected_loss > review_cost ∧ p̂ ≥ τ_block → AUTO-BLOCK (friction-budgeted)
otherwise                             → HUMAN REVIEW
```

Held-out result: the calibrated detector concedes ₹13.50 of prevented value
versus the hand-tuned investigation engine — because spending ₹400 of human
time to prevent ₹13.50 is bad economics, and the system knows it. That single
number is the honest cost of automation, visible and bounded per instance.

### 2.5 Drift monitoring (`governor-server/src/state.rs`)

Conformal guarantees hold only while serving data stays exchangeable with
calibration data — so drift monitoring is not optional. The server keeps a
risk-score histogram and exports PSI (Population Stability Index) against a
reference distribution (`SCORE_REFERENCE_JSON`). Score-distribution drift is
the earliest observable signal while fraud labels are still maturing (Dal
Pozzolo et al., TNNLS 2017). Folk cutoffs (PSI > 0.25) are known to be
sample-size-biased (Yurdakul & Naranjo 2021); alert thresholds belong to the
monitoring layer, calibrated to n.

## 2.6 Guarantee validation that exercises the bound (`eval-harness/src/stress.rs`)

On clean synthetic worlds the budgets never bind (`tau_block` lands at 1.0) — which proves
nothing: a coverage bound that never binds cannot be validated. The stress harness fixes
this by **camouflaging abusers toward the legitimate manifold** (account ages, merchant/product
diversity, and timing redrawn from the background distributions while return/refund *rates*
stay elevated — a partially-overlapping signal, like an adaptive adversary who mimics everything
except the behavior that generates their payout). Per independent run, thresholds are calibrated
on a camouflaged cohort (5 worlds, same mixture as deployment — exchangeability requirement)
and checked on a fresh camouflaged held-out world with the raw CRC rule:

```
score ≤ tau_clear  → CLEAR            (P(leaked abuser) ≤ α_leak, unconditionally)
score ≥ tau_block  → AUTO-BLOCK       (P(blocked legit) ≤ α_friction, unconditionally)
otherwise          → HUMAN REVIEW
```

Results over 12 runs (2 616 customers, held-out headline kept separate):

- **leaked 54 vs α_leak·N=52, z=0.23 — HOLDS**; worst run 4.1% leak, best 0.5%
- **blocked-legit 31 vs α_friction·N=26, z=0.95 — HOLDS**; worst run 2.8% friction
- tau_clear collapses 0.23→~0.04 and review share rises to ~36% — exactly the designed
  conservative response when scores overlap and uncertainty is real
- conditional view for context: ~25% of abusers would be missed if CLEAR were score-only,
  but the *unconditional* leak (the quantity the bound governs) stays at budget —
  budgets are global-rate budgets, not per-abuser recall budgets (base rate matters)
- mean PSI clean→camouflage ≈0.99 — the score histogram already fires its tripwire before
  any label could; without PSI, the exchangeability breach would be invisible for months

Lesson seeded by a failed first experiment: calibrating on a mixed cohort and testing on
ReturnAbuse-only blew the budget with 87% leak (z ≫ 2). The fix was making the cohort's
mixture match deployment — thresholds must be calibrated on traffic that *resembles* the
traffic they will govern. This is the evaluation analog of the "+1" exchangeability point
above. CI gates mean leak/friction via a pooled binomial z-test, not a naive `mean ≤ α`.

## 3. The LLM's actual job

LLMs are used for exactly one thing: extracting structured claims from the
agent's free-text `declared_intent` (amount claimed, action type hinted,
urgency language). Constraints:

- **Evidence only, never decider.** Claims feed `intent_mismatch_score`,
  which can force HUMAN REVIEW but can never lower risk below policy
  boundaries or fire an allow. Verified against hard request fields downstream.
- **Hardened prompt surface.** Agent-controlled text is sanitized before it
  enters any prompt: control characters stripped, code fences neutralized,
  role-injection openers escaped case-insensitively, hard length cap, and the
  payload delimited inside `<declared_intent>` tags with explicit instructions
  to treat it as data (`intent-engine::sanitize_intent`). A successful
  injection can therefore only produce claims that get cross-checked against
  the request anyway.
- **Deterministic fallback.** No key configured → heuristic extractor. LLM
  timeout/error → heuristic fallback flagged `degraded=true`, so evidence
  quality drops are visible in the audit trail.
- This matches Razorpay's own MCP-server positioning: agents get tools;
  money movement still flows through governed APIs.

## 3.1 Judge-regenerable held-out evaluation

Headline numbers recompute against externally supplied seeds:

```bash
EVAL_HELDOUT_SEEDS=12345,67890 cargo run --release -p eval-harness  # headline from worlds this repo has never seen
```

The harness prints whether seeds are "committed defaults" or "externally supplied". A skeptical
verifier does not have to trust the held-out claim — they supply their own seeds and watch
coverage hold. The published bead for the marketplace glyph is the artifact of the correct run.

## 4. Production path (what changes, what doesn't)

The pipeline is already wired for the production loop:

1. **Label maturation:** `OutcomeRecorded` audit events + Razorpay webhooks
   (`chargeback.*`, refund outcomes) mature labels for scored actions. Fraud
   labels lag ~30–120 days (Dal Pozzolo's maturity convention).
2. **Retraining:** same trainer, sliding window of matured labels, recent-
   weighted. Trigger-based (on PSI alarm) beats fixed schedules; Stripe's own
   data says freshness alone was worth ~0.5pp recall/month.
3. **Recalibration:** CRC thresholds recomputed on each new calibration
   cohort; the two budgets (α_leak, α_friction) are the merchant's policy
   knobs, unchanged.
4. **Deployment gate:** new artifact only ships if held-out metrics hold and
   score-distribution PSI vs the previous artifact is within bounds.
5. **Reviewer feedback loop:** human review outcomes are a biased sample
   (selected on high risk) — trained as a separate signal, never merged into
   the main labeled set (Dal Pozzolo's sample-selection-bias result).

## 5. Honest limits

- Training/calibration data is synthetic. It proves the *machinery*
  (train → calibrate → guarantee → monitor) and that the feature set carries
  signal; it does not predict production performance.
- The conformal guarantee is marginal, not conditional (impossible in
  general — Vovk); segment-level recalibration is the mitigation.
- Since `3748727` the learned detector **is wired into the live pipeline** (`action-service` scores every request via `DefaultLearnedScorer` before `DecisionMade`, escalating `ALLOW → REVIEW/BLOCK` on `p̂ × amount > ₹400` within the conformal band, and the HTTP layer only persists/audits the final outcome). Before that revision the HTTP layer rewrote the response after execution (now fixed; see `docs/BUGS.md #19`). Webhook-driven label maturation (§4-1) remains the next milestone for *retraining* on real outcomes, but the live gate is no longer “future”.

## References

- Vovk, Gammerman, Saunders — *Machine Learning* (1999); Vovk et al.,
  *Algorithmic Learning in a Random World* (2005)
- Papadopoulos et al. — Inductive/inductive conformal prediction (2002)
- Angelopoulos & Bates — *Conformal Prediction: A Gentle Introduction*,
  FnTML 16(4), 2023, arXiv:2107.07511
- Angelopoulos, Bates, Fisch, Lei, Schuster — *Conformal Risk Control*,
  ICLR 2024, arXiv:2208.02814
- Linusson et al. — *Classification with Reject Option using Conformal
  Prediction*, PAKDD 2018
- Hallberg Szabadváry et al. — Distribution-free error guarantees for
  classification with reject option, arXiv:2506.21802
- Elkan — *The Foundations of Cost-Sensitive Learning*, IJCAI 2001
- Höppner, Baesens, Verbeke, Verdonck — *Instance-dependent cost-sensitive
  learning for detecting transfer fraud* (cslogit/csboost), EJOR 297(1), 2022
- Bahnsen et al. — example-dependent cost matrices, 2014–2017
- Carbajal, Cao, Vilar — Cost-sensitive thresholding over a 2-D decision
  region for fraud detection
- Dal Pozzolo — *Adaptive Machine Learning for Credit Card Fraud Detection*,
  PhD thesis, ULB 2015; Dal Pozzolo et al., TNNLS 2017
- Yurdakul & Naranjo — Statistical properties of PSI, J. Risk Model
  Validation, 2021
- Tortorella (2000); Hendrickx et al. — *ML with a reject option: a survey*,
  Machine Learning 113, 2024
- Stripe Engineering — *How we built it: Stripe Radar*; *Similarity
  clustering* blog (pairwise model → connected components → analyst queues)
- IEEE-CIS Fraud Detection Kaggle winning solutions (feature/UID methodology)
