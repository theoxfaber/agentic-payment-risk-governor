import { useQuery } from '@tanstack/react-query'
import { useEffect } from 'react'
import { api } from './lib/api'
import { useStore } from './lib/store'
import MetricsTicker from './components/MetricsTicker'
import QueueRail from './components/QueueRail'
import DetailPanel from './components/DetailPanel'
import AuditRail from './components/AuditRail'

export default function App() {
  const { selectedId, setSelected } = useStore()
  const { data, error, refetch } = useQuery({ queryKey: ['decisions'], queryFn: () => api.list() })

  useEffect(() => {
    if (error && (error as Error).message === 'unauthorized') {
      const k = prompt('Risk Governor API key (X-API-Key) — try demo123 for live demo:')
      if (k) { api.setKey(k); refetch() }
    }
  }, [error, refetch])

  const detailQuery = useQuery({
    queryKey: ['decision', selectedId],
    queryFn: () => api.get(selectedId!),
    enabled: !!selectedId,
  })

  const list = data ?? []

  return (
    <div className="h-screen flex flex-col">
      <header className="px-4 py-3 border-b border-slate-800 flex items-center gap-3">
        <h1 className="text-sm font-bold tracking-tight">Risk Governor — Triage Console</h1>
        <span className="text-xs text-slate-500">low-latency proxy · 18MB RSS · deterministic invariants</span>
        <span className="ml-auto flex items-center gap-2">
          <span className="font-mono text-xs text-slate-500">GOVERNOR_API_KEY</span>
          <input
            id="key-input"
            placeholder="demo123"
            defaultValue={api.getKey()}
            onKeyDown={(e) => {
              if (e.key === 'Enter') { api.setKey((e.target as HTMLInputElement).value); refetch() }
            }}
            className="bg-slate-900 border border-slate-700 rounded px-2 py-1 font-mono text-xs w-32"
          />
          <button onClick={() => { const v = (document.getElementById('key-input') as HTMLInputElement).value; api.setKey(v); refetch() }} className="px-2 py-1 rounded bg-slate-800 border border-slate-700 text-xs">Save</button>
          <button onClick={() => { api.clearKey(); location.reload() }} className="px-2 py-1 rounded border border-slate-700 text-xs text-slate-400">Clear</button>
        </span>
      </header>
      <MetricsTicker />
      <div className="flex-1 grid grid-cols-[320px_1fr_340px] min-h-0">
        <div className="border-r border-slate-800 bg-slate-950 overflow-hidden">
          <QueueRail data={list} />
        </div>
        <div className="bg-slate-950 overflow-hidden border-r border-slate-800">
          <DetailPanel id={selectedId} />
        </div>
        <div className="bg-slate-950 overflow-hidden">
          {detailQuery.data ? <AuditRail trail={detailQuery.data.audit_trail} /> : <div className="h-full flex items-center justify-center text-sm text-slate-500 p-6 text-center">Select a decision to see SHA-256 chain</div>}
        </div>
      </div>
      <div className="px-3 py-1.5 border-t border-slate-800 bg-slate-900 flex items-center gap-3 text-[11px] font-mono text-slate-500">
        <span>Click a card → center shows 4 voting planes + paise math · Right shows hash chain · Use Approve for REVIEW</span>
        <span className="ml-auto">{list.length} decisions · {selectedId ? `selected ${selectedId.slice(0, 8)}…` : 'no selection'}</span>
        <button onClick={() => setSelected(null)} className="text-slate-400 hover:text-white">Clear</button>
      </div>
    </div>
  )
}
