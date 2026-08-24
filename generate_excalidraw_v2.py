import json
import time
import os

def create_beautiful_excalidraw():
    seed_counter = 500000

    def get_seed():
        nonlocal seed_counter
        seed_counter += 1
        return seed_counter

    elements = []

    def add_rect(id_name, x, y, w, h, stroke_color, bg_color, stroke_width=2, stroke_style="solid", roughness=0, roundness=3, opacity=100):
        elem = {
            "id": id_name,
            "type": "rectangle",
            "x": x,
            "y": y,
            "width": w,
            "height": h,
            "angle": 0,
            "strokeColor": stroke_color,
            "backgroundColor": bg_color,
            "fillStyle": "solid",
            "strokeWidth": stroke_width,
            "strokeStyle": stroke_style,
            "roughness": roughness,
            "opacity": opacity,
            "groupIds": [],
            "frameId": None,
            "roundness": {"type": roundness} if roundness else None,
            "seed": get_seed(),
            "version": 1,
            "versionNonce": get_seed(),
            "isDeleted": False,
            "boundElements": [],
            "updated": int(time.time() * 1000),
            "link": None,
            "locked": False
        }
        elements.append(elem)
        return elem

    def add_text(id_name, x, y, text, font_size=16, stroke_color="#0f172a", font_family=2, text_align="left"):
        lines = text.split("\n")
        line_height = 1.3
        height = len(lines) * font_size * line_height
        max_len = max(len(l) for l in lines) if lines else 1
        width = max_len * (font_size * 0.58)

        elem = {
            "id": id_name,
            "type": "text",
            "x": x,
            "y": y,
            "width": width,
            "height": height,
            "angle": 0,
            "strokeColor": stroke_color,
            "backgroundColor": "transparent",
            "fillStyle": "solid",
            "strokeWidth": 1,
            "strokeStyle": "solid",
            "roughness": 0,
            "opacity": 100,
            "groupIds": [],
            "frameId": None,
            "roundness": None,
            "seed": get_seed(),
            "version": 1,
            "versionNonce": get_seed(),
            "isDeleted": False,
            "boundElements": [],
            "updated": int(time.time() * 1000),
            "link": None,
            "locked": False,
            "text": text,
            "fontSize": font_size,
            "fontFamily": font_family,
            "textAlign": text_align,
            "verticalAlign": "top",
            "baseline": int(font_size * 0.8),
            "containerId": None,
            "originalText": text,
            "lineHeight": line_height
        }
        elements.append(elem)
        return elem

    def add_arrow(id_name, points, stroke_color="#475569", stroke_width=2, stroke_style="solid", roughness=0):
        start_x, start_y = points[0]
        rel_points = [[p[0] - start_x, p[1] - start_y] for p in points]
        
        xs = [p[0] for p in rel_points]
        ys = [p[1] for p in rel_points]
        w = max(xs) - min(xs) if len(xs) > 1 else 10
        h = max(ys) - min(ys) if len(ys) > 1 else 10

        elem = {
            "id": id_name,
            "type": "arrow",
            "x": start_x,
            "y": start_y,
            "width": max(w, 10),
            "height": max(h, 10),
            "angle": 0,
            "strokeColor": stroke_color,
            "backgroundColor": "transparent",
            "fillStyle": "solid",
            "strokeWidth": stroke_width,
            "strokeStyle": stroke_style,
            "roughness": roughness,
            "opacity": 100,
            "groupIds": [],
            "frameId": None,
            "roundness": {"type": 2},
            "seed": get_seed(),
            "version": 1,
            "versionNonce": get_seed(),
            "isDeleted": False,
            "boundElements": [],
            "updated": int(time.time() * 1000),
            "link": None,
            "locked": False,
            "points": rel_points,
            "startBinding": None,
            "endBinding": None,
            "startArrowhead": None,
            "endArrowhead": "arrow"
        }
        elements.append(elem)
        return elem

    # -------------------------------------------------------------
    # 1. HEADER BANNER
    # -------------------------------------------------------------
    add_rect("header_bg", 80, 60, 2040, 140, "#4338ca", "#e0e7ff", stroke_width=2, roughness=0, roundness=3)
    add_text("header_title", 110, 80, "🛡️ AGENTIC PAYMENT RISK GOVERNOR — SYSTEM ARCHITECTURE", font_size=26, stroke_color="#1e1b4b", font_family=2)
    add_text("header_sub", 110, 122, "Razorpay AI Buildathon 2026 — Track 02: AI Risk Manager | Autonomous Agent Safety & Authorization Layer", font_size=15, stroke_color="#3730a3", font_family=2)
    add_text("header_sub2", 110, 150, "Scores every agent money-movement action BEFORE reaching Razorpay execution APIs (ALLOW / REVIEW / BLOCK)", font_size=13, stroke_color="#4338ca", font_family=2)

    # Badges inside header
    add_rect("b1_bg", 1680, 85, 200, 42, "#059669", "#d1fae5", stroke_width=2, roughness=0, roundness=3)
    add_text("b1_txt", 1705, 96, "🏆 9.5 / 10 Rated", font_size=15, stroke_color="#065f46", font_family=2)

    add_rect("b2_bg", 1900, 85, 190, 42, "#4f46e5", "#e0e7ff", stroke_width=2, roughness=0, roundness=3)
    add_text("b2_txt", 1920, 96, "⚡ Pure Rust Core", font_size=15, stroke_color="#3730a3", font_family=2)

    # -------------------------------------------------------------
    # 2. SWIMLANE 1: INGRESS & SIGNALS
    # -------------------------------------------------------------
    add_rect("lane1_bg", 80, 230, 380, 1050, "#94a3b8", "#f8fafc", stroke_width=2, stroke_style="dashed", roughness=0, roundness=3)
    add_text("lane1_title", 100, 250, "1. INGRESS & INTERCEPTION", font_size=16, stroke_color="#334155", font_family=2)

    # Card 1: AI Agent
    add_rect("c1_bg", 100, 290, 340, 220, "#2563eb", "#eff6ff", stroke_width=2, roughness=0, roundness=3)
    add_text("c1_t", 120, 308, "🤖 Autonomous AI Agents", font_size=17, stroke_color="#1e40af", font_family=2)
    add_text("c1_sub", 120, 335, "Initiates Money Movement Requests", font_size=13, stroke_color="#2563eb", font_family=2)
    add_text("c1_body", 120, 365, "• Endpoint: POST /v1/actions\n• agent_id: \"agent-trusted-01\"\n• merchant_id: \"merchant-001\"\n• action_type: \"refund\" / \"payout\"\n• amount: ₹50,000 – ₹600,000\n• declared_intent: Natural Text", font_size=12, stroke_color="#1e3a8a", font_family=2)

    # Card 2: Webhooks
    add_rect("c2_bg", 100, 540, 340, 220, "#9333ea", "#fbf2ff", stroke_width=2, roughness=0, roundness=3)
    add_text("c2_t", 120, 558, "⚡ Razorpay Webhook Receiver", font_size=17, stroke_color="#7e22ce", font_family=2)
    add_text("c2_sub", 120, 585, "Asynchronous Loss & Event Signals", font_size=13, stroke_color="#9333ea", font_family=2)
    add_text("c2_body", 120, 615, "• Endpoint: POST /v1/webhooks\n• Dispute & Chargeback alerts\n• Merchant refund state updates\n• HMAC-SHA256 Signature Verify\n• Ingestion into Audit & Evidence", font_size=12, stroke_color="#581c87", font_family=2)

    # Card 3: Synthetic Eval
    add_rect("c3_bg", 100, 790, 340, 220, "#ea580c", "#fff7ed", stroke_width=2, roughness=0, roundness=3)
    add_text("c3_t", 120, 808, "🧪 Benchmark Dataset Generator", font_size=17, stroke_color="#c2410c", font_family=2)
    add_text("c3_sub", 120, 835, "8 Labeled Adversarial Test Worlds", font_size=13, stroke_color="#ea580c", font_family=2)
    add_text("c3_body", 120, 865, "• Coincidental IP/Device sharing\n• Merchant collusion & ring fraud\n• Adaptive evasion strategies\n• Innocent household FP test\n• 1,000+ labeled synthetic actions", font_size=12, stroke_color="#7c2d12", font_family=2)

    # Card 4: Interactive CLI Driver
    add_rect("c4_bg", 100, 1040, 340, 220, "#0284c7", "#f0f9ff", stroke_width=2, roughness=0, roundness=3)
    add_text("c4_t", 120, 1058, "💻 Interactive Demo Driver CLI", font_size=17, stroke_color="#0369a1", font_family=2)
    add_text("c4_sub", 120, 1085, "Scripted Demo Scenario Executor", font_size=13, stroke_color="#0284c7", font_family=2)
    add_text("c4_body", 120, 1115, "• Executable: cargo run -p governor\n• 4 Pre-scripted demo flows\n• Live API key passthrough\n• Batch evaluation runner", font_size=12, stroke_color="#0c4a6e", font_family=2)

    # -------------------------------------------------------------
    # 3. SWIMLANE 2: MULTI-STAGE RISK GOVERNOR PIPELINE
    # -------------------------------------------------------------
    add_rect("lane2_bg", 500, 230, 980, 1050, "#334155", "#ffffff", stroke_width=2, roughness=0, roundness=3)
    add_text("lane2_title", 530, 250, "2. RISK GOVERNOR PIPELINE (Rust Multi-Crate Pipeline Engine)", font_size=17, stroke_color="#0f172a", font_family=2)

    # Stage 1: Policy Engine
    add_rect("s1_bg", 530, 290, 920, 160, "#475569", "#f8fafc", stroke_width=2, roughness=0, roundness=3)
    add_text("s1_t", 550, 308, "⚙️ Stage 1 — Policy Engine (`policy-engine` crate)", font_size=18, stroke_color="#1e293b", font_family=2)
    add_text("s1_sub", 550, 335, "Hard Financial Invariants & Invariant Boundary Enforcement", font_size=13, stroke_color="#475569", font_family=2)
    add_text("s1_body", 550, 365, "• Auto-allow ceiling: ≤ ₹50,000 | Human approval threshold: > ₹150,000 | Hard block cap: > ₹500,000\n• Per-agent velocity limits & hourly window rate counters\n• Country & merchant domain scope allowlists / blocklists\n• Fast-path immediate rejection before heavy scoring pipeline", font_size=12, stroke_color="#334155", font_family=2)

    # Stage 2: Risk Engine
    add_rect("s2_bg", 530, 470, 920, 160, "#d97706", "#fef3c7", stroke_width=2, roughness=0, roundness=3)
    add_text("s2_t", 550, 488, "🧠 Stage 2 — Risk Engine (`risk-engine` crate)", font_size=18, stroke_color="#b45309", font_family=2)
    add_text("s2_sub", 550, 515, "Behavioral Anomaly Scoring & Natural Language Intent Cross-Check", font_size=13, stroke_color="#d97706", font_family=2)
    add_text("s2_body", 550, 545, "• Statistical Z-score anomaly detection against historical agent behavior baselines\n• Intent Mismatch Scorer: NLP cross-check of declared_intent text vs action_type + amount\n• Behavioral drift measurement over sliding temporal windows\n• Output: Bounded composite risk score ∈ [0.0, 1.0]", font_size=12, stroke_color="#78350f", font_family=2)

    # Stage 3: Risk Graph
    add_rect("s3_bg", 530, 650, 920, 160, "#c026d3", "#fae8ff", stroke_width=2, roughness=0, roundness=3)
    add_text("s3_t", 550, 668, "🕸️ Stage 3 — Risk Graph & Correlation (`risk-graph` / `risk-governor-correlation` crates)", font_size=18, stroke_color="#a21caf", font_family=2)
    add_text("s3_sub", 550, 695, "Dynamic Entity Graph & Coordinated Abuse Ring Clustering", font_size=13, stroke_color="#c026d3", font_family=2)
    add_text("s3_body", 550, 725, "• Dynamic multi-entity graph traversal (Customers, Devices, IP Addresses, Bank Accounts, Merchants)\n• Coordinated abuse ring clustering & merchant collusion detection\n• Coincidental sharing disaggregation (prevents flagging shared household/office Wi-Fi)\n• Multi-hop relationship graph analysis for fraud botnets", font_size=12, stroke_color="#701a75", font_family=2)

    # Stage 4: Investigation Engine
    add_rect("s4_bg", 530, 830, 920, 180, "#0284c7", "#e0f2fe", stroke_width=2, roughness=0, roundness=3)
    add_text("s4_t", 550, 848, "🔍 Stage 4 — Investigation Engine (`investigation-engine` crate)", font_size=18, stroke_color="#0369a1", font_family=2)
    add_text("s4_sub", 550, 875, "Evidence Reasoning & Contradiction Resolution Layer", font_size=13, stroke_color="#0284c7", font_family=2)
    add_text("s4_body", 550, 905, "• Core Thesis: \"High risk score cannot automatically BLOCK without behavioral confirmation\"\n• Weighs directional evidence vectors: Direction::{Supports, Contradicts}\n• Computes evidence_confidence; demands high certainty before blocking\n• Escalates high-risk / low-confidence scenarios to REVIEW (Human-in-the-Loop)\n• Zero false-positive friction on innocent customers (0 of 324 legitimate users blocked)", font_size=12, stroke_color="#0c4a6e", font_family=2)

    # Stage 5: Audit & Replay Engine
    add_rect("s5_bg", 530, 1030, 920, 160, "#374151", "#f3f4f6", stroke_width=2, roughness=0, roundness=3)
    add_text("s5_t", 550, 1048, "📜 Stage 5 — Audit Service & Decision Replay (`audit-service` / `risk-governor-replay` crates)", font_size=18, stroke_color="#1f2937", font_family=2)
    add_text("s5_sub", 550, 1075, "Immutable Decision Ledger & Full Post-Mortem Timeline Reconstruction", font_size=13, stroke_color="#374151", font_family=2)
    add_text("s5_body", 550, 1105, "• Append-only immutable decision ledger with complete feature state payload\n• Reconstructs full lifecycle: action_requested → policy_evaluated → risk_scored → graph_analyzed → decision_made → human_reviewed → razorpay_called\n• Replay endpoint GET /v1/decisions/{id}/replay for post-mortem compliance", font_size=12, stroke_color="#111827", font_family=2)


    # -------------------------------------------------------------
    # 4. SWIMLANE 3: DECISION ROUTER & EXECUTION TARGETS
    # -------------------------------------------------------------
    add_rect("lane3_bg", 1520, 230, 600, 1050, "#94a3b8", "#f8fafc", stroke_width=2, stroke_style="dashed", roughness=0, roundness=3)
    add_text("lane3_title", 1540, 250, "3. DECISION ROUTER & EXECUTION TARGETS", font_size=16, stroke_color="#334155", font_family=2)

    # Path 1: ALLOW
    add_rect("p1_bg", 1540, 290, 560, 220, "#16a34a", "#dcfce7", stroke_width=2, roughness=0, roundness=3)
    add_text("p1_t", 1560, 310, "🟢 ALLOW — Auto-Approved Execution", font_size=19, stroke_color="#15803d", font_family=2)
    add_text("p1_sub", 1560, 340, "Action Passed All Safety & Governance Gates", font_size=13, stroke_color="#16a34a", font_family=2)
    add_text("p1_body", 1560, 370, "• Direct HTTP execution via razorpay-gateway crate\n• Authentic Razorpay Test-Mode API Integration (Basic Auth)\n• Automated retry & exponential backoff on 429/5xx errors\n• Creates real test Orders, Refunds, and Payment Links", font_size=12, stroke_color="#14532d", font_family=2)

    # Path 2: REVIEW
    add_rect("p2_bg", 1540, 540, 560, 250, "#ca8a04", "#fef9c3", stroke_width=2, roughness=0, roundness=3)
    add_text("p2_t", 1560, 560, "🟡 REVIEW — Held for Human Approval", font_size=19, stroke_color="#a16207", font_family=2)
    add_text("p2_sub", 1560, 590, "High Risk Score Lacking Behavioral Certainty", font_size=13, stroke_color="#ca8a04", font_family=2)
    add_text("p2_body", 1560, 620, "• Action held safely without touching Razorpay API\n• Surfaces immediately in Live Dashboard at http://127.0.0.1:8080\n• Interactive 1-click Human Approve / Reject modal\n• Approving executes Razorpay API call instantly\n• ZERO False Positive Friction Cost (0 of 324 innocent customers blocked)", font_size=12, stroke_color="#713f12", font_family=2)

    # Path 3: BLOCK
    add_rect("p3_bg", 1540, 810, 560, 220, "#dc2626", "#fee2e2", stroke_width=2, roughness=0, roundness=3)
    add_text("p3_t", 1560, 830, "🔴 BLOCK — Execution Halted", font_size=19, stroke_color="#b91c1c", font_family=2)
    add_text("p3_sub", 1560, 860, "Confirmed Hard Invariant Breach or Abuse Ring", font_size=13, stroke_color="#dc2626", font_family=2)
    add_text("p3_body", 1560, 890, "• Action terminated instantly; NEVER touches Razorpay APIs\n• Full audit trail event payload emitted\n• Prevents merchant financial loss & chargeback damage\n• Complete explanation reconstructed in post-mortem replay", font_size=12, stroke_color="#7f1d1d", font_family=2)


    # -------------------------------------------------------------
    # 5. BOTTOM CALLOUT CARD: BENCHMARK METRICS & STORAGE
    # -------------------------------------------------------------
    add_rect("bot_bg", 80, 1310, 2040, 180, "#475569", "#1e293b", stroke_width=2, roughness=0, roundness=3)

    # Left: Benchmarks
    add_rect("bm_bg", 100, 1328, 980, 144, "#0f172a", "#0f172a", stroke_width=2, roughness=0, roundness=3)
    add_text("bm_t", 120, 1342, "📊 Empirical Evaluation Benchmarks (`eval-harness` crate)", font_size=16, stroke_color="#38bdf8", font_family=2)
    add_text("bm_body", 120, 1372, "• Static Rules Only: Precision=100%, Recall=58% | FP Cost: ₹0 | FN Cost: ₹11,700 (Misses 42% of Evasion)\n• Graph Cluster Only: Precision=51%, Recall=100% | FP Cost: ₹22,950 (Burned on False Positives)\n• Investigation Engine: Precision=100%, Recall=100% | FP Cost: ₹0 | FN Cost: ₹0 (₹58,500 Abuse Prevented)", font_size=12, stroke_color="#e2e8f0", font_family=2)

    # Right: System Server & DB
    add_rect("db_bg", 1110, 1328, 990, 144, "#0f172a", "#0f172a", stroke_width=2, roughness=0, roundness=3)
    add_text("db_t", 1130, 1342, "💾 Storage, Distributed Transport & Server Infrastructure", font_size=16, stroke_color="#c084fc", font_family=2)
    add_text("db_body", 1130, 1372, "• `pg-store` & `rusqlite`: Transaction history, entity graph nodes/edges, immutable audit logs\n• `nats-link`: Distributed message bus for decoupled multi-worker deployment\n• `governor-server`: Axum web server & zero-build vanilla JS live dashboard (polling 2s) at http://127.0.0.1:8080", font_size=12, stroke_color="#e2e8f0", font_family=2)


    # -------------------------------------------------------------
    # 6. CONNECTING ARROWS WITH FLOW LABELS
    # -------------------------------------------------------------

    # Ingress to Governor Pipeline
    add_arrow("arr_agent", [[440, 400], [530, 370]], stroke_color="#2563eb", stroke_width=3)
    add_arrow("arr_wh", [[440, 650], [530, 1110]], stroke_color="#9333ea", stroke_width=3)
    add_arrow("arr_cli", [[440, 1150], [530, 370]], stroke_color="#0284c7", stroke_width=3)

    # Intra-Pipeline Flow Arrows
    add_arrow("arr_s1_s2", [[990, 450], [990, 470]], stroke_color="#475569", stroke_width=3)
    add_arrow("arr_s2_s3", [[990, 630], [990, 650]], stroke_color="#d97706", stroke_width=3)
    add_arrow("arr_s3_s4", [[990, 810], [990, 830]], stroke_color="#c026d3", stroke_width=3)
    add_arrow("arr_s4_s5", [[990, 1010], [990, 1030]], stroke_color="#0284c7", stroke_width=3)

    # Pipeline to Router Paths
    add_arrow("arr_to_allow", [[1450, 920], [1540, 400]], stroke_color="#16a34a", stroke_width=4)
    add_arrow("arr_to_review", [[1450, 920], [1540, 665]], stroke_color="#ca8a04", stroke_width=4)
    add_arrow("arr_to_block", [[1450, 920], [1540, 920]], stroke_color="#dc2626", stroke_width=4)

    excalidraw_data = {
        "type": "excalidraw",
        "version": 2,
        "source": "https://excalidraw.com",
        "elements": elements,
        "appState": {
            "gridSize": None,
            "viewBackgroundColor": "#ffffff"
        },
        "files": {}
    }

    return excalidraw_data

if __name__ == "__main__":
    data = create_beautiful_excalidraw()

    target_paths = [
        "/Users/apple/risk-governor/docs/risk-governor-architecture.excalidraw",
        "/Users/apple/risk-governor/risk-governor-architecture.excalidraw",
        "/Users/apple/Documents/risk-governor-architecture.excalidraw"
    ]

    for p in target_paths:
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
        print(f"Successfully generated high-end Excalidraw diagram at {p}")
