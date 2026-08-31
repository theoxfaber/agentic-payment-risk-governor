import { useStore } from '../lib/store'
import { DecisionSummary, fmtINR, fmtTime } from '../lib/api'

export default function QueueRail({ data }: { data: DecisionSummary[] }) {
  const { filter, setFilter, selectedId, setSelected } = useStore()
  const filtered =
    filter === 'all' ? data : data.filter((d) => (filter === 'blocked' ? d.decision === 'block' : d.decision === filter))
  const counts = {
    all: data.length,
    review: data.filter((d) => d.decision === 'review').length,
    blocked: data.filter((d) => d.decision === 'block').length,
    allow: data.filter((d) => d.decision === 'allow').length,
  }

  return (
    <div className="flex flex-col h-full">
      <div className="flex gap-1 p-2">
        {(['all', 'review', 'blocked', 'allow'] as const).map((f) => (
          <button
            key={f}
            onClick={() => setFilter(f)}
            className={`px-2.5 py-1.5 rounded-md text-xs font-semibold capitalize border ${
              filter === f ? 'bg-slate-800 border-slate-600 text-white' : 'border-transparent text-slate-400 hover:text-slate-200'
            }`}
          >
            {f} <span className="ml-1 font-mono text-[11px] opacity-70">{counts[f as keyof typeof counts] ?? counts.all}</span>
          </button>
        ))}
      </div>
      <div className="flex-1 overflow-y-auto px-2 pb-2 space-y-1.5">
        {filtered.length === 0 ? (
          <div className="text-center py-10 text-sm text-slate-500">No decisions in {filter}</div>
        ) : (
          filtered
            .slice()
            .sort((a, b) => +new Date(b.created_at) - +new Date(a.created_at))
            .map((d) => (
              <button
                key={d.decision_id}
                onClick={() => setSelected(d.decision_id)}
                className={`w-full text-left rounded-lg border p-3 transition ${
                  selectedId === d.decision_id ? 'bg-slate-800 border-slate-600' : 'bg-slate-900 border-slate-800 hover:border-slate-700'
                }`}
              >
                <div className="flex items-center justify-between">
                  <span className="font-mono text-xs text-slate-300">{d.agent_id}</span>
                  <span
                    className={`text-[10px] font-bold tracking-widest px-2 py-0.5 rounded-full border font-mono ${
                      d.decision === 'allow'
                        ? 'text-emerald-400 border-emerald-500/40 bg-emerald-500/10'
                        : d.decision === 'review'
                          ? 'text-amber-400 border-amber-500/40 bg-amber-500/10'
                          : 'text-rose-400 border-rose-500/40 bg-rose-500/10'
                    }`}
                  >
                    {d.decision.toUpperCase()}
                  </span>
                </div>
                <div className="mt-1 flex items-center justify-between">
                  <span className="text-xs text-slate-400">{d.action_type}</span>
                  <span className="font-mono text-sm font-semibold">{fmtINR(d.amount)}</span>
                </div>
                <div className="mt-1 flex items-center justify-between text-[11px] font-mono">
                  <span className="text-slate-500">{fmtTime(d.created_at)}</span>
                  <span className="text-slate-400">risk {d.risk_score.toFixed(3)}</span>
                </div>
                {d.learned_p_hat != null && (
                  <div className="mt-1 flex items-center gap-1.5">
                    <span className={`text-[10px] font-mono font-bold px-1.5 py-0.5 rounded border ${d.learned_band === 'clear' ? 'text-emerald-400 border-emerald-500/30 bg-emerald-500/10' : d.learned_band === 'block' ? 'text-rose-400 border-rose-500/30 bg-rose-500/10' : 'text-amber-400 border-amber-500/30 bg-amber-500/10'}`}>
                      p̂ {Number(d.learned_p_hat).toFixed(3)} · {String(d.learned_band)}
                    </span>
                    <span className="font-mono text-[10px] text-slate-500 truncate">{d.learned_version?.split('-')[0]}</span>
                  </div>
                )}
                <div className="mt-1 font-mono text-[10px] text-slate-600 truncate">{d.decision_id}</div>
              </button>
            ))
        )}
      </div>
    </div>
  )
}
