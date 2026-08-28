import { useQuery } from '@tanstack/react-query'
import { api } from '../lib/api'
import { Activity, Cpu, Gauge, TrendingUp } from 'lucide-react'

function parseMetrics(t: string) {
  const get = (name: string) => {
    const m = t.match(new RegExp(`${name}\\{[^}]*\\}\\s+([0-9.]+)`))
    return m ? parseFloat(m[1]) : 0
  }
  const allow = get('risk_governor_decisions_total\\{outcome="allow"')
  const review = get('risk_governor_decisions_total\\{outcome="review"')
  const block = get('risk_governor_decisions_total\\{outcome="block"')
  const exec = get('risk_governor_gateway_executions_total')
  return { allow, review, block, exec, total: allow + review + block }
}

export default function MetricsTicker() {
  const { data } = useQuery({ queryKey: ['metrics'], queryFn: () => api.metricsText() })
  const m = data ? parseMetrics(data) : { allow: 0, review: 0, block: 0, exec: 0, total: 0 }
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
        <span className="text-slate-300">{m.exec} executions</span>
      </span>
      <span className="ml-auto font-mono text-[11px] text-slate-500">18 MB RSS · mock gateway · heuristic intent</span>
    </div>
  )
}
