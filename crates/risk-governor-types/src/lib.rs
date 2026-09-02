use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Refund,
    Payout,
    PaymentLink,
    Transfer,
    Capture,
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    Allow,
    Review,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyVerdict {
    Allow,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentActionRequest {
    pub agent_id: String,
    pub merchant_id: String,
    pub action_type: ActionType,
    pub amount: i64,
    pub currency: String,
    pub declared_intent: String,
    pub context: serde_json::Value,
    pub timestamp: DateTime<Utc>,
    pub correlation_id: Uuid,
}

/// Recursively canonicalizes a serde_json::Value by sorting all object keys,
/// producing deterministic byte representation independent of map order.
pub fn canonical_json_bytes(val: &serde_json::Value) -> Vec<u8> {
    match val {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut buf = Vec::new();
            buf.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    buf.push(b',');
                }
                buf.extend(serde_json::to_vec(k).unwrap_or_default());
                buf.push(b':');
                buf.extend(canonical_json_bytes(&map[*k]));
            }
            buf.push(b'}');
            buf
        }
        serde_json::Value::Array(arr) => {
            let mut buf = Vec::new();
            buf.push(b'[');
            for (i, elem) in arr.iter().enumerate() {
                if i > 0 {
                    buf.push(b',');
                }
                buf.extend(canonical_json_bytes(elem));
            }
            buf.push(b']');
            buf
        }
        other => serde_json::to_vec(other).unwrap_or_default(),
    }
}

impl AgentActionRequest {
    /// SHA-256 hash of the canonical request input state, establishing a
    /// tamper-evident reference. Fields are length-prefixed to prevent
    /// delimiter-collision attacks (e.g. agent_id="A|B" vs "A" + merchant="B").
    pub fn input_hash(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        // Length-prefix helper: write 4-byte BE length then the bytes.
        // This makes field boundaries unambiguous regardless of content.
        let mut lp = |data: &[u8]| {
            hasher.update((data.len() as u32).to_be_bytes());
            hasher.update(data);
        };
        lp(self.agent_id.as_bytes());
        lp(self.merchant_id.as_bytes());
        lp(format!("{:?}", self.action_type).as_bytes());
        lp(&self.amount.to_be_bytes());
        lp(self.currency.to_uppercase().as_bytes());
        lp(self.declared_intent.as_bytes());
        lp(&canonical_json_bytes(&self.context));
        hex::encode(hasher.finalize())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHistory {
    pub agent_id: String,
    pub total_actions_30d: u32,
    pub total_volume_30d: i64,
    pub avg_amount: i64,
    pub max_amount: i64,
    #[serde(default)]
    pub std_amount: i64,
    pub refund_rate: f64,
    pub block_rate: f64,
    pub review_rate: f64,
    pub first_seen: DateTime<Utc>,
    pub last_action: DateTime<Utc>,
    pub action_type_distribution: HashMap<ActionType, u32>,
    pub anomaly_flags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskTier {
    Standard,
    Medium,
    High,
    VeryHigh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerchantPolicy {
    pub merchant_id: String,
    pub max_refund_amount: i64,
    pub max_payout_amount: i64,
    pub max_payment_link_amount: i64,
    pub daily_refund_limit: i64,
    pub daily_payout_limit: i64,
    pub velocity_threshold_per_hour: u32,
    pub allowed_countries: Vec<String>,
    pub blocked_countries: Vec<String>,
    pub require_approval_above: i64,
    pub custom_rules: Vec<CustomRule>,
    #[serde(default = "default_risk_tier")]
    pub risk_tier: RiskTier,
    #[serde(default = "default_pmla_retention")]
    pub pmla_retention_days: u32,
    #[serde(default)]
    pub fri_score: Option<u8>,
}

fn default_risk_tier() -> RiskTier {
    RiskTier::Standard
}
fn default_pmla_retention() -> u32 {
    1825
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomRule {
    pub rule_id: String,
    pub condition: String,
    pub action: PolicyVerdict,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerHistory {
    pub customer_id: String,
    pub total_transactions: u32,
    pub total_volume: i64,
    pub chargeback_count: u32,
    pub refund_count: u32,
    pub avg_ticket_size: i64,
    pub first_transaction: DateTime<Utc>,
    pub last_transaction: DateTime<Utc>,
    pub risk_score: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VelocityStats {
    pub actions_last_hour: u32,
    pub volume_last_hour: i64,
    pub actions_last_24h: u32,
    pub volume_last_24h: i64,
    pub unique_merchants_24h: u32,
    pub unique_customers_24h: u32,
    #[serde(default)]
    pub declines_last_hour: u32,
    #[serde(default)]
    pub rto_signals_24h: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub agent_history: AgentHistory,
    pub merchant_policy: MerchantPolicy,
    pub customer_history: Option<CustomerHistory>,
    pub recent_velocity: VelocityStats,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyResult {
    pub verdict: PolicyVerdict,
    pub matched_rules: Vec<String>,
    pub violated_thresholds: Vec<String>,
    pub evaluated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFeatures {
    pub amount_zscore: f64,
    pub velocity_zscore: f64,
    pub intent_mismatch_score: f64,
    pub behavioral_drift_score: f64,
    pub merchant_risk_score: f64,
    pub agent_risk_score: f64,
    pub customer_risk_score: f64,
    pub time_since_last_action_hours: f64,
    pub amount_vs_avg_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskResult {
    pub risk_score: f64,
    pub intent_mismatch_score: f64,
    pub features: RiskFeatures,
    pub model_version: String,
    pub evaluated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedInsight {
    pub model_version: String,
    pub p_hat: f64,
    pub tau_clear: f64,
    pub tau_block: f64,
    pub band: String,
    pub features: std::collections::HashMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contributions: Option<std::collections::BTreeMap<String, f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Decision {
    pub decision_id: Uuid,
    pub action: AgentActionRequest,
    pub policy_result: PolicyResult,
    pub risk_result: RiskResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learned_insight: Option<LearnedInsight>,
    pub decision: DecisionOutcome,
    pub model_version: String,
    pub evidence_snapshot: Evidence,
    pub created_at: DateTime<Utc>,
    pub human_review: Option<HumanReview>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanReview {
    pub reviewer_id: String,
    pub decision: DecisionOutcome,
    pub notes: Option<String>,
    pub reviewed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRecord {
    pub record_id: Uuid,
    pub decision_id: Option<Uuid>,
    pub event_type: AuditEventType,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub previous_hash: Option<String>,
    #[serde(default)]
    pub current_hash: String,
}

impl AuditRecord {
    /// Computes SHA-256 hash forming a cryptographic tamper-evident audit chain.
    pub fn compute_hash(
        record_id: Uuid,
        decision_id: Option<Uuid>,
        event_type: AuditEventType,
        payload: &serde_json::Value,
        created_at: DateTime<Utc>,
        previous_hash: Option<&str>,
    ) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(previous_hash.unwrap_or("GENESIS").as_bytes());
        hasher.update(b"|");
        hasher.update(record_id.to_string().as_bytes());
        hasher.update(b"|");
        hasher.update(decision_id.map(|d| d.to_string()).unwrap_or_default().as_bytes());
        hasher.update(b"|");
        hasher.update(format!("{:?}", event_type).as_bytes());
        hasher.update(b"|");
        hasher.update(canonical_json_bytes(payload));
        hasher.update(b"|");
        hasher.update(created_at.to_rfc3339().as_bytes());
        hex::encode(hasher.finalize())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    ActionRequested,
    PolicyEvaluated,
    RiskScored,
    GraphAnalyzed,
    DecisionMade,
    HumanReviewed,
    RazorpayCalled,
    WebhookReceived,
    OutcomeRecorded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RazorpayWebhookPayload {
    pub event: String,
    pub payload: serde_json::Value,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaySnapshot {
    pub decision: Decision,
    pub policy_version: String,
    pub risk_model_version: String,
    pub evidence_at_decision: Evidence,
    pub audit_trail: Vec<AuditRecord>,
}

/// Wire format: what action-service sends to the policy-engine worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyEvaluateJob {
    pub request: AgentActionRequest,
    pub evidence: Evidence,
}

/// Reply payload for evidence.gather — distinguishes transport-degraded
/// (handled by the caller as fail-safe) from application-level NotFound
/// (fail closed with a real error).
// Wire enum: Evidence is big but the NotFound path is rare — boxing would
// complicate every producer for no measurable gain.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EvidenceOutcome {
    Ready(Evidence),
    NotFound(String),
}

// ---------------------------------------------------------------------------
// Investigation plane (consumed by the decision combiner)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvestigationVerdict {
    /// Evidence supports the hypothesis → escalation justified.
    Supported,
    /// Real supporting AND real counter-evidence → human decides.
    Conflicted,
    /// Hypothesis not established.
    Unsupported,
}

/// What the combiner needs from the investigation plane, plus enough context
/// to explain itself in the audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigationSummary {
    pub verdict: InvestigationVerdict,
    /// 0..1 — how much of the decision-relevant picture was observable.
    pub evidence_confidence: f64,
    pub support_signals: u32,
    pub contradiction_count: u32,
    /// Strong structural linkage exists independent of behavioral outcome.
    pub structurally_suspicious: bool,
    /// Total weight of contradicting evidence (distinguishes "unconfirmed"
    /// from "exonerated" for structurally-linked clusters).
    pub counter_weight: f64,
    pub estimated_exposure_paise: i64,
}

pub fn generate_correlation_id() -> Uuid {
    Uuid::new_v4()
}

pub fn now_utc() -> DateTime<Utc> {
    Utc::now()
}
