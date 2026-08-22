//! Risk Investigation Engine — constructs, tests and challenges a risk
//! hypothesis instead of scoring a transaction in isolation.
//!
//! Input:  an abuse-ring candidate (cluster) from the evidence graph,
//!         plus per-customer behavioral records.
//! Output: InvestigationResult — supporting evidence, COUNTER-evidence
//!         (the household defense), missing evidence, confidence, verdict.
//!
//! Core safety principle this module exists to enforce:
//!   a high risk score with LOW evidence confidence must never auto-act.

use risk_governor_types::{AgentActionRequest, Evidence, InvestigationSummary};
use risk_graph::{Cluster, EntityId, EntityKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Behavioral record: what the graph alone cannot say
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerBehavior {
    pub customer_id: String,
    pub order_count: u32,
    pub return_count: u32,
    pub refund_count: u32,
    pub dispute_count: u32,
    pub distinct_merchants: u32,
    pub distinct_products: u32,
    pub account_age_days: u64,
    /// Hours between purchase and return for each returned order.
    #[serde(default)]
    pub purchase_to_return_hours: Vec<f64>,
}

impl CustomerBehavior {
    pub fn return_rate(&self) -> f64 {
        if self.order_count == 0 { 0.0 } else { self.return_count as f64 / self.order_count as f64 }
    }
}

/// Population baseline the investigator compares cluster behavior against.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Baseline {
    /// e.g. 0.06 = average customer returns ~6% of orders.
    pub avg_return_rate: f64,
    /// Multiplier above which a cluster is considered anomalous.
    pub return_rate_anomaly_multiplier: f64,
}

impl Default for Baseline {
    fn default() -> Self {
        // 2.5x, not 3x: evasion rings deliberately keep per-account rates
        // under classic thresholds; their AGGREGATE is what betrays them.
        Self { avg_return_rate: 0.06, return_rate_anomaly_multiplier: 2.5 }
    }
}

// ---------------------------------------------------------------------------
// Evidence model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Supports,
    Contradicts,
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub direction: Direction,
    pub signal: String,
    pub description: String,
    /// 0..1 — how strongly this item supports/undermines on its own.
    pub weight: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HypothesisKind {
    CoordinatedReturnAbuse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Evidence supports the hypothesis → escalation justified.
    Supported,
    /// Real supporting AND real counter evidence → human must decide.
    Conflicted,
    /// Hypothesis not established → no escalation from this signal.
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvestigationResult {
    pub hypothesis: HypothesisKind,
    pub cluster_members: Vec<String>,
    pub supporting: Vec<EvidenceItem>,
    pub counter: Vec<EvidenceItem>,
    pub missing: Vec<EvidenceItem>,
    /// 0..1 — how much of the decision-relevant picture we could actually see.
    pub evidence_confidence: f64,
    pub verdict: Verdict,
    /// Strong structural linkage exists (cluster ≥3 members OR ≥2 distinct
    /// shared resource types), independent of behavioral outcome.
    pub structurally_suspicious: bool,
    /// Total weight of contradicting evidence. Consumers need this to
    /// distinguish "unconfirmed" from "exonerated".
    pub counter_weight: f64,
    /// Sum of weights of exposed value in the cluster (paise).
    pub estimated_exposure_paise: i64,
}

impl InvestigationResult {
    /// Operational escalation policy — should money be held?
    ///
    /// Under ADVERSARIAL EVASION, a structurally-linked cluster whose
    /// behavioral investigation comes back empty is NOT exonerated:
    /// absence of behavioral confirmation, when nothing contradicts the
    /// linkage itself, is itself suspicious. But when real counter-evidence
    /// outweighs the hypothesis (established diverse accounts = household),
    /// clearing is correct. Hence the asymmetry:
    ///   Supported / Conflicted        → hold (block or human review)
    ///   Unsupported + strong linkage
    ///     + weak counter-evidence     → hold for human review
    ///   Unsupported otherwise         → clear
    pub fn should_hold_funds(&self) -> bool {
        match self.verdict {
            Verdict::Supported | Verdict::Conflicted => true,
            Verdict::Unsupported => {
                self.structurally_suspicious && self.counter_weight < 0.25
            }
        }
    }

    /// True when funds are held but a HUMAN, not automation, decided.
    pub fn requires_human(&self) -> bool {
        match self.verdict {
            Verdict::Conflicted => true,
            Verdict::Supported => self.evidence_confidence < 0.5,
            Verdict::Unsupported => self.should_hold_funds(),
        }
    }
}

// ---------------------------------------------------------------------------
// Investigator
// ---------------------------------------------------------------------------

pub struct Investigator {
    baseline: Baseline,
}

impl Investigator {
    pub fn new(baseline: Baseline) -> Self {
        Self { baseline }
    }

    /// Investigate one cluster under the coordinated-return-abuse hypothesis.
    ///
    /// `behaviors` keyed by EXTERNAL customer id (the part after `cus:`),
    /// `exposure` keyed the same way.
    pub fn investigate_return_abuse(
        &self,
        _graph: &risk_graph::PropertyGraph,
        cluster: &Cluster,
        behaviors: &HashMap<String, CustomerBehavior>,
        exposure_by_customer: &HashMap<String, i64>,
    ) -> InvestigationResult {
        let mut supporting = Vec::new();
        let mut counter = Vec::new();
        let mut missing = Vec::new();

        // --- structural signals (from the graph) ---

        if cluster.members.len() >= 3 {
            supporting.push(EvidenceItem {
                direction: Direction::Supports,
                signal: "cluster_size".into(),
                description: format!("{} accounts linked through shared resources", cluster.members.len()),
                weight: 0.2,
            });
        }

        let resource_kinds = cluster.link_kinds.len();
        if resource_kinds >= 2 {
            supporting.push(EvidenceItem {
                direction: Direction::Supports,
                signal: "multiple_shared_resource_types".into(),
                description: format!(
                    "accounts share {} independent resource types (e.g. device + address + instrument)",
                    resource_kinds
                ),
                weight: 0.25,
            });
        }

        // A payment instrument shared across unrelated accounts is itself a
        // strong signal — cards/wallets aren't shared like family laptops.
        if cluster.link_kinds.contains(&risk_graph::RelationKind::UsesInstrument)
            && cluster.members.len() >= 3
        {
            supporting.push(EvidenceItem {
                direction: Direction::Supports,
                signal: "shared_payment_instrument".into(),
                description: "multiple accounts transact with the same payment instrument".into(),
                weight: 0.25,
            });
        }

        // --- behavioral signals (graph × behavior join) ---

        let member_behaviors: Vec<&CustomerBehavior> = cluster
            .members
            .iter()
            .filter_map(|m| external_of(m))
            .filter_map(|ext| behaviors.get(ext))
            .collect();

        let observed_all = member_behaviors.len() == cluster.members.len();
        if !observed_all {
            missing.push(EvidenceItem {
                direction: Direction::Missing,
                signal: "partial_behavior_data".into(),
                description: format!(
                    "{} of {} members have behavioral history",
                    member_behaviors.len(),
                    cluster.members.len()
                ),
                weight: 0.15,
            });
        }

        // Fresh coordinated accounts: rings mint new accounts in bursts.
        // Individually unremarkable; collectively telling.
        let all_young = !member_behaviors.is_empty()
            && member_behaviors.iter().all(|b| b.account_age_days < 180);
        if all_young && cluster.members.len() >= 3 {
            supporting.push(EvidenceItem {
                direction: Direction::Supports,
                signal: "young_cluster".into(),
                description: format!(
                    "all {} accounts are under 180 days old despite shared resources",
                    member_behaviors.len()
                ),
                weight: 0.15,
            });
        }

        if !member_behaviors.is_empty() {
            let cluster_rate = weighted_return_rate(&member_behaviors);
            let threshold = self.baseline.avg_return_rate * self.baseline.return_rate_anomaly_multiplier;
            if cluster_rate >= threshold {
                supporting.push(EvidenceItem {
                    direction: Direction::Supports,
                    signal: "return_rate_anomaly".into(),
                    description: format!(
                        "cluster return rate {:.1}x population baseline ({:.0}% vs {:.0}%)",
                        cluster_rate / self.baseline.avg_return_rate.max(f64::EPSILON),
                        cluster_rate * 100.0,
                        self.baseline.avg_return_rate * 100.0
                    ),
                    weight: 0.3,
                });
            } else {
                counter.push(EvidenceItem {
                    direction: Direction::Contradicts,
                    signal: "normal_return_rates".into(),
                    description: format!(
                        "cluster return rate {:.0}% within normal bounds",
                        cluster_rate * 100.0
                    ),
                    weight: 0.2,
                });
            }

            // Refunds without returns: money flowing back with no merchandise
            // coming back — classic refund-abuse shape, invisible to
            // return-rate rules. Deliberately narrow: a customer whose
            // refunds merely MATCH their legitimate returns is normal.
            let refund_heavy = member_behaviors.iter().any(|b| {
                b.return_count == 0 && b.refund_count >= 2 && b.order_count > 0
            });
            let avg_refund_share = member_behaviors
                .iter()
                .map(|b| if b.order_count == 0 { 0.0 } else { b.refund_count as f64 / b.order_count as f64 })
                .sum::<f64>()
                / member_behaviors.len() as f64;
            if refund_heavy {
                supporting.push(EvidenceItem {
                    direction: Direction::Supports,
                    signal: "refunds_without_returns".into(),
                    description: format!(
                        "cluster takes refunds with no corresponding returns (avg refund share {:.0}% vs {:.0}% threshold)",
                        avg_refund_share * 100.0,
                        threshold * 100.0
                    ),
                    weight: 0.3,
                });
            }

            // Synchronized returns: ≥ half the members returned shortly after
            // purchase, and their gaps are similar (coordinated playbook feel).
            let gaps: Vec<f64> = member_behaviors
                .iter()
                .filter(|b| !b.purchase_to_return_hours.is_empty())
                .map(|b| median(&b.purchase_to_return_hours))
                .collect();
            if gaps.len() >= 2 {
                let spread = max_spread(&gaps);
                let avg_gap = gaps.iter().sum::<f64>() / gaps.len() as f64;
                let fast = avg_gap < 72.0;
                if spread < 48.0 && fast {
                    supporting.push(EvidenceItem {
                        direction: Direction::Supports,
                        signal: "synchronized_returns".into(),
                        description: format!(
                            "{} members return purchases within similar short windows (~{:.0}h median)",
                            gaps.len(),
                            median(&gaps)
                        ),
                        weight: 0.25,
                    });
                }
            }

            // Counter: household defense — diverse merchants/products and
            // long-established accounts look like a family, not a ring.
            let diverse = member_behaviors.iter().all(|b| b.distinct_merchants >= 2 && b.distinct_products >= 2);
            let established = member_behaviors.iter().any(|b| b.account_age_days > 365);
            if diverse && established {
                counter.push(EvidenceItem {
                    direction: Direction::Contradicts,
                    signal: "household_plausible".into(),
                    description: "diverse purchasing across members and long account history — consistent with a household sharing devices/address".into(),
                    weight: 0.3,
                });
            }

            // Counter: disputes absent across members — but only meaningful
            // with substantial volume; small quiet clusters prove nothing.
            if member_behaviors.iter().all(|b| b.dispute_count == 0)
                && member_behaviors.iter().map(|b| b.order_count).sum::<u32>() > 50
            {
                counter.push(EvidenceItem {
                    direction: Direction::Contradicts,
                    signal: "no_disputes".into(),
                    description: "no chargebacks anywhere in the cluster despite volume".into(),
                    weight: 0.1,
                });
            }
        }

        // --- exposure ---
        let exposure: i64 = cluster
            .members
            .iter()
            .filter_map(|m| external_of(m).and_then(|e| exposure_by_customer.get(e)))
            .sum();

        // --- confidence: how much of the picture was observable ---
        let mut confidence = 0.3f64;
        if observed_all { confidence += 0.25; }
        confidence += (supporting.len().min(5) as f64) * 0.05;
        confidence += (counter.len().min(3) as f64) * 0.03;
        // Incompleteness DAMPENS rather than merely fails-to-reward: strong
        // signals seen through a keyhole are still seen through a keyhole.
        if !observed_all {
            confidence *= 0.75;
        }
        confidence = confidence.clamp(0.0, 1.0);

        // --- verdict ---
        let support_weight: f64 = supporting.iter().map(|e| e.weight).sum();
        let counter_weight: f64 = counter.iter().map(|e| e.weight).sum();

        // Verdict asymmetry (deliberate):
        //   Supported needs strong support and near-silence from counter-
        //     evidence — either the classic 0.6-weight case, or several
        //     agreeing signals (≥0.45) with NOTHING contradicting.
        //   Conflicted requires the evidence to LEAN toward the hypothesis
        //     (support > counter) — if counter-evidence outweighs support,
        //     the hypothesis simply fails (Unsupported). This is what keeps
        //     shared-device households from escalating.
        let verdict = match () {
            _ if counter_weight == 0.0 && support_weight >= 0.45 => Verdict::Supported,
            _ if support_weight >= 0.6 && counter_weight < 0.25 => Verdict::Supported,
            _ if support_weight >= 0.4
                && counter_weight >= 0.25
                && support_weight > counter_weight =>
            {
                Verdict::Conflicted
            }
            _ => Verdict::Unsupported,
        };

        InvestigationResult {
            hypothesis: HypothesisKind::CoordinatedReturnAbuse,
            cluster_members: cluster.members.iter().map(|m| m.0.clone()).collect(),
            supporting,
            counter,
            missing,
            evidence_confidence: confidence,
            verdict,
            structurally_suspicious: cluster.members.len() >= 3
                || cluster.link_kinds.len() >= 2,
            counter_weight,
            estimated_exposure_paise: exposure,
        }
    }
}

fn external_of(id: &EntityId) -> Option<&str> {
    id.0.split_once(':').map(|(_, ext)| ext)
}

fn weighted_return_rate(bs: &[&CustomerBehavior]) -> f64 {
    let orders: u32 = bs.iter().map(|b| b.order_count).sum();
    let returns: u32 = bs.iter().map(|b| b.return_count).sum();
    if orders == 0 { 0.0 } else { returns as f64 / orders as f64 }
}

fn median(v: &[f64]) -> f64 {
    let mut s: Vec<f64> = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = s.len();
    if n == 0 { 0.0 } else if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 }
}

fn max_spread(v: &[f64]) -> f64 {
    let mn = v.iter().cloned().fold(f64::INFINITY, f64::min);
    let mx = v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    mx - mn
}

// ---------------------------------------------------------------------------
// Bridge to action-service: per-request investigation over the graph
// ---------------------------------------------------------------------------

/// Implements `action_service::Investigator`. Given a request carrying
/// `context.customer_id`, finds that customer's cluster and runs the
/// coordinated-return-abuse hypothesis.
pub struct GraphInvestigator {
    pub graph: Arc<risk_graph::PropertyGraph>,
    pub behaviors: HashMap<String, CustomerBehavior>,
    pub exposure: HashMap<String, i64>,
    pub baseline: Baseline,
}

impl GraphInvestigator {
    pub fn new(
        graph: Arc<risk_graph::PropertyGraph>,
        behaviors: HashMap<String, CustomerBehavior>,
        exposure: HashMap<String, i64>,
        baseline: Baseline,
    ) -> Self {
        Self { graph, behaviors, exposure, baseline }
    }

    pub fn into_trait(self) -> Arc<dyn action_service::Investigator> {
        Arc::new(self)
    }
}

#[async_trait::async_trait]
impl action_service::Investigator for GraphInvestigator {
    async fn investigate(
        &self,
        request: &AgentActionRequest,
        _evidence: &Evidence,
    ) -> Result<(InvestigationSummary, serde_json::Value), action_service::ActionServiceError> {
        let customer_id = request
            .context
            .get("customer_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                action_service::ActionServiceError::Validation(
                    "investigation requires context.customer_id".into(),
                )
            })?;

        // Find clusters containing this customer (min size 2 — solo customers
        // have no ring hypothesis to test).
        let cid = EntityId::new(EntityKind::Customer, customer_id);
        let cluster = self
            .graph
            .abuse_ring_clusters(2)
            .into_iter()
            .find(|c| c.members.contains(&cid));

        let result = match cluster {
            Some(c) => {
                let inv = Investigator::new(self.baseline.clone());
                inv.investigate_return_abuse(&self.graph, &c, &self.behaviors, &self.exposure)
            }
            None => InvestigationResult {
                hypothesis: HypothesisKind::CoordinatedReturnAbuse,
                cluster_members: vec![cid.0],
                supporting: vec![],
                counter: vec![],
                missing: vec![EvidenceItem {
                    direction: Direction::Missing,
                    signal: "no_cluster".into(),
                    description: "customer shares no resources with other accounts — no ring hypothesis".into(),
                    weight: 0.1,
                }],
                evidence_confidence: 0.6,
                verdict: Verdict::Unsupported,
                structurally_suspicious: false,
                counter_weight: 0.0,
                estimated_exposure_paise: 0,
            },
        };

        let summary = InvestigationSummary {
            verdict: match result.verdict {
                Verdict::Supported => risk_governor_types::InvestigationVerdict::Supported,
                Verdict::Conflicted => risk_governor_types::InvestigationVerdict::Conflicted,
                Verdict::Unsupported => risk_governor_types::InvestigationVerdict::Unsupported,
            },
            evidence_confidence: result.evidence_confidence,
            support_signals: result.supporting.len() as u32,
            contradiction_count: result.counter.len() as u32,
            structurally_suspicious: result.structurally_suspicious,
            counter_weight: result.counter_weight,
            estimated_exposure_paise: result.estimated_exposure_paise,
        };

        let payload = serde_json::to_value(&result).map_err(|e| {
            action_service::ActionServiceError::Validation(format!("investigation payload: {e}"))
        })?;

        Ok((summary, payload))
    }
}