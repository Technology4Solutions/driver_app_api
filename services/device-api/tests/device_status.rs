use axum::{body::Body, http::Request, Extension, Router};
use base64::{engine::general_purpose::STANDARD, Engine};
use device_api::{device_status::Config, router, telemetry::Metrics};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::{mysql::MySqlPoolOptions, MySqlPool};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

fn config(endpoint: &str) -> Config {
    Config {
        username: "api-user".into(),
        password: "api:password".into(),
        previous_password: "old-password".into(),
        broadcast_enabled: true,
        broadcast_endpoint: endpoint.into(),
        broadcast_timeout: Duration::from_secs(2),
    }
}
fn app(pool: MySqlPool, config: Config) -> Router {
    router(Metrics::new().unwrap())
        .layer(Extension(pool))
        .layer(Extension(Arc::new(config)))
}
fn sample() -> Value {
    serde_json::from_str(include_str!("fixtures/device_status.json")).unwrap()
}
async fn post(app: Router, auth: Option<&str>, body: &str) -> Value {
    let mut request =
        Request::post("/device/DeviceStatus/update").header("content-type", "application/json");
    if let Some(auth) = auth {
        request = request.header("authorization", auth);
    }
    let response = app
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    assert!(response.status() == 200 || response.status() == 400);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
fn authorization(password: &str) -> String {
    format!("Basic {}", STANDARD.encode(format!("api-user:{password}")))
}
fn lazy_pool() -> MySqlPool {
    MySqlPoolOptions::new()
        .acquire_timeout(Duration::from_millis(20))
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap()
}

#[tokio::test]
async fn authentication_fails_closed_before_database_access() {
    let router = app(lazy_pool(), config("tcp://127.0.0.1:1"));
    for auth in [
        None,
        Some("Basic !!!!"),
        Some("Bearer token"),
        Some("Basic dXNlcg=="),
    ] {
        let body = post(router.clone(), auth, &sample().to_string()).await;
        assert_eq!(
            body,
            json!({"ReaxiumResponse":{"code":3,"message":"Invalid Authentication","object":[]}})
        );
    }
    let mut empty = config("tcp://127.0.0.1:1");
    empty.username.clear();
    empty.password.clear();
    empty.previous_password.clear();
    let body = post(app(lazy_pool(), empty), Some(&authorization("")), "{}").await;
    assert_eq!(body["ReaxiumResponse"]["message"], "Invalid Authentication");
}

#[tokio::test]
async fn current_and_previous_passwords_accept_basic_auth_and_validate_payload() {
    let router = app(lazy_pool(), config("tcp://127.0.0.1:1"));
    for password in ["api:password", "old-password"] {
        let auth = authorization(password);
        let body = post(router.clone(), Some(&auth), "{}").await;
        assert_eq!(body["ReaxiumResponse"]["code"], 500);
        let body = post(router.clone(), Some(&auth), "{").await;
        assert_eq!(body["ReaxiumResponse"]["code"], 500);
        let body = post(
            router.clone(),
            Some(&auth),
            "{\"ReaxiumParameters\":{\"DeviceStatus\":{}}}",
        )
        .await;
        assert_eq!(body["ReaxiumResponse"]["code"], 500);
    }
}

#[test]
fn numeric_strings_dates_and_coordinates_are_validated() {
    let mut value = sample()["ReaxiumParameters"]["DeviceStatus"].clone();
    value["user_id"] = json!("1");
    value["device_location"]["latitude"] = json!("33.5731");
    value["device_activity"] = json!({"route_id":"7","users_in_track":[1,"2"]});
    assert!(
        serde_json::from_value::<device_api::device_status::Update>(value.clone())
            .unwrap()
            .valid()
    );
    value["status_date"] = json!("2026-02-30 10:00:00");
    assert!(
        !serde_json::from_value::<device_api::device_status::Update>(value.clone())
            .unwrap()
            .valid()
    );
    value["status_date"] = json!("2026-10-20 10:00:00");
    value["device_location"]["latitude"] = json!(91);
    assert!(
        !serde_json::from_value::<device_api::device_status::Update>(value)
            .unwrap()
            .valid()
    );
}

#[tokio::test]
async fn valid_status_without_broker_returns_queueing_error() {
    let request = Request::post("/api/DeviceStatus/updateStatus")
        .header("content-type", "application/json")
        .header("authorization", authorization("api:password"))
        .body(Body::from(sample().to_string()))
        .unwrap();
    let response = app(lazy_pool(), config("tcp://127.0.0.1:1"))
        .oneshot(request)
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["ReaxiumResponse"]["code"], 500);
}

#[test]
fn charging_accepts_java_boolean_numeric_and_null_values() {
    for value in [json!(true), json!(false), json!(1), json!(0), Value::Null] {
        let mut request = sample()["ReaxiumParameters"]["DeviceStatus"].clone();
        request["device_status"]["is_charging"] = value;
        assert!(
            serde_json::from_value::<device_api::device_status::Update>(request)
                .unwrap()
                .valid()
        );
    }
}

async fn wait_locations(pool: &MySqlPool, expected: i64) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_location")
                .fetch_one(pool)
                .await
                .unwrap();
            if count == expected {
                return;
            }
            assert!(count < expected, "unexpected extra location: {count}");
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("worker did not persist expected updates");
}

#[tokio::test]
#[ignore = "requires disposable TEST_DATABASE_URL, TEST_RABBITMQ_URL, TEST_REDIS_URL"]
async fn queues_filters_persists_and_broadcasts_like_java() {
    use device_api::device_status::queue;
    use lapin::{options::*, types::FieldTable, BasicProperties, Connection, ConnectionProperties};
    let pool = MySqlPool::connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let name: String = sqlx::query_scalar("SELECT DATABASE()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        name, "device_api_migration_test",
        "only use the disposable test schema"
    );
    sqlx::raw_sql(include_str!("fixtures/device_status.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE routes_stops_relationship LIKE routesStopsRelationship; INSERT INTO routes_stops_relationship SELECT * FROM routesStopsRelationship;")
        .execute(&pool).await.unwrap();

    let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
    let http = axum::Router::new().route(
        "/dispatch",
        axum::routing::post(
            move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
                let sent = sent.clone();
                async move {
                    sent.send((headers, body)).unwrap();
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    std::env::set_var("BROADCAST_ENABLED", "true");
    std::env::set_var(
        "DEVICES_API_EXPORT_DEVICE_STATUS_URL",
        format!("http://{}/dispatch", listener.local_addr().unwrap()),
    );
    std::env::set_var("DEVICE_STATUS_BROADCAST_USERNAME", "broadcast-user");
    std::env::set_var("DEVICE_STATUS_BROADCAST_PASSWORD", "broadcast-pass");
    let server = tokio::spawn(async move {
        axum::serve(listener, http).await.unwrap();
    });
    let queue_config = queue::Config {
        url: std::env::var("TEST_RABBITMQ_URL").unwrap(),
        redis_url: std::env::var("TEST_REDIS_URL").unwrap(),
        queue: "device-status-parity-test".into(),
        exchange: "device-status-parity-test".into(),
        routing_key: "reaxium.devices.api.#".into(),
        min_interval_seconds: 15,
    };
    let broker = Connection::connect(&queue_config.url, ConnectionProperties::default())
        .await
        .unwrap();
    let channel = broker.create_channel().await.unwrap();
    let mut redis = redis::Client::open(queue_config.redis_url.clone())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    let _: () = redis::cmd("DEL")
        .arg("device:last_status_date:registered-device")
        .query_async(&mut redis)
        .await
        .unwrap();
    let (publisher, worker) = queue::start(pool.clone(), queue_config.clone())
        .await
        .unwrap();
    let router = app(pool.clone(), config("tcp://127.0.0.1:1")).layer(Extension(publisher.clone()));
    let auth = authorization("api:password");
    let mut request = sample();
    request["ReaxiumParameters"]["DeviceStatus"]["device_activity"] =
        json!({"route_id":7,"users_in_track":[1]});
    request["ReaxiumParameters"]["DeviceStatus"]["device_status"]["last_sync"] =
        json!("2026-10-19 12:00:00");
    request["ReaxiumParameters"]["DeviceStatus"]["device_status"]["is_charging"] = json!(true);
    let result = post(router.clone(), Some(&auth), &request.to_string()).await;
    assert_eq!(
        result,
        json!({"ReaxiumResponse":{"code":0,"message":"Device Status queued","object":[]}})
    );
    let (headers, body) = tokio::time::timeout(Duration::from_secs(15), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        headers["authorization"],
        format!("Basic {}", STANDARD.encode("broadcast-user:broadcast-pass"))
    );
    let content = &body["ReaxiumParameters"]["WebSocketPusher"]["content"];
    assert_eq!(content["gps_serial"], "registered-device");
    assert_eq!(content["last_associated_traffic"], 3);
    assert_eq!(
        content["device"]["status"]["last_sync"],
        "2026-10-19 12:00:00"
    );
    assert_eq!(content["device"]["status"]["is_charging"], true);
    assert_eq!(content["device"]["activity"]["users_in_track"], json!([1]));
    assert_eq!(
        content["device"]["activity"]["route"]["route_address"],
        "Test Route"
    );
    assert!(content["driver"].get("password").is_none());
    let age: i64 = sqlx::query_scalar("SELECT CAST(ABS(TIMESTAMPDIFF(SECOND, activity_date, UTC_TIMESTAMP())) AS SIGNED) FROM device_activity").fetch_one(&pool).await.unwrap();
    assert!(age < 30);

    // FIFO sentinel at +15 proves duplicates, older and too-frequent messages were skipped.
    for date in [
        "2026-10-20 10:00:00",
        "2026-10-20 09:59:00",
        "2026-10-20 10:00:14",
        "2026-10-20 10:00:15",
    ] {
        request["ReaxiumParameters"]["DeviceStatus"]["status_date"] = json!(date);
        publisher.publish(&request).await.unwrap();
    }
    wait_locations(&pool, 2).await;
    tokio::time::timeout(Duration::from_secs(5), received.recv())
        .await
        .unwrap()
        .unwrap();
    let ttl: i64 = redis::cmd("TTL")
        .arg("device:last_status_date:registered-device")
        .query_async(&mut redis)
        .await
        .unwrap();
    assert!((86300..=86400).contains(&ttl));

    // A poison message is rejected without requeue; the following valid message still runs.
    channel
        .basic_publish(
            &queue_config.exchange,
            &queue_config.routing_key,
            BasicPublishOptions::default(),
            b"not-json",
            BasicProperties::default(),
        )
        .await
        .unwrap()
        .await
        .unwrap();
    request["ReaxiumParameters"]["DeviceStatus"]["status_date"] = json!("2026-10-20 10:00:30");
    request["ReaxiumParameters"]["DeviceStatus"]["device_activity"] = Value::Null;
    request["ReaxiumParameters"]["DeviceStatus"]["device_status"]["last_sync"] = Value::Null;
    publisher.publish(&request).await.unwrap();
    wait_locations(&pool, 3).await;
    tokio::time::timeout(Duration::from_secs(5), received.recv())
        .await
        .unwrap()
        .unwrap();
    let sync: Option<String> =
        sqlx::query_scalar("SELECT CAST(last_sync AS CHAR) FROM device_status")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(sync, None);
    let activities: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_activity")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(activities, 0);

    // A failed status write rolls back the location. A later FIFO sentinel succeeds.
    sqlx::raw_sql("CREATE TRIGGER fail_status BEFORE UPDATE ON device_status FOR EACH ROW BEGIN IF NEW.last_status_update='2026-10-20 10:00:45' THEN SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='forced failure'; END IF; END")
        .execute(&pool).await.unwrap();
    for date in ["2026-10-20 10:00:45", "2026-10-20 10:01:00"] {
        request["ReaxiumParameters"]["DeviceStatus"]["status_date"] = json!(date);
        publisher.publish(&request).await.unwrap();
    }
    wait_locations(&pool, 4).await;
    tokio::time::timeout(Duration::from_secs(5), received.recv())
        .await
        .unwrap()
        .unwrap();
    worker.shutdown().await;
    let queued = channel
        .queue_declare(
            &queue_config.queue,
            QueueDeclareOptions {
                passive: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        queued.message_count(),
        0,
        "HTTP 503 must not requeue saved telemetry"
    );
    assert!(received.try_recv().is_err());

    // Messages queued without a consumer are processed after restart, with broadcasting disabled.
    request["ReaxiumParameters"]["DeviceStatus"]["status_date"] = json!("2026-10-20 10:01:15");
    channel
        .basic_publish(
            &queue_config.exchange,
            &queue_config.routing_key,
            BasicPublishOptions::default(),
            &serde_json::to_vec(&request).unwrap(),
            BasicProperties::default().with_delivery_mode(2),
        )
        .await
        .unwrap()
        .await
        .unwrap();
    std::env::set_var("BROADCAST_ENABLED", "false");
    let (publisher, worker) = queue::start(pool.clone(), queue_config.clone())
        .await
        .unwrap();
    wait_locations(&pool, 5).await;
    worker.shutdown().await;
    assert!(received.try_recv().is_err());
    assert!(
        publisher.publish(&request).await.is_err(),
        "stopped worker must not falsely report queued"
    );
    channel
        .queue_delete(&queue_config.queue, QueueDeleteOptions::default())
        .await
        .unwrap();
    channel
        .exchange_delete(&queue_config.exchange, ExchangeDeleteOptions::default())
        .await
        .unwrap();
    broker.close(200, "test complete").await.unwrap();
    server.abort();
    pool.close().await;
}
