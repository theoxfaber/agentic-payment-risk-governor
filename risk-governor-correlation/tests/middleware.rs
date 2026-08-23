use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use risk_governor_correlation::{current_correlation_id, CorrelationLayer, RequestCorrelation};
use std::convert::Infallible;
use tower::{Layer, Service, ServiceExt};

fn echo_service() -> tower::util::BoxCloneService<Request<Body>, Response<Body>, Infallible> {
    use tower::util::BoxCloneService;
    BoxCloneService::new(tower::service_fn(|_req: Request<Body>| async move {
        Ok::<_, Infallible>(Response::builder().body(Body::empty()).unwrap())
    }))
}

/// No header → middleware generates one; same ID lands in the task-local
/// inside the handler AND in the response header.
#[tokio::test]
async fn generates_and_propagates_when_missing() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut svc = CorrelationLayer.layer(tower::service_fn(move |_req: Request<Body>| {
        let tx = tx.clone();
        async move {
            tx.send(current_correlation_id()).unwrap();
            Ok::<_, Infallible>(Response::builder().body(Body::empty()).unwrap())
        }
    }));

    let res = svc
        .ready()
        .await
        .unwrap()
        .call(Request::builder().body(Body::empty()).unwrap())
        .await
        .unwrap();

    let seen_in_handler = rx.recv().await.unwrap();
    let echoed = res.headers()["x-correlation-id"].to_str().unwrap().to_string();

    assert_eq!(seen_in_handler.to_string(), echoed);
}

/// Header present → echoed verbatim everywhere (client-side tracing joins up).
#[tokio::test]
async fn honors_incoming_header() {
    let incoming = uuid::Uuid::new_v4();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut svc = CorrelationLayer.layer(tower::service_fn(move |req: Request<Body>| {
        let tx = tx.clone();
        async move {
            // Handler reads the extension, not headers — proves insertion happened
            let cid = req
                .extensions()
                .get::<RequestCorrelation>()
                .expect("extension inserted")
                .0;
            tx.send((cid, current_correlation_id())).unwrap();
            Ok::<_, Infallible>(Response::builder().body(Body::empty()).unwrap())
        }
    }));

    let res = svc
        .ready()
        .await
        .unwrap()
        .call(
            Request::builder()
                .header("x-correlation-id", incoming.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let (ext_cid, scoped_cid) = rx.recv().await.unwrap();
    assert_eq!(ext_cid, incoming);
    assert_eq!(scoped_cid, incoming);
    assert_eq!(
        res.headers()["x-correlation-id"].to_str().unwrap(),
        incoming.to_string()
    );
}

/// Malformed header value → treated as absent (fresh ID), never a 400.
/// A tracing/observability concern must not become a request-rejection path.
#[tokio::test]
async fn malformed_header_falls_back_to_generated() {
    let mut svc = CorrelationLayer.layer(echo_service());
    let res = svc
        .ready()
        .await
        .unwrap()
        .call(
            Request::builder()
                .header("x-correlation-id", "not-a-uuid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let echoed = res.headers()["x-correlation-id"].to_str().unwrap();
    assert!(uuid::Uuid::parse_str(echoed).is_ok(), "got {echoed}");
    // And it still succeeded rather than erroring
    assert_eq!(res.status(), StatusCode::OK);
}
