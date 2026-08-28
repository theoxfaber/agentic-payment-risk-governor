export default function PaiseMath({ captured, refunded, requested }: { captured: number; refunded: number; requested: number }) {
  const available = captured - refunded
  const ok = requested <= available
  return (
    <div className="rounded-lg border border-slate-800 bg-slate-900 p-3">
      <div className="text-xs font-semibold tracking-widest text-slate-400">INTEGER PAISE MATH</div>
      <div className="mt-2 grid grid-cols-3 gap-2 font-mono text-xs">
        <div className="rounded bg-slate-950 border border-slate-800 p-2 text-center">
          <div className="text-slate-500">captured</div>
          <div className="font-semibold">₹{(captured / 100).toFixed(2)}</div>
          <div className="text-[11px] text-slate-500">{captured} paise</div>
        </div>
        <div className="rounded bg-slate-950 border border-slate-800 p-2 text-center">
          <div className="text-slate-500">− refunded</div>
          <div className="font-semibold">₹{(refunded / 100).toFixed(2)}</div>
          <div className="text-[11px] text-slate-500">{refunded} paise</div>
        </div>
        <div className={`rounded border p-2 text-center ${ok ? 'bg-emerald-500/10 border-emerald-500/20' : 'bg-rose-500/10 border-rose-500/20'}`}>
          <div className="text-slate-500">= available</div>
          <div className="font-semibold">₹{(available / 100).toFixed(2)}</div>
          <div className="text-[11px]">{available} paise</div>
        </div>
      </div>
      <div className="mt-2 flex items-center justify-between font-mono text-xs">
        <span className={ok ? 'text-emerald-400' : 'text-rose-400'}>
          requested {requested} paise {ok ? '≤' : '>'} available {available} paise → {ok ? 'PASS' : 'BLOCK'}
        </span>
        <span className="text-slate-500">checked subtraction (no float)</span>
      </div>
    </div>
  )
}
