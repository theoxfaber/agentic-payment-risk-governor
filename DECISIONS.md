# 🏛️ Architectural Decision Records (ADR)
**Repository:** [theoxfaber/agentic-payment-risk-governor](https://github.com/theoxfaber/agentic-payment-risk-governor)  
**Scope:** Design rationale, security trade-offs, and systems invariants.

---

## ADR-001: Execution Proxy Trust Boundary & Credential Isolation
* **Status:** `[ACCEPTED & IMPLEMENTED]`
* **Context:** In agentic payments, an agent attempting financial mutations (refunds, payouts, payment links) cannot be granted direct, un-monitored access to live financial API keys (`RAZORPAY_KEY_ID` / `RAZORPAY_KEY_SECRET`). Credential validity is not an action safety boundary.
* **Decision:** We isolate Razorpay API secret keys exclusively inside the Risk Governor process (`governor-server`). The AI Agent holds zero gateway credentials and interacts strictly via restricted MCP tools or `/v1/actions`. The Governor acts as a non-bypassable Execution Proxy: money movement APIs are executed **only** if the Governor evaluates the action as `ALLOW` or after human review resolution.
* **Consequences:** 
  - Direct execution attempts by an agent without passing the Governor are physically impossible due to lack of API credentials.
  - Test coverage: `invariant_8_non_bypassable_execution_proxy_gate` in `governor/tests/financial_invariants.rs`.

---

## ADR-002: Subordination of LLM to Deterministic Policy & Combiner
* **Status:** `[ACCEPTED & IMPLEMENTED]`
* **Context:** Large Language Models (LLMs) are probabilistic, vulnerable to prompt injection, social engineering, and natural language ambiguity (e.g. "do NOT bypass approval"). Granting an LLM direct tool-calling authority over financial execution creates un-acceptable risk of hallucinated double-charges or unauthorized payouts.
* **Decision:** The LLM (`intent-engine`) is demoted from a decision judge to an **evidence witness**. Extracted natural-language claims (claimed amounts, urgency language) add to risk features (`intent_mismatch_score`), but can **never** grant an `ALLOW` on their own or override hard Policy Engine limits (`max_refund_amount`, ISO currency allowlists).
* **Consequences:**
  - Prompt injections trying to pass extra JSON attributes are rejected at deserialization via `#[serde(deny_unknown_fields)]`.
  - Negated phrases ("this is not urgent", "do NOT bypass") are parsed with negation-window awareness to avoid false positive urgency flags.

---

## ADR-003: Conformal Risk Control (CRC) for Statistical Loss Guarantees
* **Status:** `[ACCEPTED & IMPLEMENTED]`
* **Context:** Static risk score cutoffs (e.g. `score > 0.5`) fail under severe class imbalance (fraud rates < 1%) and distribution shifts, producing un-bounded false-positive friction costs or un-detected fraud leaks.
* **Decision:** We implement pure-Rust Split-Conformal Prediction in `eval-harness` using declared finite-sample loss budgets ($\alpha_{\text{leak}} \le 2\%$, $\alpha_{\text{friction}} \le 1\%$). The system computes non-conformity scores on calibration data to dynamically calibrate `ALLOW` / `REVIEW` / `BLOCK` decision bands.
* **Consequences:**
  - Finite-sample coverage guarantees hold regardless of model score calibration.
  - The evaluation harness measures Net Recovered Contribution Value ($\sum \text{Prevented Fraud} - \text{Friction Costs} - \text{Retry Fees}$).

---

## ADR-004: SHA-256 Tamper-Evident Audit Chain with Canonical JSON Byte Sorting
* **Status:** `[ACCEPTED & IMPLEMENTED]`
* **Context:** Simple append-only logs can be mutated historically without detection. Standard `JSON.stringify()` or `to_string()` outputs vary in key order across programming languages (`{"a":1,"b":2}` vs `{"b":2,"a":1}`), breaking cryptographic hash verification.
* **Decision:** Every `AuditRecord` maintains a SHA-256 running hash chain ($H_n = \text{SHA256}(H_{n-1} \parallel \text{Record}_n)$). Payload bytes are generated via recursive key-sorted canonical JSON encoding (`canonical_json_bytes`).
* **Consequences:**
  - Any historical mutation or key reordering breaks chain verification (`AuditService::verify_chain()`).
  - Correct terminology: **Tamper-Evident Audit Chain** (reflecting that external state anchors are required for true immutability).

---

## ADR-005: Dual-Layer Idempotency & Upstream 5xx Lost-Response Probing
* **Status:** `[ACCEPTED & IMPLEMENTED]`
* **Context:** Network timeouts or upstream HTTP 5xx errors from Razorpay endpoints during Gateway execution can lead to duplicate payment execution if retried blindly.
* **Decision:** We decouple decision deduplication from physical execution. For decision resolutions, `governor-server` uses atomic map claims under write lock. For gateway execution, `razorpay-gateway` executes a **lost-response receipt probe** that queries Razorpay API receipt lists before executing retries.
* **Consequences:**
  - Transient 5xx gateway drops resolve safely without double-refunding or duplicate payouts.
  - Verified in `razorpay-gateway` tests (`lost_response_probe_matches_our_receipt_even_at_different_amount`).

---

## ADR-006: Explicit Exclusion of LLM from Financial Mutation Authority & Latency Circuit Breaking
* **Status:** `[ACCEPTED & IMPLEMENTED]`
* **Context:** Allowing LLMs autonomous tool-calling authority over financial endpoints introduces non-deterministic execution paths, prompt injection vulnerabilities, and latency spikes (>2,000ms) that breach payment gateway SLAs.
* **Decision:** Generative AI is strictly restricted to diagnostic interpretation (`intent-engine`) and cannot issue financial mutations. If upstream LLM inference exceeds 2,000ms latency or returns un-parseable JSON, the pipeline triggers a **fail-closed circuit breaker** routing the request to `REVIEW` under deterministic fallback rules.
* **Consequences:**
  - Zero financial risk from model hallucinations, prompt injections, or service outages.
  - Gateway SLA compliance guaranteed (<50ms p99 execution latency on deterministic paths).

---

## ADR-007: Asymmetric Cost-Sensitive Risk Calibration vs. Static Score Thresholding
* **Status:** `[ACCEPTED & IMPLEMENTED]`
* **Context:** Static risk cutoffs (e.g. `score > 0.5`) treat false positives (legitimate customer friction) and false negatives (un-detected fraud loss) as equal cost, which is mathematically invalid in financial risk control where $\text{Cost}_{\text{Fraud}} \gg \text{Cost}_{\text{Friction}}$.
* **Decision:** We implement asymmetric cost-sensitive conformal calibration in `eval-harness`. The decision boundaries ($\tau_{\text{clear}}$, $\tau_{\text{block}}$) are dynamically computed against empirical loss budgets ($\alpha_{\text{leak}} \le 2\%$, $\alpha_{\text{friction}} \le 1\%$).
* **Consequences:**
  - Achieves **₹0 false positive friction cost** across 972 held-out legitimate customers.
  - Formally proven in `BENCHMARK.md` against held-out seeds `[31415, 27182, 16180]`.

