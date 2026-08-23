use evidence_service::InMemoryEvidenceStore;
use risk_governor_types::*;
use serde::Serialize;
use std::sync::Arc;

/// One labeled example from the synthetic adversarial dataset.
#[derive(Debug, Clone, Serialize)]
pub struct LabeledCase {
    pub case_id: String,
    pub scenario: String, // e.g. "prompt_injection", "stolen_credential", "legit_outlier"
    pub request: AgentActionRequest,
    /// Ground truth: what SHOULD have happened
    pub expected: DecisionOutcome,
}

#[derive(Debug, Default, Serialize)]
pub struct ConfusionCounts {
    pub true_allow: u32,
    pub false_allow: u32, // FN-cost driver: bad action let through
    pub true_block: u32,
    pub false_block: u32, // FP-cost driver: good action blocked
    pub review_total: u32,
}

#[derive(Debug, Serialize)]
pub struct EvalReport {
    pub total_cases: u32,
    pub precision: f64,
    pub recall: f64,
    pub fp_cost_paise: i64,
    pub fn_cost_paise: i64,
    pub prevented_value_paise: i64,
    pub confusion: ConfusionCounts,
}

pub struct EvaluationService {
    store: Arc<InMemoryEvidenceStore>,
}

impl EvaluationService {
    pub fn new(store: Arc<InMemoryEvidenceStore>) -> Self {
        Self { store }
    }

    /// Runs the labeled dataset through the full in-process decision pipeline.
    pub async fn run(&self, dataset: Vec<LabeledCase>) -> Result<EvalReport, String> {
        let policy = policy_engine::PolicyEngine::new();
        let risk = risk_engine::RiskEngine::default();
        let mut counts = ConfusionCounts::default();
        let mut fp_cost = 0i64;
        let mut fn_cost = 0i64;
        let mut prevented = 0i64;

        for case in &dataset {
            // Seed fresh per-case context so velocity/history reflect the scenario
            self.seed_case(case).await;

            let evidence = match self.store_gather(&case.request).await {
                Ok(ev) => ev,
                Err(e) => return Err(format!("case {}: {e}", case.case_id)),
            };

            let policy_result = policy
                .evaluate(&case.request, &evidence)
                .await
                .map_err(|e| e.to_string())?;
            let risk_result = risk.score(&case.request, &evidence).await.map_err(|e| e.to_string())?;

            let outcome = combine(policy_result.verdict, risk_result.risk_score);

            match (outcome, case.expected) {
                (DecisionOutcome::Allow, DecisionOutcome::Allow) => counts.true_allow += 1,
                (DecisionOutcome::Block, DecisionOutcome::Block) => {
                    counts.true_block += 1;
                    prevented += case.request.amount;
                }
                (DecisionOutcome::Allow, _) => {
                    // FN: harmful action allowed through
                    counts.false_allow += 1;
                    fn_cost += case.request.amount;
                }
                (DecisionOutcome::Block, _) => {
                    // FP: legitimate action blocked
                    counts.false_block += 1;
                    fp_cost += case.request.amount;
                }
                (DecisionOutcome::Review, DecisionOutcome::Review) => counts.review_total += 1,
                (DecisionOutcome::Review, DecisionOutcome::Block) => {
                    // Review is a soft block for cost purposes; count as caught
                    counts.review_total += 1;
                    prevented += case.request.amount;
                }
                (DecisionOutcome::Review, DecisionOutcome::Allow) => {
                    // Review on a legit action = friction, not a hard FP. Track separately.
                    counts.review_total += 1;
                }
            }
        }

        let blocked = counts.true_block as f64;
        let flagged = blocked + counts.false_block as f64;
        let actual_bad = blocked + counts.false_allow as f64;

        Ok(EvalReport {
            total_cases: dataset.len() as u32,
            precision: if flagged > 0.0 { blocked / flagged } else { 0.0 },
            recall: if actual_bad > 0.0 { blocked / actual_bad } else { 0.0 },
            fp_cost_paise: fp_cost,
            fn_cost_paise: fn_cost,
            prevented_value_paise: prevented,
            confusion: counts,
        })
    }

    async fn seed_case(&self, case: &LabeledCase) {
        // Scenario-specific seeding lives with the dataset generator (Phase 4).
        // Here we just ensure the merchant policy exists so gather() succeeds.
        let _ = self
            .store
            .seed_default_policy_if_missing(&case.request.merchant_id)
            .await;
    }

    async fn store_gather(&self, request: &AgentActionRequest) -> Result<Evidence, String> {
        use evidence_service::EvidenceStore as _;
        let agent_history = self
            .store
            .agent_history(&request.agent_id)
            .await
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| AgentHistory {
                agent_id: request.agent_id.clone(),
                total_actions_30d: 10,
                total_volume_30d: 500_000,
                avg_amount: 50_000,
                max_amount: 200_000,
                refund_rate: 0.05,
                block_rate: 0.02,
                review_rate: 0.03,
                first_seen: chrono::Utc::now() - chrono::Duration::days(90),
                last_action: chrono::Utc::now() - chrono::Duration::hours(2),
                action_type_distribution: Default::default(),
                anomaly_flags: vec![],
            });
        let merchant_policy = self
            .store
            .merchant_policy(&request.merchant_id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("merchant policy missing")?;
        let customer_history = None;
        let recent_velocity = self
            .store
            .velocity(&request.agent_id)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Evidence {
            agent_history,
            merchant_policy,
            customer_history,
            recent_velocity,
            fetched_at: now_utc(),
        })
    }
}

fn combine(verdict: PolicyVerdict, risk_score: f64) -> DecisionOutcome {
    match (verdict, risk_score) {
        (PolicyVerdict::Block, _) => DecisionOutcome::Block,
        (PolicyVerdict::Allow, s) if s >= 0.8 => DecisionOutcome::Block,
        (PolicyVerdict::Allow, s) if s >= 0.5 => DecisionOutcome::Review,
        (PolicyVerdict::Allow, _) => DecisionOutcome::Allow,
    }
}
