use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use device_api::{admin_router, router, telemetry::Metrics};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn hello_world_returns_plain_text() {
    let response = router(Metrics::new().unwrap())
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"Hello, world!");
}

#[tokio::test]
async fn monitoring_is_separate_and_observes_requests() {
    let metrics = Metrics::new().unwrap();
    let app = router(metrics.clone());
    let response = app
        .clone()
        .oneshot(Request::get("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let admin = admin_router(metrics);
    let response = admin
        .clone()
        .oneshot(Request::get("/health/live").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = admin
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.contains("http_requests_total"));
    assert!(text.contains("route=\"/\""));
}
