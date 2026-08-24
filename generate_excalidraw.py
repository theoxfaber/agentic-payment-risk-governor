import json
import time

def create_excalidraw():
    seed_counter = 100000

    def get_seed():
        nonlocal seed_counter
        seed_counter += 1
        return seed_counter

    elements = []

    def add_rectangle(id_name, x, y, width, height, stroke_color, bg_color, stroke_width=2, stroke_style="solid", roughness=1, roundness=3):
        elem = {
            "id": id_name,
            "type": "rectangle",
            "x": x,
            "y": y,
            "width": width,
            "height": height,
            "angle": 0,
            "strokeColor": stroke_color,
            "backgroundColor": bg_color,
            "fillStyle": "solid",
            "strokeWidth": stroke_width,
            "strokeStyle": stroke_style,
            "roughness": roughness,
            "opacity": 100,
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
        line_height = 1.25
        height = len(lines) * font_size * line_height
        max_len = max(len(l) for l in lines) if lines else 1
        width = max_len * (font_size * 0.55)

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

    def add_arrow(id_name, start_x, start_y, end_x, end_y, stroke_color="#475569", stroke_width=2, stroke_style="solid", start_arrow=None, end_arrow="arrow"):
        dx = end_x - start_x
        dy = end_y - start_y

        elem = {
            "id": id_name,
            "type": "arrow",
            "x": start_x,
            "y": start_y,
            "width": abs(dx),
            "height": abs(dy),
            "angle": 0,
            "strokeColor": stroke_color,
            "backgroundColor": "transparent",
            "fillStyle": "solid",
            "strokeWidth": stroke_width,
            "strokeStyle": stroke_style,
            "roughness": 1,
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
            "points": [[0, 0], [dx, dy]],
            "startBinding": None,
            "endBinding": None,
            "startArrowhead": start_arrow,
            "endArrowhead": end_arrow
        }
        elements.append(elem)
        return elem

    # 1. Header Banner
    add_rectangle("header_bg", 60, 40, 1480, 100, "#1e293b", "#f8fafc", stroke_width=2, roughness=0, roundness=3)
    add_text("header_title", 80, 55, "AGENTIC PAYMENT RISK GOVERNOR — SYSTEM ARCHITECTURE", font_size=24, stroke_color="#0f172a", font_family=2)
    add_text("header_sub", 80, 95, "Razorpay AI Buildathon 2026 | Track 02: AI Risk Manager — Safety & Governance Layer for Autonomous Financial Agents", font_size=14, stroke_color="#475569", font_family=2)

    # 2. Left Column: Ingress Zone
    add_rectangle("ingress_bg", 60, 170, 300, 840, "#94a3b8", "#f1f5f9", stroke_width=1, stroke_style="dashed", roughness=0, roundness=3)
    add_text("ingress_header", 80, 185, "1. INGRESS & INTERCEPTION", font_size=15, stroke_color="#334155", font_family=2)

    # Ingress Box 1: AI Agent
    add_rectangle("agent_box", 80, 220, 260, 170, "#2563eb", "#eff6ff", stroke_width=2, roughness=1)
    add_text("agent_title", 95, 235, "🤖 Autonomous AI Agents", font_size=16, stroke_color="#1e40af", font_family=2)
    add_text("agent_body", 95, 265, "Initiates Money Movement\n• POST /v1/actions\n• Agent ID & Merchant ID\n• Action Type: Refund/Payout\n• Amount & Declared Intent", font_size=12, stroke_color="#1e3a8a", font_family=2)

    # Ingress Box 2: Razorpay Webhooks
    add_rectangle("webhook_box", 80, 420, 260, 170, "#9333ea", "#fbf2ff", stroke_width=2, roughness=1)
    add_text("webhook_title", 95, 435, "⚡ Razorpay Webhooks", font_size=16, stroke_color="#7e22ce", font_family=2)
    add_text("webhook_body", 95, 465, "Asynchronous Signals\n• POST /v1/webhooks\n• Dispute & Chargeback alerts\n• Merchant loss events\n• HMAC-SHA256 Verification", font_size=12, stroke_color="#581c87", font_family=2)

    # Ingress Box 3: Eval Harness & Synthetic Worlds
    add_rectangle("eval_box", 80, 620, 260, 170, "#ea580c", "#fff7ed", stroke_width=2, roughness=1)
    add_text("eval_title", 95, 635, "🧪 Benchmark Harness", font_size=16, stroke_color="#c2410c", font_family=2)
    add_text("eval_body", 95, 665, "Adversarial Test Worlds\n• 8 Labeled Agent Worlds\n• Coincidental sharing test\n• Merchant collusion & evasion\n• Innocent household FP test", font_size=12, stroke_color="#7c2d12", font_family=2)

    # Ingress Box 4: CLI & Intent Driver
    add_rectangle("cli_box", 80, 820, 260, 170, "#0284c7", "#f0f9ff", stroke_width=2, roughness=1)
    add_text("cli_title", 95, 835, "💻 Interactive Driver CLI", font_size=16, stroke_color="#0369a1", font_family=2)
    add_text("cli_body", 95, 865, "Demo Script Executor\n• cargo run -p governor\n• 4 Scripted Scenarios\n• Live API key passthrough\n• Batch scenario runner", font_size=12, stroke_color="#0c4a6e", font_family=2)


    # 3. Center Column: Core Risk Governor Pipeline
    add_rectangle("governor_bg", 400, 170, 720, 840, "#475569", "#ffffff", stroke_width=2, roughness=0, roundness=3)
    add_text("governor_header", 420, 185, "2. RISK GOVERNOR PIPELINE (Single-Thesis Rust Core)", font_size=16, stroke_color="#0f172a", font_family=2)

    # Stage 1: Policy Engine
    add_rectangle("stage1_box", 430, 220, 660, 130, "#475569", "#f8fafc", stroke_width=2, roughness=1)
    add_text("stage1_title", 450, 235, "⚙️ Stage 1 — Policy Engine (Hard Invariants & Scope)", font_size=16, stroke_color="#1e293b", font_family=2)
    add_text("stage1_body", 450, 265, "• Hard threshold checks (e.g. ₹50k auto-allow ceiling, ₹150k approval threshold)\n• Per-agent velocity limits & hourly window rate counters\n• Country & merchant domain scope allowlists / blocklists\n• Fast-path rejection before heavy scoring pipeline", font_size=12, stroke_color="#334155", font_family=2)

    # Stage 2: Risk Engine
    add_rectangle("stage2_box", 430, 380, 660, 130, "#d97706", "#fef3c7", stroke_width=2, roughness=1)
    add_text("stage2_title", 450, 395, "🧠 Stage 2 — Risk Engine (Behavioral Scoring & Intent Analysis)", font_size=16, stroke_color="#b45309", font_family=2)
    add_text("stage2_body", 450, 425, "• Statistical Z-score anomaly detection against historical agent behavior\n• Intent Mismatch Scorer: NLP cross-check of declared intent vs action & amount\n• Behavioral drift tracking over sliding time windows\n• Output: Scaled composite risk score ∈ [0.0, 1.0]", font_size=12, stroke_color="#78350f", font_family=2)

    # Stage 3: Risk Graph & Correlation
    add_rectangle("stage3_box", 430, 540, 660, 130, "#c026d3", "#fae8ff", stroke_width=2, roughness=1)
    add_text("stage3_title", 450, 555, "🕸️ Stage 3 — Risk Graph & Correlation Engine", font_size=16, stroke_color="#a21caf", font_family=2)
    add_text("stage3_body", 450, 585, "• Dynamic entity graph (Customers, Devices, IP addresses, Cards, Merchants)\n• Multi-hop cluster traversal for coordinated abuse ring detection\n• Coincidental sharing disaggregation (separates shared Wi-Fi from fraud rings)\n• Detects merchant collusion & distributed botnet topologies", font_size=12, stroke_color="#701a75", font_family=2)

    # Stage 4: Investigation Engine
    add_rectangle("stage4_box", 430, 700, 660, 140, "#0284c7", "#e0f2fe", stroke_width=2, roughness=1)
    add_text("stage4_title", 450, 715, "🔍 Stage 4 — Investigation Engine (Evidence Reasoning Layer)", font_size=16, stroke_color="#0369a1", font_family=2)
    add_text("stage4_body", 450, 745, "• Core Thesis: High risk cannot automatically BLOCK without behavioral confirmation\n• Weighs evidence signals: Direction::{Supports, Contradicts}\n• Computes evidence_confidence score; requires certainty before blocking\n• Escalates high-risk + low-confidence cases to REVIEW (Human-in-the-Loop)", font_size=12, stroke_color="#0c4a6e", font_family=2)

    # Stage 5: Audit & Replay Engine
    add_rectangle("stage5_box", 430, 870, 660, 120, "#374151", "#f3f4f6", stroke_width=2, roughness=1)
    add_text("stage5_title", 450, 885, "📜 Stage 5 — Audit Service & Decision Replay", font_size=16, stroke_color="#1f2937", font_family=2)
    add_text("stage5_body", 450, 915, "• Append-only immutable decision ledger with full feature breakdown\n• End-to-end timeline reconstruction: action → policy → risk → graph → decision → call\n• Replay endpoint GET /v1/decisions/{id}/replay for post-mortem auditing", font_size=12, stroke_color="#111827", font_family=2)


    # 4. Right Column: Decision Router & Razorpay Execution
    add_rectangle("router_bg", 1160, 170, 380, 840, "#94a3b8", "#f1f5f9", stroke_width=1, stroke_style="dashed", roughness=0, roundness=3)
    add_text("router_header", 1180, 185, "3. DECISION ROUTER & EXECUTION", font_size=15, stroke_color="#334155", font_family=2)

    # Path 1: ALLOW (Green)
    add_rectangle("allow_box", 1180, 220, 340, 220, "#16a34a", "#dcfce7", stroke_width=2, roughness=1)
    add_text("allow_title", 1200, 235, "🟢 ALLOW (Auto-Approved)", font_size=18, stroke_color="#15803d", font_family=2)
    add_text("allow_body", 1200, 270, "Action passed all safety gates.\nExecuted against Razorpay Test API:\n\n• razorpay-gateway HTTP Client\n• Basic Auth (Key ID & Secret)\n• Automated retry & backoff on 429/5xx\n• Real Order & Refund Creation", font_size=12, stroke_color="#14532d", font_family=2)

    # Path 2: REVIEW (Yellow)
    add_rectangle("review_box", 1180, 480, 340, 250, "#ca8a04", "#fef9c3", stroke_width=2, roughness=1)
    add_text("review_title", 1200, 495, "🟡 REVIEW (Human Approval)", font_size=18, stroke_color="#a16207", font_family=2)
    add_text("review_body", 1200, 530, "High risk score lacking certainty.\nHeld safely for human intervention:\n\n• Surfaces in Live Dashboard (/)\n• Inline One-Click Approve / Reject\n• Approving executes Razorpay API call\n• ZERO False Positive Friction Cost\n  (0 of 324 legitimate users blocked)", font_size=12, stroke_color="#713f12", font_family=2)

    # Path 3: BLOCK (Red)
    add_rectangle("block_box", 1180, 760, 340, 220, "#dc2626", "#fee2e2", stroke_width=2, roughness=1)
    add_text("block_title", 1200, 775, "🔴 BLOCK (Execution Halted)", font_size=18, stroke_color="#b91c1c", font_family=2)
    add_text("block_body", 1200, 810, "Confirmed hard breach or abuse.\nExecution stopped immediately:\n\n• NEVER touches Razorpay APIs\n• Detailed audit event recorded\n• Prevents merchant financial loss\n• Full explanation logged in replay", font_size=12, stroke_color="#7f1d1d", font_family=2)


    # 5. Bottom Storage & Infrastructure Layer
    add_rectangle("store_bg", 60, 1030, 1480, 130, "#475569", "#f8fafc", stroke_width=1, stroke_style="dashed", roughness=0, roundness=3)

    add_rectangle("db_box", 80, 1045, 460, 100, "#0f172a", "#ffffff", stroke_width=2, roughness=1)
    add_text("db_title", 95, 1060, "💾 Persistence Layer (PG / SQLite / Memory)", font_size=15, stroke_color="#0f172a", font_family=2)
    add_text("db_body", 95, 1090, "• pg-store & rusqlite backends • Agent history & feature snapshots\n• Entity graph nodes/edges • Decision audit trail storage", font_size=12, stroke_color="#475569", font_family=2)

    add_rectangle("nats_box", 570, 1045, 460, 100, "#0f172a", "#ffffff", stroke_width=2, roughness=1)
    add_text("nats_title", 585, 1060, "📡 Distributed Event Link (NATS Link)", font_size=15, stroke_color="#0f172a", font_family=2)
    add_text("nats_body", 585, 1090, "• Decoupled microservice transport across pipeline stages\n• Async event streams for high-throughput merchant environments", font_size=12, stroke_color="#475569", font_family=2)

    add_rectangle("dash_box", 1060, 1045, 460, 100, "#0f172a", "#ffffff", stroke_width=2, roughness=1)
    add_text("dash_title", 1075, 1060, "🌐 Axum Server & Zero-Build Dashboard", font_size=15, stroke_color="#0f172a", font_family=2)
    add_text("dash_body", 1075, 1090, "• Unified binary governor-server at http://127.0.0.1:8080\n• Real-time decision stream (polling 2s) + inline approval UI", font_size=12, stroke_color="#475569", font_family=2)


    # 6. Connecting Arrows

    # Ingress to Governor
    add_arrow("arr_agent_gov", 340, 305, 430, 285, stroke_color="#2563eb", stroke_width=2)
    add_arrow("arr_wh_gov", 340, 505, 430, 930, stroke_color="#9333ea", stroke_width=2)
    add_arrow("arr_cli_gov", 340, 905, 430, 285, stroke_color="#0284c7", stroke_width=2)

    # Intra-pipeline Stage Arrows
    add_arrow("arr_s1_s2", 760, 350, 760, 380, stroke_color="#475569", stroke_width=2)
    add_arrow("arr_s2_s3", 760, 510, 760, 540, stroke_color="#d97706", stroke_width=2)
    add_arrow("arr_s3_s4", 760, 670, 760, 700, stroke_color="#c026d3", stroke_width=2)
    add_arrow("arr_s4_s5", 760, 840, 760, 870, stroke_color="#0284c7", stroke_width=2)

    # Pipeline to Decision Router
    add_arrow("arr_dec_allow", 1090, 770, 1180, 330, stroke_color="#16a34a", stroke_width=3)
    add_arrow("arr_dec_review", 1090, 770, 1180, 605, stroke_color="#ca8a04", stroke_width=3)
    add_arrow("arr_dec_block", 1090, 770, 1180, 870, stroke_color="#dc2626", stroke_width=3)

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
    import os
    data = create_excalidraw()

    target_paths = [
        "/Users/apple/risk-governor/docs/risk-governor-architecture.excalidraw",
        "/Users/apple/risk-governor/risk-governor-architecture.excalidraw"
    ]

    for p in target_paths:
        os.makedirs(os.path.dirname(p), exist_ok=True)
        with open(p, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2)
        print(f"Successfully generated Excalidraw diagram at {p}")
