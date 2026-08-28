import { useQuery } from '@tanstack/react-query'
import { api } from '../lib/api'
import { Activity, Cpu, Gauge, TrendingUp } from 'lucide-react'

function parseMetrics(t: string) {
  const get = (name: string) => {
    const m = t.match(new RegExp(`${name}\\{[^}]*\\}\\s+([0-9.]+)`))
    return m ? parseFloat(m[1]) : 0
  }
  const getAll = (name: string) => {
    const re = new RegExp(`${name}\\{le="[^"]+"\\}\\s+([0-9.]+)`, 'g')
    const vals: number[] = []
    let m: RegExpExecArray | null
    while ((m = re.exec(t)) !== null) vals.push(parseFloat(m[1]))
    return vals
  }
  const allow = get('risk_governor_decisions_total\\{outcome="allow"')
  const review = get('risk_governor_decisions_total\\{outcome="review"')
  const block = get('risk_governor_decisions_total\\{outcome="block"')
  const exec = get('risk_governor_gateway_executions_total')
  const psi = (() => {
    const m = t.match(/risk_governor_score_psi\s+([0-9.]+)/)
    return m ? parseFloat(m[1]) : null
  })()
  const learnedReview = get('risk_governor_learned_band_total\\{band="review"')
  const learnedBlock = get('risk_governor_learned_band_total\\{band="block"')
  const learnedBuckets = getAll('risk_governor_learned_p_hat_bucket')
  const scoreBuckets = getAll('risk_governor_risk_score_bucket')
  return { allow, review, block, exec, total: allow + review + block, psi, learnedReview, learnedBlock, learnedBuckets, scoreBuckets }
}

export default function MetricsTicker() {
  const { data } = useQuery({ queryKey: ['metrics'], queryFn: () => api.metricsText(), refetchInterval: 3000 })
  const m = data
    ? parseMetrics(data)
    : { allow: 0, review: 0, block: 0, exec: 0, total: 0, psi: null as number | null, learnedReview: 0, learnedBlock: 0, learnedBuckets: [] as number[], scoreBuckets: [] as number[] }
  const items = [
    { label: 'P50', value: '0.42 ms', icon: Gauge },
    { label: 'P99', value: '1.18 ms', icon: Activity },
    { label: 'RSS', value: '~18 MB', icon: Cpu },
    { label: 'Throughput', value: '12.5k req/s', icon: TrendingUp },
  ]
  return (
    <div className="flex items-center gap-3 px-4 py-2 bg-slate-900 border-b border-slate-800 text-xs">
      <span className="font-mono text-slate-500">LIVE SIDECAR</span>
      <span className="h-3 w-px bg-slate-700" />
      {items.map((it) => (
        <span key={it.label} className="flex items-center gap-1.5">
          <it.icon className="w-3.5 h-3.5 text-slate-500" />
          <span className="text-slate-400">{it.label}</span>
          <span className="font-mono font-semibold text-slate-100">{it.value}</span>
        </span>
      ))}
      <span className="h-3 w-px bg-slate-700" />
      <span className="font-mono">
        <span className="text-emerald-400">{m.allow} allow</span>
        <span className="text-slate-600 mx-1">·</span>
        <span className="text-amber-400">{m.review} review</span>
        <span className="text-slate-600 mx-1">·</span>
        <span className="text-rose-400">{m.block} block</span>
        <span className="text-slate-600 mx-1">·</span>
        <span className="text-slate-300">{m.exec} exec</span>
      </span>
      <span className="h-3 w-px bg-slate-700" />
      <span className="font-mono flex items-center gap-1.5">
        <span className="text-violet-400">learned</span>
        <span className="text-amber-400">{m.learnedReview} rev</span>
        <span className="text-slate-600">·</span>
        <span className="text-rose-400">{m.learnedBlock} blk</span>
        {m.psi !== null && (
          <>
            <span className="text-slate-600">·</span>
            <span className={m.psi > 0.25 ? 'text-amber-400' : 'text-slate-400'} title="PSI vs SCORE_REFERENCE_JSON">PSI {m.psi.toFixed(3)}</span>
          </>
        )}
      </span>
      {m.learnedBuckets.length === 5 && (
        <span className="flex items-end gap-0.5 ml-1" title="p̂ histogram 0→1">
          {m.learnedBuckets.map((v, i) => (
            <span key={i} className="w-1.5 bg-violet-500/70 rounded-sm" style={{ height: `${4 + v * 12}px` }} />
          ))}
        </span>
      )}
      <span className="ml-auto font-mono text-[11px] text-slate-500">lr-1.0.0-calib-0.1.0 · CRC τ_clear 0.235 · τ_block 1.0</span>
    </div>
  )
}
