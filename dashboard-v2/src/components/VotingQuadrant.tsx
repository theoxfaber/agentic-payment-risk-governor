import { ShieldCheck, Activity, Share2, Scale } from 'lucide-react'

type Props = {
  policy: { verdict: string; matched_rules: unknown[]; violated_thresholds: string[] }
  risk: { risk_score: number; intent_mismatch_score: number; features: Record<string, number> }
  evidence: { agent_history: { refund_rate: number; anomaly_flags: string[] }; recent_velocity: { actions_last_hour: number } }
}

export default function VotingQuadrant({ policy, risk, evidence }: Props) {
  const policyOk = policy.verdict === 'allow'
  const riskLevel = risk.risk_score
  const riskColor = riskLevel > 0.5 ? 'rose' : riskLevel > 0.25 ? 'amber' : 'emerald'

  return (
    <div className="grid grid-cols-2 gap-3">
      <div className={`rounded-lg border p-3 ${policyOk ? 'bg-emerald-500/5 border-emerald-500/20' : 'bg-rose-500/5 border-rose-500/20'}`}>
        <div className="flex items-center gap-1.5 text-xs font-semibold tracking-widest text-slate-400">
          <ShieldCheck className="w-3.5 h-3.5" /> POLICY
        </div>
        <div className={`mt-1 font-mono text-xs font-bold ${policyOk ? 'text-emerald-400' : 'text-rose-400'}`}>{policy.verdict.toUpperCase()}</div>
        <ul className="mt-2 space-y-1 font-mono text-[11px] text-slate-300">
          {(policy.matched_rules?.length ? policy.matched_rules : ['no rules matched']).map((r, i) => (
            <li key={i} className="truncate">• {String(r)}</li>
          ))}
          {policy.violated_thresholds?.map((t, i) => (
            <li key={`v-${i}`} className="text-rose-400">✕ {t}</li>
          ))}
        </ul>
        <div className="mt-2 font-mono text-[11px] text-slate-500">is_integer_paise==true · is_captured==checked</div>
      </div>

      <div className="rounded-lg border border-slate-800 bg-slate-900 p-3">
        <div className="flex items-center gap-1.5 text-xs font-semibold tracking-widest text-slate-400">
          <Activity className="w-3.5 h-3.5" /> RISK
        </div>
        <div className="mt-2">
          <div className="h-2 rounded-full bg-slate-800 overflow-hidden">
            <div
              className={`h-full ${riskColor === 'emerald' ? 'bg-emerald-500' : riskColor === 'amber' ? 'bg-amber-500' : 'bg-rose-500'}`}
              style={{ width: `${Math.round(riskLevel * 100)}%` }}
            />
          </div>
          <div className="mt-1 flex justify-between font-mono text-[11px]">
            <span className="text-slate-400">risk {risk.risk_score.toFixed(3)}</span>
            <span className="text-slate-400">mismatch {risk.intent_mismatch_score.toFixed(3)}</span>
          </div>
        </div>
        <div className="mt-2 grid grid-cols-2 gap-1 font-mono text-[11px] text-slate-400">
          <span>z-amount {risk.features.amount_zscore?.toFixed(2)}</span>
          <span>z-vel {risk.features.velocity_zscore?.toFixed(2)}</span>
          <span>drift {risk.features.behavioral_drift_score?.toFixed(2)}</span>
          <span>agent {risk.features.agent_risk_score?.toFixed(2)}</span>
        </div>
        {evidence.agent_history.anomaly_flags.length > 0 && (
          <div className="mt-1 font-mono text-[11px] text-amber-400">flags: {evidence.agent_history.anomaly_flags.join(', ')}</div>
        )}
      </div>

      <div className="rounded-lg border border-slate-800 bg-slate-900 p-3">
        <div className="flex items-center gap-1.5 text-xs font-semibold tracking-widest text-slate-400">
          <Share2 className="w-3.5 h-3.5" /> GRAPH
        </div>
        <div className="mt-2 flex items-center gap-2">
          <div className="flex -space-x-1">
            {[1, 2, 3].map((i) => (
              <div key={i} className="w-6 h-6 rounded-full bg-slate-700 border-2 border-slate-900 flex items-center justify-center font-mono text-[10px]">
                {i}
              </div>
            ))}
          </div>
          <span className="font-mono text-xs text-slate-400">cluster size 3 · union-find</span>
        </div>
        <div className="mt-2 font-mono text-[11px] text-slate-500">shared device / address / instrument → transitive merge</div>
      </div>

      <div className="rounded-lg border border-slate-800 bg-slate-900 p-3">
        <div className="flex items-center gap-1.5 text-xs font-semibold tracking-widest text-slate-400">
          <Scale className="w-3.5 h-3.5" /> INVESTIGATION
        </div>
        <div className="mt-2 flex gap-2 font-mono text-xs">
          <span className="flex-1 rounded bg-emerald-500/10 border border-emerald-500/20 px-2 py-1 text-emerald-300 text-center">for 0.42</span>
          <span className="flex-1 rounded bg-rose-500/10 border border-rose-500/20 px-2 py-1 text-rose-300 text-center">against 0.58</span>
        </div>
        <div className="mt-2 font-mono text-[11px] text-slate-400">confidence dampened on partial visibility · hold on conflict</div>
      </div>
    </div>
  )
}
