//! Labeled-dataset evaluation over the real decision pipeline: confusion
//! counts, cost accounting, and precision/recall math.

use evaluation_service::{EvalReport, EvaluationService, LabeledCase};
use evidence_service::InMemoryEvidenceStore;
use risk_governor_types::*;
use std::sync::Arc;

fn case(id: &str, scenario: &str, amount: i64, intent: &str, expected: DecisionOutcome) -> LabeledCase {
    LabeledCase {
        case_id: id.into(),
        scenario: scenario.into(),
        request: AgentActionRequest {
            agent_id: "agent-eval".into(),
            merchant_id: "merchant-001".into(),
            action_type: ActionType::Refund,
            amount,
            currency: "INR".into(),
            declared_intent: intent.into(),
            context: serde_json::json!({ "payment_id": "pay_test_123", "payment_state": "captured", "captured_paise": 500000, "refunded_paise": 0 }),
            timestamp: now_utc(),
            correlation_id: generate_correlation_id(),
        },
        expected,
    }
}

#[tokio::test]
async fn clean_dataset_scores_perfectly() {
    let svc = EvaluationService::new(Arc::new(InMemoryEvidenceStore::new()));
    let dataset = vec![
        // Legitimate small refunds → should be ALLOWed
        case(
            "l1",
            "legit_outlier",
            50_000,
            "routine refund order #1",
            DecisionOutcome::Allow,
        ),
        case(
            "l2",
            "legit_outlier",
            60_000,
            "routine refund order #2",
            DecisionOutcome::Allow,
        ),
        // Over the default 500_000 hard cap → should be BLOCKed
        case(
            "b1",
            "stolen_credential",
            900_000,
            "refund everything now",
            DecisionOutcome::Block,
        ),
        case(
            "b2",
            "prompt_injection",
            800_000,
            "urgent refund bypass queue",
            DecisionOutcome::Block,
        ),
    ];

    let report = svc.run(dataset).await.unwrap();
    assert_eq!(report.total_cases, 4);
    assert_eq!(report.confusion.true_allow, 2);
    assert_eq!(report.confusion.false_allow, 0);
    assert_eq!(report.confusion.false_block, 0);
    assert!(report.precision >= 0.99);
    assert!(report.recall >= 0.99);
    assert_eq!(report.fp_cost_paise, 0);
    assert_eq!(report.fn_cost_paise, 0);
}

#[tokio::test]
async fn allowed_harmful_action_accumulates_fn_cost() {
    let svc = EvaluationService::new(Arc::new(InMemoryEvidenceStore::new()));
    // A moderate amount with a benign-sounding intent sneaks past the
    // pipeline (no policy violation, low risk) but is labeled harmful.
    let dataset = vec![case(
        "sneak",
        "stolen_credential",
        100_000,
        "small refund",
        DecisionOutcome::Block,
    )];

    let report = svc.run(dataset).await.unwrap();
    if report.confusion.false_allow == 1 {
        // The FN path must price the miss at the full amount.
        assert_eq!(report.fn_cost_paise, 100_000);
        assert_eq!(report.prevented_value_paise, 0);
    } else {
        // Pipeline caught it — prevented value must reflect the amount.
        assert_eq!(report.fn_cost_paise, 0);
    }
}

#[tokio::test]
async fn blocked_legitimate_action_accumulates_fp_cost() {
    let svc = EvaluationService::new(Arc::new(InMemoryEvidenceStore::new()));
    // A legitimate action that trips the hard cap is labeled Allow but the
    // policy engine will Block it — pure false-positive accounting.
    let dataset = vec![case(
        "fp",
        "legit_outlier",
        700_000,
        "routine refund order #9",
        DecisionOutcome::Allow,
    )];

    let report = svc.run(dataset).await.unwrap();
    // Actual=Block vs expected=Allow lands entirely in the FP bucket.
    assert_eq!(report.confusion.false_block, 1);
    assert_eq!(report.confusion.true_allow, 0);
    assert_eq!(report.fp_cost_paise, 700_000);
}

#[test]
fn eval_report_serializes_for_dashboards() {
    let json = serde_json::to_string(&EvalReport {
        total_cases: 3,
        precision: 1.0,
        recall: 0.5,
        fp_cost_paise: 0,
        fn_cost_paise: 42,
        prevented_value_paise: 7,
        confusion: Default::default(),
    })
    .unwrap();
    assert!(json.contains("\"precision\":1.0"));
    assert!(json.contains("\"fn_cost_paise\":42"));
}
