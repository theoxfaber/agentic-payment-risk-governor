import { useQuery, useQueryClient } from '@tanstack/react-query'
import { api, fmtINR } from '../lib/api'
import VotingQuadrant from './VotingQuadrant'
import PaiseMath from './PaiseMath'
import { useState } from 'react'

export default function DetailPanel({ id }: { id: string | null }) {
  const qc = useQueryClient()
  const { data, refetch, isLoading } = useQuery({
    queryKey: ['decision', id],
    queryFn: () => api.get(id!),
    enabled: !!id,
  })
  const [reviewer, setReviewer] = useState('analyst-7')
  const [busy, setBusy] = useState(false)
  const [msg, setMsg] = useState('')

  if (!id) return <div className="h-full flex items-center justify-center text-slate-500 text-sm">Select a decision from the queue</div>
  if (isLoading || !data) return <div className="p-6 text-slate-500">Loading…</div>

  const d = data.decision
  const ctx = d.action.context as Record<string, unknown>
  const captured = Number(ctx.captured_paise ?? 0)
  const refunded = Number(ctx.refunded_paise ?? 0)

  const approve = async (approved: boolean) => {
    if (!reviewer.trim()) { setMsg('Enter reviewer id'); return }
    setBusy(true)
    setMsg('')
    try {
      const res = await api.approve(d.decision_id, reviewer, approved)
      setMsg(approved ? `Approved — ${res.human_review?.decision ?? 'executed'} (gateway: ${res.decision})` : `Rejected — ${res.human_review?.decision}`)
      await Promise.all([refetch(), qc.invalidateQueries({ queryKey: ['decisions'] })])
    } catch (e: unknown) {
      const m = e instanceof Error ? e.message : String(e)
      if (m === 'unauthorized') setMsg('Auth failed — check X-API-Key (demo123) in header')
      else if (m.includes('already reviewed')) setMsg('Already reviewed — refresh queue')
      else setMsg(m)
    } finally { setBusy(false) }
  }

  return (
    <div className="h-full overflow-y-auto p-4 space-y-4">
      <div className="flex items-start justify-between">
        <div>
          <div className="flex items-center gap-2">
            <h2 className="text-sm font-bold">Decision Replay</h2>
            <span
              className={`text-xs font-mono font-bold tracking-widest px-2 py-0.5 rounded-full border ${
                d.decision === 'allow'
                  ? 'text-emerald-400 border-emerald-500/30 bg-emerald-500/10'
                  : d.decision === 'review'
                    ? 'text-amber-400 border-amber-500/30 bg-amber-500/10'
                    : 'text-rose-400 border-rose-500/30 bg-rose-500/10'
              }`}
            >
              {d.decision.toUpperCase()}
            </span>
          </div>
          <div className="font-mono text-xs text-slate-500">{d.decision_id}</div>
          <div className="font-mono text-xs text-slate-500">correlation {d.action.correlation_id}</div>
        </div>
        <div className="text-right font-mono text-xs text-slate-400">
          <div>{new Date(d.created_at).toLocaleString()}</div>
          <div className="text-slate-500">{d.model_version}</div>
        </div>
      </div>

      <div className="rounded-lg border border-slate-800 bg-slate-900 p-3">
        <div className="text-xs font-semibold tracking-widest text-slate-400">AGENT INTENT vs INTEGER SCHEMA</div>
        <div className="mt-2 grid grid-cols-2 gap-3 font-mono text-xs">
          <div>
            <div className="text-slate-500">declared_intent</div>
            <div className="rounded bg-slate-950 border border-slate-800 p-2">“{d.action.declared_intent}”</div>
          </div>
          <div>
            <div className="text-slate-500">sanitised schema</div>
            <div className="rounded bg-slate-950 border border-slate-800 p-2">
              action_type={String(d.action.action_type)} · amount={d.action.amount} · {d.action.currency}
              <br />
              payment_id={String(ctx.payment_id ?? '—')} · state={String(ctx.payment_state ?? '—')}
            </div>
          </div>
        </div>
        <div className="mt-2 font-mono text-[11px] text-slate-500">
          Idempotency-Key: <span className="text-slate-300">rfnd_{String(ctx.payment_id)}_{d.decision_id.slice(0, 8)}…</span> · amount is i64 paise, never float
        </div>
      </div>

      <PaiseMath captured={captured} refunded={refunded} requested={d.action.amount} />

      <VotingQuadrant policy={d.policy_result} risk={d.risk_result} evidence={d.evidence_snapshot} />

      {d.learned_insight && (() => {
        const p = d.learned_insight.p_hat
        const expLoss = p * d.action.amount
        const reviewCost = 40000
        const escalated = d.policy_result.matched_rules.some((r) => String(r).startsWith('learned_escalation'))
        const pos = Math.min(100, Math.max(0, p * 100))
        const tauClearPos = d.learned_insight.tau_clear * 100
        const tauBlockPos = Math.min(100, d.learned_insight.tau_block * 100)
        return (
        <div className="rounded-lg border border-violet-500/20 bg-violet-500/5 p-3">
          <div className="flex items-center justify-between">
            <div className="text-xs font-semibold tracking-widest text-violet-300">LEARNED — p̂ + CRC BAND + ECONOMICS</div>
            <span className="font-mono text-xs text-slate-400">{d.learned_insight.model_version}</span>
          </div>
          {escalated && (
            <div className="mt-2 rounded bg-amber-500/10 border border-amber-500/30 px-2 py-1.5 font-mono text-xs text-amber-300">
              ⚡ Escalated by learned economics: p̂×₹{(d.action.amount/100).toFixed(0)} = ₹{(expLoss/100).toFixed(0)} {expLoss > reviewCost ? '>' : '≤'} ₹400 → {d.learned_insight.band} band
            </div>
          )}
          <div className="mt-2 relative h-2 rounded-full bg-slate-800 overflow-hidden">
            <div className="absolute inset-y-0 w-0.5 bg-emerald-500" style={{ left: `${tauClearPos}%` }} title={`τ_clear ${d.learned_insight.tau_clear.toFixed(3)}`} />
            <div className="absolute inset-y-0 w-0.5 bg-rose-500" style={{ left: `${tauBlockPos}%` }} title={`τ_block ${d.learned_insight.tau_block.toFixed(3)}`} />
            <div className="absolute top-0 h-full bg-violet-500" style={{ width: `${pos}%`, opacity: 0.9 }} />
          </div>
          <div className="mt-1 flex justify-between font-mono text-[10px] text-slate-500">
            <span>0</span><span>τ_clear {d.learned_insight.tau_clear.toFixed(3)}</span><span>τ_block {d.learned_insight.tau_block.toFixed(2)}</span><span>1</span>
          </div>
          <div className="mt-2 grid grid-cols-3 gap-2 font-mono text-xs">
            <div className="rounded bg-slate-950 border border-slate-800 p-2 text-center">
              <div className="text-slate-500">p̂ abuse</div>
              <div className="text-lg font-bold text-violet-300">{p.toFixed(4)}</div>
              <div className="text-[11px] text-slate-500">E[loss] ₹{(expLoss/100).toFixed(0)} vs ₹400</div>
            </div>
            <div className="rounded bg-slate-950 border border-slate-800 p-2 text-center">
              <div className="text-slate-500">band</div>
              <div className={`font-bold text-sm ${d.learned_insight.band === 'clear' ? 'text-emerald-400' : d.learned_insight.band === 'block' ? 'text-rose-400' : 'text-amber-400'}`}>
                {d.learned_insight.band.toUpperCase()}
              </div>
              <div className="text-[11px] text-slate-500">{expLoss <= reviewCost ? 'cheap to be wrong → clear' : p >= d.learned_insight.tau_block ? 'CRC auto-block' : 'human review'}</div>
            </div>
            <div className="rounded bg-slate-950 border border-slate-800 p-2">
              <div className="text-slate-500">8 features → logit</div>
              <div className="mt-1 space-y-1">
                {Object.entries(d.learned_insight.features).sort((a,b)=> Math.abs(b[1] as number)-Math.abs(a[1] as number)).slice(0,4).map(([k,v]) => (
                  <div key={k} className="flex justify-between text-[11px]"><span className="text-slate-500 truncate">{k}</span><span className={(v as number) > 0.5 ? 'text-violet-300' : 'text-slate-400'}>{(v as number).toFixed(2)}</span></div>
                ))}
              </div>
            </div>
          </div>
          <details className="mt-2">
            <summary className="font-mono text-[11px] text-slate-400 cursor-pointer">all 8 features</summary>
            <div className="mt-1 font-mono text-[11px] leading-tight text-slate-500 break-all">
              {Object.entries(d.learned_insight.features).map(([k, v]) => `${k}=${(v as number).toFixed(3)}`).join(' · ')}
            </div>
          </details>
          <div className="mt-2 font-mono text-[11px] text-slate-500">
            α_leak 0.02 · α_friction 0.01 · CRC finite-sample valid · artifact <span className="text-slate-300">eval-harness/artifacts/lr_model.json</span> · deterministic verifier still gates money
          </div>
        </div>
        )})()}

      <div className="rounded-lg border border-slate-800 bg-slate-900 p-3">
        <div className="text-xs font-semibold tracking-widest text-slate-400">EVIDENCE SNAPSHOT</div>
        <div className="mt-2 grid grid-cols-3 gap-2 font-mono text-xs">
          <div className="rounded bg-slate-950 border border-slate-800 p-2">
            <div className="text-slate-500">agent refund_rate</div>
            <div>{(d.evidence_snapshot.agent_history.refund_rate * 100).toFixed(1)}%</div>
          </div>
          <div className="rounded bg-slate-950 border border-slate-800 p-2">
            <div className="text-slate-500">velocity last hour</div>
            <div>{d.evidence_snapshot.recent_velocity.actions_last_hour} actions · ₹{(d.evidence_snapshot.recent_velocity.volume_last_hour / 100).toLocaleString('en-IN')}</div>
          </div>
          <div className="rounded bg-slate-950 border border-slate-800 p-2">
            <div className="text-slate-500">risk model</div>
            <div>{d.risk_result.model_version}</div>
          </div>
        </div>
      </div>

      <div className={`rounded-lg border p-3 flex flex-wrap items-center gap-2 ${d.human_review ? 'border-emerald-500/30 bg-emerald-500/10' : 'border-amber-500/30 bg-amber-500/5'}`}>
        <input value={reviewer} onChange={(e) => setReviewer(e.target.value)} placeholder="reviewer id" className="bg-slate-950 border border-slate-700 rounded px-2 py-1.5 font-mono text-xs w-36" disabled={!!d.human_review} />
        <button
          disabled={busy || d.decision !== 'review' || !!d.human_review}
          onClick={() => approve(true)}
          className="px-3 py-1.5 rounded bg-emerald-500 text-slate-950 font-bold text-xs disabled:opacity-40 hover:brightness-110"
        >
          {busy ? '…' : 'Approve & execute'}
        </button>
        <button
          disabled={busy || d.decision !== 'review' || !!d.human_review}
          onClick={() => approve(false)}
          className="px-3 py-1.5 rounded bg-rose-500 text-white font-bold text-xs disabled:opacity-40 hover:brightness-110"
        >
          Reject
        </button>
        <span className={`font-mono text-xs ${d.human_review ? 'text-emerald-300' : msg.includes('Approved') ? 'text-emerald-300' : msg ? 'text-amber-300' : 'text-slate-400'}`}>
          {d.human_review ? `✓ Reviewed by ${d.human_review.reviewer_id}: ${String(d.human_review.decision).toUpperCase()}${d.human_review.notes ? ` — ${d.human_review.notes}` : ''}` : msg || (d.decision !== 'review' ? 'Only REVIEW can be approved' : 'Human-in-the-loop required — approve to fire gateway')}
        </span>
      </div>

      <div className="font-mono text-xs">
        <div className="text-slate-400">Raw context</div>
        <pre className="mt-1 rounded bg-slate-950 border border-slate-800 p-2 overflow-auto text-[11px]">{JSON.stringify(ctx, null, 2)}</pre>
      </div>
    </div>
  )
}
