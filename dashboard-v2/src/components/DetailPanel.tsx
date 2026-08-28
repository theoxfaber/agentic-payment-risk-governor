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

      {d.learned_insight && (
        <div className="rounded-lg border border-violet-500/20 bg-violet-500/5 p-3">
          <div className="flex items-center justify-between">
            <div className="text-xs font-semibold tracking-widest text-violet-300">LEARNED MODEL — CALIBRATED PROBABILITY + CONFORMAL BAND</div>
            <span className="font-mono text-xs text-slate-400">{d.learned_insight.model_version}</span>
          </div>
          <div className="mt-2 grid grid-cols-3 gap-2 font-mono text-xs">
            <div className="rounded bg-slate-950 border border-slate-800 p-2 text-center">
              <div className="text-slate-500">p̂ (abuse)</div>
              <div className="text-lg font-bold text-violet-300">{d.learned_insight.p_hat.toFixed(4)}</div>
            </div>
            <div className="rounded bg-slate-950 border border-slate-800 p-2 text-center">
              <div className="text-slate-500">band</div>
              <div className={`font-bold ${d.learned_insight.band === 'clear' ? 'text-emerald-400' : d.learned_insight.band === 'block' ? 'text-rose-400' : 'text-amber-400'}`}>
                {d.learned_insight.band.toUpperCase()}
              </div>
              <div className="text-[11px] text-slate-500">
                τ_clear {d.learned_insight.tau_clear.toFixed(3)} · τ_block {d.learned_insight.tau_block.toFixed(3)}
              </div>
            </div>
            <div className="rounded bg-slate-950 border border-slate-800 p-2">
              <div className="text-slate-500">features (8)</div>
              <div className="text-[11px] leading-tight">
                {Object.entries(d.learned_insight.features)
                  .map(([k, v]) => `${k}=${(v as number).toFixed(3)}`)
                  .join(' · ')}
              </div>
            </div>
          </div>
          <div className="mt-2 font-mono text-[11px] text-slate-500">
            Live refund path now emits learned + conformal — deterministic verifier remains the gate, learned is observability + economics (p̂×exposure ≤ ₹400). Artifact `eval-harness/artifacts/lr_model.json`
          </div>
        </div>
      )}

      <div className="rounded-lg border border-slate-800 bg-slate-900 p-3">
        <div className="text-xs font-semibold tracking-widest text-slate-400">EVIDENCE SNAPSHOT</div>
        <div className="mt-2 grid grid-cols-3 gap-2 font-mono text-xs">
          <div className="rounded bg-slate-950 border border-slate-800 p-2">
            <div className="text-slate-500">agent refund_rate</div>
            <div>{(d.evidence_snapshot.agent_history.refund_rate * 100).toFixed(1)}%</div>
          </div>
          <div className="rounded bg-slate-950 border border-slate-800 p-2">
            <div className="text-slate-500">velocity last hour</div>
            <div>{d.evidence_snapshot.recent_velocity.actions_last_hour} actions · ₹{d.evidence_snapshot.recent_velocity.volume_last_hour}</div>
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
