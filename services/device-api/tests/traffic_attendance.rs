#[path = "support/jwt.rs"]
mod jwt;
use jwt::support_token;

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Extension,
};
use device_api::{device_status::Config, router, telemetry::Metrics};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

#[tokio::test]
async fn authentication_validation_and_unavailable_queue_keep_response_contract() {
    let app = router(Metrics::new().unwrap()).layer(Extension(Arc::new(Config {
        jwt_secret: "test-signing-secret-at-least-32-bytes".into(),
        dispatcher_token: "test-dispatcher-token".into(),
        broadcast_enabled: false,
        broadcast_endpoint: String::new(),
        broadcast_timeout: Duration::from_secs(1),
    })));
    for (auth, body, status, code, message) in [
        (false, "{}", StatusCode::OK, 3, "Invalid Authentication"),
        (
            true,
            "{",
            StatusCode::BAD_REQUEST,
            500,
            "Error reporting Traffic attendance record",
        ),
        (
            true,
            "{}",
            StatusCode::BAD_REQUEST,
            500,
            "Error reporting Traffic attendance record",
        ),
        (
            true,
            r#"{"ReaxiumParameters":{"UserAccess":{"user_id":6}}}"#,
            StatusCode::BAD_REQUEST,
            500,
            "Error reporting Traffic attendance record",
        ),
    ] {
        let mut request = Request::post("/api/Device/saveTrafficAttendance")
            .header("content-type", "application/json");
        if auth {
            request = request.header(
                "authorization",
                support_token("test", "secret"),
            );
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        let body: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(
            body,
            json!({"ReaxiumResponse":{"code":code,"message":message,"object":[]}})
        );
    }
}

#[tokio::test]
#[ignore = "requires disposable TEST_DATABASE_URL and TEST_RABBITMQ_URL"]
async fn queued_events_are_inserted_without_deduplication() {
    use device_api::traffic_attendance::queue;
    let pool = sqlx::MySqlPool::connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let (database,): (String,) = sqlx::query_as("SELECT DATABASE()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        database, "device_api_migration_test",
        "only use the disposable test database"
    );
    sqlx::query("CREATE TABLE IF NOT EXISTS traffic_attendance (traffic_attendance_id BIGINT PRIMARY KEY AUTO_INCREMENT, device_id VARCHAR(100), traffic_type_id INT, access_id INT, traffic_info TEXT, datetime VARCHAR(100), session_uid INT, activity_id INT, place_id INT, trace_id VARCHAR(100), user_id INT)").execute(&pool).await.unwrap();
    let trace = format!("traffic-attendance-test-{}", std::process::id());
    sqlx::query("DELETE FROM traffic_attendance WHERE trace_id = ?")
        .bind(&trace)
        .execute(&pool)
        .await
        .unwrap();
    let config = queue::Config {
        url: std::env::var("TEST_RABBITMQ_URL").unwrap(),
        queue: format!("traffic-attendance-test-{}", std::process::id()),
        exchange: format!("traffic-attendance-test-{}", std::process::id()),
        routing_key: "attendance.test".into(),
    };
    let (publisher, worker) = queue::start(pool.clone(), config.clone()).await.unwrap();
    let app = router(Metrics::new().unwrap())
        .layer(Extension(publisher))
        .layer(Extension(Arc::new(Config {
            jwt_secret: "test-signing-secret-at-least-32-bytes".into(),
            dispatcher_token: "test-dispatcher-token".into(),
            broadcast_enabled: false,
            broadcast_endpoint: String::new(),
            broadcast_timeout: Duration::from_secs(1),
        })));
    let event = json!({"ReaxiumParameters":{"UserAccess":{
        "device_id":"42", "activity_id":4, "place_id":5, "traffic_type":1001, "access_type":2, "traffic_info":"Check in",
        "trace_id":trace, "user_id":6
    }}});
    for _ in 0..2 {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/Device/saveTrafficAttendance")
                    .header("content-type", "application/json")
                    .header(
                        "authorization",
                        support_token("test", "secret"),
                    )
                    .body(Body::from(event.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(
            body,
            json!({"ReaxiumResponse":{"code":0,"message":"traffic Attendance record queued","object":[]}})
        );
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM traffic_attendance WHERE trace_id = ? AND device_id = '42' AND activity_id = 4 AND place_id = 5 AND traffic_type_id = 1001 AND access_id = 2 AND traffic_info = 'Check in' AND user_id = 6 AND session_uid = 0 AND datetime IS NOT NULL").bind(&trace).fetch_one(&pool).await.unwrap();
            if count == 2 { break; }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await.unwrap();
    worker.shutdown().await;
    sqlx::query("DELETE FROM traffic_attendance WHERE trace_id = ?")
        .bind(&trace)
        .execute(&pool)
        .await
        .unwrap();
    let connection =
        lapin::Connection::connect(&config.url, lapin::ConnectionProperties::default())
            .await
            .unwrap();
    let channel = connection.create_channel().await.unwrap();
    channel
        .queue_delete(&config.queue, lapin::options::QueueDeleteOptions::default())
        .await
        .unwrap();
    channel
        .exchange_delete(
            &config.exchange,
            lapin::options::ExchangeDeleteOptions::default(),
        )
        .await
        .unwrap();
    connection.close(200, "test complete").await.unwrap();
    pool.close().await;
}
