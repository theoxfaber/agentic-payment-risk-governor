export default function AuditRail({ trail }: { trail: { event_type: string; created_at: string; previous_hash?: string; current_hash?: string }[] }) {
  const copy = (t: string) => navigator.clipboard.writeText(t)
  return (
    <div className="h-full flex flex-col">
      <div className="px-3 py-2 border-b border-slate-800 text-xs font-semibold tracking-widest text-slate-400">AUDIT LEDGER — SHA-256</div>
      <div className="flex-1 overflow-y-auto p-3 space-y-3">
        {trail.map((e, i) => (
          <div key={i} className="relative pl-4 border-l-2 border-slate-800">
            <div className="absolute -left-1.5 top-1 w-2.5 h-2.5 rounded-full bg-slate-600 border-2 border-slate-950" />
            <div className="font-mono text-xs font-semibold text-slate-200">{e.event_type}</div>
            <div className="font-mono text-[11px] text-slate-500">{new Date(e.created_at).toLocaleString()}</div>
            {e.current_hash && (
              <div className="mt-1 rounded bg-slate-950 border border-slate-800 p-1.5">
                <div className="font-mono text-[10px] text-slate-500">current_hash</div>
                <div className="font-mono text-[11px] break-all text-slate-300">{e.current_hash}</div>
                <button onClick={() => copy(e.current_hash!)} className="mt-1 text-[11px] text-slate-400 hover:text-white">
                  copy
                </button>
              </div>
            )}
            {e.previous_hash && <div className="font-mono text-[10px] text-slate-600 truncate">prev {e.previous_hash.slice(0, 16)}…</div>}
          </div>
        ))}
        <div className="rounded border border-slate-800 bg-slate-900 p-2 font-mono text-[11px] text-slate-400">
          Canonical JSON (sorted keys) → SHA256(previous_hash ‖ record) — verify via <span className="text-slate-200">AuditService::verify_chain()</span>
        </div>
      </div>
    </div>
  )
}
