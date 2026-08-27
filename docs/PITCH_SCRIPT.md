# 5-Minute Technical Pitch & Architecture Demo Script

**Project**: Agentic Payment Risk Governor  
**Track**: AI Risk Manager / AI Finance Controller (Open Infrastructure)  
**Presenter**: Lead Systems & AI Engineer  

---

### Segment 1: The Problem & Architectural Paradigm (0:00 – 1:00)

**[Visual Setup]**: Full screen showing system architecture diagram: Sandboxed LLM $\to$ Rust Governor $\to$ Razorpay Gateway.

> **Spoken Voiceover**:  
> "Every team building AI for fintech makes the same catastrophic mistake: they give an LLM direct execution authority over financial tools. When an un-governed agent encounters a network timeout, it hallucinates a retry. When it receives a duplicate webhook, it creates a duplicate charge.
>
> In payments, probabilistic reasoning must never touch physical execution.
>
> We built the **Agentic Payment Risk Governor**—a high-throughput, non-bypassable control plane written in Rust. It treats GenAI models strictly as advisory diagnostic agents, enforcing mathematical invariants, Conformal Risk Control, and atomic idempotency locks before any call reaches the Razorpay API."

---

### Segment 2: "What Broke at 2 AM" — Live Concurrency & Failure Recovery (1:00 – 2:30)

**[Visual Setup]**: Split screen. Left: Terminal executing `cargo test --test test_adversarial_concurrency`. Right: Real-time event log viewer.

> **Spoken Voiceover**:  
> "Let's watch what happens when production fails under stress.
>
> In this live test, 10 concurrent worker threads fire simultaneous payment requests sharing the exact same idempotency payload. 
>
> Notice the trace: Worker 0 acquires the atomic database lease and dispatches to the Razorpay API. Workers 1 through 9 are intercepted instantly by the Governor's memory-safe state machine. They receive cached, deduplicated responses. Downstream Razorpay API calls: exactly one. Duplicate charges: zero.
>
> Next, we simulate an upstream issuer degradation where the bank drops into a 5xx retry loop. The Governor's circuit breaker detects the latency spike, aborts before the 2,000ms gateway SLA timeout, and automatically routes the transaction to an alternate UPI rail without user intervention."

---

### Segment 3: Economic Proof & Conformal Risk Control (2:30 – 4:00)

**[Visual Setup]**: Screen displaying `BENCHMARK.md` charts, comparative tables, and CRC calibration curves.

> **Spoken Voiceover**:  
> "Traditional fraud systems use static thresholds like 'score above 0.5'. But in payments, missing a ₹50,000 fraud costs ten times more than challenging a genuine customer.
>
> We implemented **Conformal Risk Control** based on distribution-free statistical bounds. Across 30,000 held-out transactions over three independent seeds, our governor delivers:
> - **₹8.49 Million** in Net Recovered Value—outperforming un-governed agents by ₹10.3M.
> - **98.6% Fraud Recall** with only **1.8% False Positive friction**.
> - A mathematically guaranteed fraud leakage bound of less than 2%, verified with 99% statistical confidence."

---

### Segment 4: Code Quality, Audit Integrity & Submission Wrap-Up (4:00 – 5:00)

**[Visual Setup]**: VS Code showing Rust workspace crates (`governor-core`, `razorpay-gateway`, `audit-chain`, `eval-harness`), HMAC verification, and SHA-256 Merkle hash chain logs.

> **Spoken Voiceover**:  
> "Under the hood, this is production-ready systems engineering:
> 1. Strict crate modularity with zero unsafe Rust.
> 2. Tamper-evident SHA-256 audit hash chains linking every decision to its physical Razorpay API receipt.
> 3. Cryptographic HMAC-SHA256 signature verification on all Razorpay webhook ingestion.
>
> The Agentic Payment Risk Governor proves that you don't have to choose between AI intelligence and financial safety. 
>
> Thank you."
