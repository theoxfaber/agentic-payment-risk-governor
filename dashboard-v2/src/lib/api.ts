export type DecisionSummary = {
  decision_id: string
  agent_id: string
  action_type: string
  amount: number
  created_at: string
  decision: 'allow' | 'review' | 'block'
  human_decision: string | null
  risk_score: number
  learned_p_hat?: number | null
  learned_band?: string | null
  learned_version?: string | null
}

export type DecisionDetail = {
  decision: {
    decision_id: string
    action: {
      agent_id: string
      merchant_id: string
      action_type: string
      amount: number
      currency: string
      declared_intent: string
      context: Record<string, unknown>
      correlation_id: string
    }
    policy_result: { verdict: string; matched_rules: unknown[]; violated_thresholds: string[] }
    risk_result: {
      risk_score: number
      intent_mismatch_score: number
      features: Record<string, number>
      model_version: string
    }
    evidence_snapshot: {
      agent_history: { refund_rate: number; block_rate: number; review_rate: number; anomaly_flags: string[] }
      recent_velocity: { actions_last_hour: number; volume_last_hour: number }
      merchant_policy?: Record<string, unknown>
    }
    learned_insight?: {
      model_version: string
      p_hat: number
      tau_clear: number
      tau_block: number
      band: string
      features: Record<string, number>
      contributions?: Record<string, number> | null
    } | null
    model_version: string
    created_at: string
    human_review: { reviewer_id: string; decision: string; notes?: string } | null
    decision: string
  }
  audit_trail: { event_type: string; created_at: string; previous_hash?: string; current_hash?: string }[]
}

const getKey = () => sessionStorage.getItem('rgov_key') || ''
const base = (import.meta as unknown as { env: Record<string, string> }).env?.VITE_GOVERNOR_URL?.replace(/\/$/, '') || ''

const url = (p: string) => `${base}${p}`

export const api = {
  getKey,
  setKey(k: string) {
    sessionStorage.setItem('rgov_key', k)
  },
  clearKey() {
    sessionStorage.removeItem('rgov_key')
  },
  headers(): Record<string, string> {
    const k = getKey()
    return k ? { 'x-api-key': k } : {}
  },
  async list(): Promise<DecisionSummary[]> {
    const r = await fetch(url('/v1/decisions'), { headers: this.headers() })
    if (r.status === 401) throw new Error('unauthorized')
    if (!r.ok) throw new Error(await r.text())
    return r.json()
  },
  async get(id: string): Promise<DecisionDetail> {
    const r = await fetch(url(`/v1/decisions/${id}`), { headers: this.headers() })
    if (r.status === 401) throw new Error('unauthorized')
    if (!r.ok) throw new Error(await r.text())
    return r.json()
  },
  async approve(id: string, reviewer_id: string, approved: boolean) {
    const r = await fetch(url(`/v1/decisions/${id}/approve`), {
      method: 'POST',
      headers: { 'content-type': 'application/json', ...this.headers() },
      body: JSON.stringify({ reviewer_id, approved, notes: approved ? 'approved via console' : 'rejected via console' }),
    })
    if (r.status === 401) throw new Error('unauthorized')
    const j = await r.json()
    if (!r.ok) throw new Error(j.error || 'approve failed')
    return j
  },
  async metricsText(): Promise<string> {
    const r = await fetch(url('/metrics'))
    return r.text()
  },
}

export const fmtINR = (p: number) => '₹' + (p / 100).toLocaleString('en-IN', { minimumFractionDigits: 0, maximumFractionDigits: 2 })
export const fmtTime = (iso: string) => new Date(iso).toLocaleTimeString()
