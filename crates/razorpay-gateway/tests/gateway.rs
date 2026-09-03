//! Gateway behavior without network: mock recording, signature verification,
//! and URL construction for the money-movement endpoints.

use action_service::RazorpayGateway as _;
use hmac::Mac;
use razorpay_gateway::{verify_webhook_signature, MockGateway};
use risk_governor_types::*;
use std::sync::Arc;

fn refund_request(amount: i64) -> AgentActionRequest {
    AgentActionRequest {
        agent_id: "agent-01".into(),
        merchant_id: "merchant-001".into(),
        action_type: ActionType::Refund,
        amount,
        currency: "INR".into(),
        declared_intent: "refund order #1".into(),
        context: serde_json::json!({ "payment_id": "pay_TEST123" }),
        timestamp: now_utc(),
        correlation_id: generate_correlation_id(),
    }
}

#[tokio::test]
async fn mock_gateway_records_every_call_with_decision_id() {
    let gw = Arc::new(MockGateway::default());
    let d1 = generate_correlation_id();
    let d2 = generate_correlation_id();

    gw.execute(&refund_request(50_000), d1).await.unwrap();
    gw.execute(&refund_request(75_000), d2).await.unwrap();

    let calls = gw.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, d1);
    assert_eq!(calls[1].0, d2);
    assert_eq!(calls[1].1["amount"], 75_000);
}

#[tokio::test]
async fn mock_response_is_deterministic_per_decision() {
    let gw = MockGateway::default();
    let id = generate_correlation_id();
    let resp = gw.execute(&refund_request(10_000), id).await.unwrap();
    assert_eq!(resp["id"], format!("rfnd_mock_{id}"));
    assert_eq!(resp["status"], "processed");
}

#[test]
fn webhook_signature_round_trip_and_rejection() {
    let body = br#"{"event":"refund.processed","payload":{}}"#;
    let secret = "whsec_live_or_test";
    let mut mac = hmac::Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let good = hex::encode(mac.finalize().into_bytes());
    assert!(verify_webhook_signature(body, &good, secret));
    // case-insensitive hex comparison
    assert!(verify_webhook_signature(body, &good.to_uppercase(), secret));
    assert!(!verify_webhook_signature(body, &format!("{}0", &good[..63]), secret));
}

// ---------------------------------------------------------------------------
// Idempotency behavior against a local mock Razorpay (no external network)
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Scriptable mock of the two endpoints the refund path touches.
#[derive(Clone)]
struct MockRzp {
    refund_posts: Arc<AtomicUsize>,
    /// First POST fails with this behavior instead of succeeding.
    first_post: Arc<FirstPost>,
    recorded: Arc<AtomicBool>,
    /// Last POST body captured (receipt assertion).
    last_body: Arc<std::sync::Mutex<Option<serde_json::Value>>>,
}

#[derive(Clone, Copy)]
enum FirstPost {
    /// Normal processing succeeds immediately.
    Succeed,
    /// Money moved server-side but the response was lost (500).
    LostResponse,
    /// Genuine transient failure — nothing recorded server-side (502).
    Transient,
}

async fn spawn_mock_rzp(first_post: FirstPost) -> (String, MockRzp) {
    spawn_mock_rzp_with_list(first_post, None).await
}

/// `list_items` overrides what GET /refunds reports (defaults to mirroring
/// the mock's internal recorded state).
async fn spawn_mock_rzp_with_list(first_post: FirstPost, list_items: Option<serde_json::Value>) -> (String, MockRzp) {
    use axum::response::IntoResponse;

    let state = MockRzp {
        refund_posts: Arc::new(AtomicUsize::new(0)),
        first_post: Arc::new(first_post),
        recorded: Arc::new(AtomicBool::new(false)),
        last_body: Arc::new(std::sync::Mutex::new(None)),
    };

    let post_state = state.clone();
    let list_state = state.clone();

    let app = axum::Router::new()
        .route(
            "/payments/:id/refund",
            axum::routing::post(
                move |axum::extract::Path(_id): axum::extract::Path<String>,
                      axum::Json(body): axum::Json<serde_json::Value>| async move {
                    let n = post_state.refund_posts.fetch_add(1, Ordering::SeqCst) + 1;
                    *post_state.last_body.lock().unwrap() = Some(body);
                    if n == 1 && !matches!(*post_state.first_post, FirstPost::Succeed) {
                        match *post_state.first_post {
                            FirstPost::LostResponse => {
                                // Money moved server-side; response lost.
                                post_state.recorded.store(true, Ordering::SeqCst);
                                return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
                            }
                            _ => return axum::http::StatusCode::BAD_GATEWAY.into_response(),
                        }
                    }
                    post_state.recorded.store(true, Ordering::SeqCst);
                    axum::Json(serde_json::json!({ "id": "rfnd_MOCK", "status": "processed" })).into_response()
                },
            ),
        )
        .route(
            "/payments/:id/refunds",
            axum::routing::get(move || async move {
                if let Some(items) = &list_items {
                    return axum::Json(serde_json::json!({ "items": items }));
                }
                if list_state.recorded.load(Ordering::SeqCst) {
                    // Mirror a real Razorpay: the landed refund echoes OUR
                    // receipt (decision_id), which is what refund_landed
                    // matches on. Amount-only items must NOT dedup.
                    let receipt = list_state
                        .last_body
                        .lock()
                        .unwrap()
                        .clone()
                        .and_then(|b| b.get("receipt").and_then(|r| r.as_str()).map(|s| s.to_string()))
                        .unwrap_or_default();
                    axum::Json(serde_json::json!({ "items": [ { "amount": 50_000, "receipt": receipt } ] }))
                } else {
                    axum::Json(serde_json::json!({ "items": [] }))
                }
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), state)
}

fn gateway(base: &str) -> razorpay_gateway::HttpGateway {
    razorpay_gateway::HttpGateway::new("rzp_test_x", "secret").with_base_url(base.to_string())
}

#[tokio::test]
async fn duplicate_decision_id_executes_exactly_once() {
    let (base, mock) = spawn_mock_rzp(FirstPost::Succeed).await;
    let gw = gateway(&base);
    use action_service::RazorpayGateway as _;

    let decision_id = generate_correlation_id();
    let r1 = gw.execute(&refund_request(50_000), decision_id).await.unwrap();
    let r2 = gw.execute(&refund_request(50_000), decision_id).await.unwrap();

    assert_eq!(
        mock.refund_posts.load(Ordering::SeqCst),
        1,
        "second execute must not re-fire"
    );
    assert_eq!(r1, r2, "duplicate must receive the cached response");

    // A DIFFERENT decision is a different action — it goes through.
    gw.execute(&refund_request(50_000), generate_correlation_id())
        .await
        .unwrap();
    assert_eq!(mock.refund_posts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn refund_landing_despite_5xx_is_not_double_fired() {
    let (base, mock) = spawn_mock_rzp(FirstPost::LostResponse).await;
    let gw = gateway(&base);
    use action_service::RazorpayGateway as _;

    let resp = gw.execute(&refund_request(50_000), generate_correlation_id()).await;

    let resp = match resp {
        Ok(r) => r,
        Err(e) => panic!("lost-response guard should recover, got: {e}"),
    };
    assert_eq!(resp["deduplicated_after_upstream_error"], true);
    assert_eq!(
        mock.refund_posts.load(Ordering::SeqCst),
        1,
        "guard must probe refunds instead of blind-resending after a 5xx"
    );
}

#[tokio::test]
async fn transient_5xx_without_prior_success_retries_normally() {
    let (base, mock) = spawn_mock_rzp(FirstPost::Transient).await;
    let gw = gateway(&base);
    use action_service::RazorpayGateway as _;

    let resp = gw.execute(&refund_request(50_000), generate_correlation_id()).await;
    assert!(
        resp.is_ok(),
        "transient 502 with no landed refund should retry to success"
    );
    assert_eq!(
        mock.refund_posts.load(Ordering::SeqCst),
        2,
        "one failed attempt + one retry"
    );
}

// ---------------------------------------------------------------------------
// Payout routing: RazorpayX payouts must hit POST /payouts with the
// fund-account payload, not the generic payments endpoint.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn payout_routes_to_razorpayx_payouts_endpoint() {
    let captured = Arc::new(std::sync::Mutex::new(None::<serde_json::Value>));
    let cap = captured.clone();
    let app = axum::Router::new().route(
        "/payouts",
        axum::routing::post(move |body: axum::Json<serde_json::Value>| {
            let cap = cap.clone();
            async move {
                *cap.lock().unwrap() = Some(body.0);
                axum::Json(serde_json::json!({ "id": "pout_MOCK", "status": "processed" }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let gw = gateway(&format!("http://{addr}"));
    use action_service::RazorpayGateway as _;

    let request = AgentActionRequest {
        agent_id: "agent-01".into(),
        merchant_id: "merchant-001".into(),
        action_type: ActionType::Payout,
        amount: 250_000,
        currency: "INR".into(),
        declared_intent: "payout vendor invoice INV-42".into(),
        context: serde_json::json!({ "fund_account_id": "fa_123", "mode": "NEFT" }),
        timestamp: now_utc(),
        correlation_id: generate_correlation_id(),
    };

    let resp = gw.execute(&request, generate_correlation_id()).await.unwrap();
    assert_eq!(resp["status"], "processed");

    let body = captured.lock().unwrap().clone().expect("payout body captured");
    assert_eq!(body["amount"], 250_000);
    assert_eq!(body["fund_account_id"], "fa_123");
    assert_eq!(body["mode"], "NEFT");
    // narration is capped at 30 chars (RazorpayX limit)
    assert!(body["narration"].as_str().unwrap().chars().count() <= 30);
}

#[test]
fn payment_link_routes_off_payments_endpoint() {
    // Endpoint construction is pure — verify without network.
    let gw = gateway("https://api.razorpay.com/v1");
    let request = AgentActionRequest {
        agent_id: "agent-01".into(),
        merchant_id: "merchant-001".into(),
        action_type: ActionType::PaymentLink,
        amount: 10_000,
        currency: "INR".into(),
        declared_intent: "link for order #9".into(),
        context: serde_json::json!({}),
        timestamp: now_utc(),
        correlation_id: generate_correlation_id(),
    };
    let (url, _body) = gw.endpoint_for(&request);
    assert!(url.ends_with("/payment_links"), "payment links must not hit {url}");
}

#[tokio::test]
async fn lost_response_probe_matches_receipt_not_amount() {
    // A FOREIGN refund with the same amount but a different receipt must NOT
    // satisfy the dedup probe — matching is by our decision_id receipt.
    let foreign_refunds = serde_json::json!([
        { "amount": 50_000, "receipt": "some-other-flow-decision" }
    ]);
    let (base, mock) = spawn_mock_rzp_with_list(FirstPost::LostResponse, Some(foreign_refunds)).await;
    let gw = gateway(&base);
    use action_service::RazorpayGateway as _;

    let decision_id = generate_correlation_id();
    let resp = gw.execute(&refund_request(50_000), decision_id).await;
    assert!(resp.is_ok(), "should retry past the foreign refund and succeed");

    // Two POSTs: the lost first attempt, then a legitimate retry (probe
    // correctly rejected the foreign same-amount refund).
    assert_eq!(mock.refund_posts.load(Ordering::SeqCst), 2);

    // The receipt we send IS the decision id — that's what makes the probe exact.
    let body = mock.last_body.lock().unwrap().clone().unwrap();
    assert_eq!(body["receipt"], decision_id.to_string());
}

#[tokio::test]
async fn lost_response_probe_matches_our_receipt_even_at_different_amount() {
    // Inverse: OUR receipt in the list means it landed, even if Razorpay
    // recorded a different amount than requested (partial-refund rounding).
    let our_receipt = serde_json::json!([
        { "amount": 49_999, "receipt": "fixed-receipt-value" }
    ]);
    let (base, _mock) = spawn_mock_rzp_with_list(FirstPost::LostResponse, Some(our_receipt)).await;
    let gw = gateway(&base);
    use action_service::RazorpayGateway as _;

    let decision_id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000000").unwrap();
    // MockGateway-style fixed receipt: execute stamps receipt = decision_id,
    // so craft the list to match whatever decision id we pass.
    let (base, _mock) = {
        // Rebuild list with the actual decision id receipt.
        let items = serde_json::json!([{ "amount": 49_999, "receipt": decision_id.to_string() }]);
        drop(gw);
        spawn_mock_rzp_with_list(FirstPost::LostResponse, Some(items)).await
    };
    let gw = gateway(&base);
    let resp = gw.execute(&refund_request(50_000), decision_id).await.unwrap();
    assert_eq!(resp["deduplicated_after_upstream_error"], true);
}
