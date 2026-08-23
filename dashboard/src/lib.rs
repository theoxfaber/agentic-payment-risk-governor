//! Dashboard — Phase 5.
//!
//! ONE page (per the scope cut, not the 5-screen research doc): a live
//! decision stream polling the real `/v1/decisions` API, with click-through
//! to full replay (what the governor saw, why it decided) and human-approval
//! for REVIEW decisions. Served by governor-server; zero build step.

/// The entire UI. Vanilla JS + fetch against governor-server's own API —
/// no framework, no bundler, nothing that can rot before the demo.
pub fn page() -> &'static str {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Risk Governor — Agentic Payment Decisions</title>
<style>
  :root {
    --bg: #0d1117; --panel: #161b22; --border: #30363d;
    --text: #e6edf3; --dim: #8b949e;
    --allow: #3fb950; --review: #d29922; --block: #f85149;
    --mono: ui-monospace, 'SF Mono', Menlo, monospace;
  }
  * { box-sizing: border-box; }
  body {
    margin: 0; background: var(--bg); color: var(--text);
    font: 14px/1.5 -apple-system, 'Segoe UI', sans-serif;
  }
  header {
    padding: 18px 24px; border-bottom: 1px solid var(--border);
    display: flex; align-items: baseline; gap: 14px;
  }
  header h1 { font-size: 17px; margin: 0; font-weight: 650; }
  header .sub { color: var(--dim); font-size: 13px; }
  main { max-width: 1080px; margin: 0 auto; padding: 20px 24px 60px; }

  .stats { display: flex; gap: 10px; margin-bottom: 16px; flex-wrap: wrap; }
  .stat {
    background: var(--panel); border: 1px solid var(--border);
    border-radius: 8px; padding: 10px 16px; min-width: 110px;
  }
  .stat .n { font-size: 22px; font-weight: 700; font-family: var(--mono); }
  .stat .l { color: var(--dim); font-size: 12px; }

  table { width: 100%; border-collapse: collapse; }
  th {
    text-align: left; color: var(--dim); font-size: 11px; font-weight: 600;
    text-transform: uppercase; letter-spacing: 0.05em;
    padding: 8px 10px; border-bottom: 1px solid var(--border);
  }
  td { padding: 9px 10px; border-bottom: 1px solid #21262d; }
  tbody tr { cursor: pointer; }
  tbody tr:hover { background: #1c2129; }
  td.mono { font-family: var(--mono); font-size: 13px; }

  .badge {
    display: inline-block; padding: 2px 10px; border-radius: 999px;
    font-size: 11px; font-weight: 700; text-transform: uppercase;
    letter-spacing: 0.06em; font-family: var(--mono);
  }
  .badge.allow  { color: var(--allow);  border: 1px solid var(--allow); }
  .badge.review { color: var(--review); border: 1px solid var(--review); }
  .badge.block  { color: var(--block);  border: 1px solid var(--block); }

  /* detail overlay */
  #detail {
    position: fixed; inset: 0; background: rgba(1,4,9,0.75);
    display: none; overflow-y: auto;
  }
  #detail.open { display: block; }
  .sheet {
    background: var(--panel); border: 1px solid var(--border);
    border-radius: 10px; max-width: 780px; margin: 40px auto; padding: 24px 28px;
  }
  .sheet h2 { margin: 0 0 4px; font-size: 16px; }
  .sheet .id { color: var(--dim); font-family: var(--mono); font-size: 12px; }
  .kv { display: grid; grid-template-columns: 190px 1fr; gap: 3px 14px; margin: 14px 0; }
  .kv dt { color: var(--dim); }
  .kv dd { margin: 0; font-family: var(--mono); font-size: 13px; word-break: break-all; }

  .rules li { font-family: var(--mono); font-size: 13px; margin-bottom: 2px; }
  section h3 {
    font-size: 12px; text-transform: uppercase; letter-spacing: 0.05em;
    color: var(--dim); margin: 20px 0 8px;
  }
  .trail { list-style: none; margin: 0; padding: 0; }
  .trail li {
    position: relative; padding: 6px 0 6px 24px;
    font-family: var(--mono); font-size: 13px;
    border-left: 2px solid var(--border); margin-left: 8px;
  }
  .trail li::before {
    content: ''; position: absolute; left: -5px; top: 12px;
    width: 8px; height: 8px; border-radius: 50%; background: var(--dim);
  }
  .trail li:last-child::before { background: var(--text); }
  .trail .t { color: var(--dim); font-size: 11px; margin-left: 8px; }

  #approve-box {
    margin-top: 18px; padding: 14px; border: 1px solid var(--review);
    border-radius: 8px; display: none; align-items: center; gap: 10px;
  }
  #approve-box.open { display: flex; }
  button {
    background: var(--allow); color: #041108; border: 0; border-radius: 6px;
    padding: 7px 14px; font-weight: 700; cursor: pointer; font-size: 13px;
  }
  button.reject { background: var(--block); color: #fff; }
  button:hover { filter: brightness(1.1); }
  input {
    background: var(--bg); color: var(--text); border: 1px solid var(--border);
    border-radius: 6px; padding: 7px 10px; font-size: 13px; width: 160px;
  }
  .close-x {
    float: right; cursor: pointer; color: var(--dim); font-size: 20px;
    line-height: 1; background: none; border: none; padding: 0;
  }
  .empty { color: var(--dim); padding: 30px 10px; text-align: center; }
</style>
</head>
<body>
<header>
  <h1>Risk Governor</h1>
  <span class="sub">safety layer for autonomous financial agents — every action judged before it moves money</span>
</header>
<main>
  <div class="stats">
    <div class="stat"><div class="n" id="st-total">–</div><div class="l">decisions</div></div>
    <div class="stat"><div class="n" id="st-allow" style="color:var(--allow)">–</div><div class="l">allowed</div></div>
    <div class="stat"><div class="n" id="st-review" style="color:var(--review)">–</div><div class="l">in review</div></div>
    <div class="stat"><div class="n" id="st-block" style="color:var(--block)">–</div><div class="l">blocked</div></div>
    <div class="stat"><div class="n" id="st-prevented">₹0</div><div class="l">blocked value</div></div>
  </div>

  <table>
    <thead><tr>
      <th>time</th><th>agent</th><th>action</th><th class="amount-h">amount</th>
      <th>risk</th><th>decision</th><th>human</th>
    </tr></thead>
    <tbody id="rows">
      <tr><td colspan="7" class="empty">waiting for actions… submit one via POST /v1/actions</td></tr>
    </tbody>
  </table>
</main>

<div id="detail"><div class="sheet" id="sheet"></div></div>

<script>
'use strict';
const fmtINR = p => '₹' + (p / 100).toLocaleString('en-IN', { maximumFractionDigits: 2 });
const esc = s => String(s).replace(/[&<>"']/g,
  c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
let decisions = [];

async function refresh() {
  try {
    const r = await fetch('/v1/decisions');
    if (!r.ok) return;
    decisions = await r.json();
    render();
  } catch (_) { /* server briefly down — keep last frame */ }
}

function render() {
  const rows = document.getElementById('rows');
  if (!decisions.length) return;
  const counts = { allow: 0, review: 0, block: 0 };
  let blockedValue = 0;
  decisions.forEach(d => counts[d.decision]++);
  decisions.filter(d => d.decision === 'block').forEach(d => blockedValue += d.amount);

  document.getElementById('st-total').textContent = decisions.length;
  document.getElementById('st-allow').textContent = counts.allow;
  document.getElementById('st-review').textContent = counts.review;
  document.getElementById('st-block').textContent = counts.block;
  document.getElementById('st-prevented').textContent = fmtINR(blockedValue);

  rows.innerHTML = decisions.map(d => `
    <tr onclick="openDetail('${d.decision_id}')">
      <td class="mono">${new Date(d.created_at).toLocaleTimeString()}</td>
      <td class="mono">${esc(d.agent_id)}</td>
      <td>${esc(String(d.action_type))}</td>
      <td class="mono">${fmtINR(d.amount)}</td>
      <td class="mono">${Number(d.risk_score).toFixed(3)}</td>
      <td><span class="badge ${d.decision}">${d.decision}</span></td>
      <td>${d.human_decision ? `<span class="badge ${d.human_decision === 'allow' ? 'allow' : 'block'}">${esc(d.human_decision)}</span>` :
          (d.decision === 'review' ? '<span style="color:var(--dim)">pending</span>' : '—')}</td>
    </tr>`).join('');
}

async function openDetail(id) {
  const r = await fetch('/v1/decisions/' + id);
  const data = await r.json();
  const d = data.decision;
  const ctx = JSON.stringify(d.action.context, null, 1);

  document.getElementById('sheet').innerHTML = `
    <button class="close-x" onclick="closeDetail()">×</button>
    <h2>Decision Replay <span class="badge ${d.decision}" style="margin-left:8px">${d.decision}</span></h2>
    <div class="id">${d.decision_id}</div>

    <section><h3>what the governor saw</h3>
    <dl class="kv">
      <dt>agent</dt><dd>${esc(d.action.agent_id)}</dd>
      <dt>merchant</dt><dd>${esc(d.action.merchant_id)}</dd>
      <dt>action · amount</dt><dd>${esc(String(d.action.action_type))} · ${fmtINR(d.action.amount)} ${esc(d.action.currency)}</dd>
      <dt>declared intent</dt><dd>"${esc(d.action.declared_intent)}"</dd>
      <dt>context</dt><dd>${esc(ctx)}</dd>
    </dl></section>

    <section><h3>why it decided</h3>
    <dl class="kv">
      <dt>risk score</dt><dd>${Number(d.risk_result.risk_score).toFixed(3)} (intent mismatch ${Number(d.risk_result.intent_mismatch_score).toFixed(3)})</dd>
      <dt>policy verdict</dt><dd>${esc(String(d.policy_result.verdict))}</dd>
      <dt>matched rules</dt><dd><ul class="rules" style="margin:0;padding-left:16px">${(
        d.policy_result.matched_rules || ['<i style="color:var(--dim)">none</i>']
      ).map(r => `<li>${esc(typeof r === 'string' ? r : JSON.stringify(r))}</li>`).join('')}</ul></dd>
      <dt>evidence snapshot</dt><dd>velocity ${d.evidence_snapshot.recent_velocity.actions_last_hour}/hr ·
        agent refund rate ${(100 * Number(d.evidence_snapshot.agent_history.refund_rate)).toFixed(1)}% ·
        flags ${esc(JSON.stringify(d.evidence_snapshot.agent_history.anomaly_flags))}</dd>
      <dt>model</dt><dd>${esc(d.model_version)}</dd>
    </dl></section>

    <section><h3>audit trail</h3>
    <ul class="trail">${data.audit_trail.map(t =>
      `<li>${esc(String(t.event_type))}<span class="t">${new Date(t.created_at).toLocaleTimeString()}</span></li>`
    ).join('')}</ul></section>

    <div id="approve-box">
      <strong>Human review:</strong>
      <input id="reviewer" placeholder="your reviewer id" value="analyst-7">
      <button onclick="resolve(true)">Approve &amp; execute</button>
      <button class="reject" onclick="resolve(false)">Reject</button>
      <span id="approve-msg" style="color:var(--dim)"></span>
    </div>`;
  document.getElementById('detail').classList.add('open');

  if (d.decision === 'review' && !d.human_review) {
    document.getElementById('approve-box').classList.add('open');
  } else if (d.human_review) {
    document.getElementById('approve-box').classList.add('open');
    document.getElementById('approve-box').style.borderColor = 'var(--border)';
    document.querySelector('#approve-box strong').textContent =
      `Reviewed by ${d.human_review.reviewer_id}:`;
    document.querySelectorAll('#approve-box button').forEach(b => b.remove());
    document.getElementById('reviewer').remove();
    document.getElementById('approve-msg').textContent =
      d.human_review.decision.toUpperCase() +
      (d.human_review.notes ? ` — "${d.human_review.notes}"` : '');
  }
  window.currentId = id;
}

async function resolve(approved) {
  const reviewer = document.getElementById('reviewer').value.trim();
  if (!reviewer) { document.getElementById('approve-msg').textContent = 'enter your reviewer id'; return; }
  const r = await fetch(`/v1/decisions/${window.currentId}/approve`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ approved, reviewer_id: reviewer, notes: approved ? 'approved via dashboard' : 'rejected via dashboard' }),
  });
  const out = await r.json();
  if (!r.ok) { document.getElementById('approve-msg').textContent = out.error || 'failed'; return; }
  closeDetail(); refresh();
}

function closeDetail() {
  document.getElementById('detail').classList.remove('open');
}
document.getElementById('detail').addEventListener('click',
  e => { if (e.target.id === 'detail') closeDetail(); });

refresh();
setInterval(refresh, 2000);
</script>
</body>
</html>"#
}
