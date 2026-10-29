use axum::{body::Body, http::Request, Extension, Router};
use base64::{engine::general_purpose::STANDARD, Engine};
use device_api::{
    device_status::Config,
    device_traffic::{BeginRoute, VisionConfig},
    router,
    telemetry::Metrics,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::{mysql::MySqlPoolOptions, MySqlPool, Row};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tower::ServiceExt;
use zeromq::{Socket, SocketRecv};

fn sample() -> Value {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../run-and-install/json/begin-route.example.json"
    ))
    .unwrap();
    let r = &mut value["ReaxiumParameters"]["DeviceTraffic"];
    r["device_id"] = json!(42);
    r["stop_id"] = json!(9);
    r["route_id"] = json!(7);
    r["driver_id"] = json!(1);
    r["bus_info"] = json!("2");
    value
}
fn make_app(pool: MySqlPool, endpoint: &str, udp: Option<SocketAddr>) -> Router {
    router(Metrics::new().unwrap())
        .layer(Extension(pool))
        .layer(Extension(Arc::new(Config {
            username: "test".into(),
            password: "test-password".into(),
            previous_password: "previous".into(),
            broadcast_enabled: true,
            broadcast_endpoint: endpoint.into(),
            broadcast_timeout: Duration::from_millis(300),
        })))
        .layer(Extension(Arc::new(VisionConfig {
            enabled: true,
            destination: udp,
            timezone: chrono_tz::America::New_York,
        })))
}
async fn post(app: Router, path: &str, body: Value, password: Option<&str>) -> Value {
    let mut request = Request::post(path).header("content-type", "application/json");
    if let Some(password) = password {
        request = request.header(
            "authorization",
            format!("Basic {}", STANDARD.encode(format!("test:{password}"))),
        );
    }
    let response = app
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
#[tokio::test]
async fn both_paths_authenticate_and_reject_bad_input_without_database_access() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = make_app(pool, "tcp://127.0.0.1:1", None);
    for path in [
        "/DeviceTraffic/beginARoute",
        "/school_bus_cloud/DeviceTraffic/beginARoute",
    ] {
        let result = post(app.clone(), path, sample(), None).await;
        assert_eq!(
            result["ReaxiumResponse"]["message"],
            "Invalid Authentication"
        );
        for password in ["test-password", "previous"] {
            let mut input = sample();
            input["ReaxiumParameters"]["DeviceTraffic"]["latitude"] = json!(91);
            let result = post(app.clone(), path, input, Some(password)).await;
            assert!(result["ReaxiumResponse"]["message"]
                .as_str()
                .unwrap()
                .starts_with("Invalid Parameters"));
        }
    }
}
#[test]
fn numeric_strings_and_legacy_date_are_valid_but_missing_fields_are_not() {
    let mut input = sample()["ReaxiumParameters"]["DeviceTraffic"].clone();
    input["device_id"] = json!("42");
    input["latitude"] = json!("27.1975");
    input["traffic_date"] = json!("2026-10-21 08::00::00");
    assert!(serde_json::from_value::<BeginRoute>(input.clone())
        .unwrap()
        .valid());
    input.as_object_mut().unwrap().remove("trace_id");
    assert!(serde_json::from_value::<BeginRoute>(input).is_err());
}

#[tokio::test]
#[ignore = "requires disposable TEST_DATABASE_URL named device_api_migration_test"]
async fn saves_broadcasts_sends_udp_and_reports_rollback_and_partial_failure() {
    let pool = MySqlPoolOptions::new()
        .max_connections(5)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let database: String = sqlx::query_scalar("SELECT DATABASE()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        database, "device_api_migration_test",
        "refusing to change a non-test database"
    );
    sqlx::raw_sql(include_str!("fixtures/device_status.sql"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("fixtures/begin_route.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let mut zmq = zeromq::PullSocket::new();
    let endpoint = zmq.bind("tcp://127.0.0.1:0").await.unwrap().to_string();
    let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let unconfigured = make_app(pool.clone(), &endpoint, None);
    let app = make_app(pool.clone(), &endpoint, Some(udp.local_addr().unwrap()));
    let result = post(
        app.clone(),
        "/DeviceTraffic/beginARoute",
        sample(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
    let row = sqlx::query("SELECT * FROM device_traffic")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<i32, _>("traffic_type_id"), 3);
    assert_eq!(row.get::<i32, _>("traffic_situation_id"), 1);
    assert_eq!(row.get::<String, _>("stop_number"), "STOP9");
    assert_eq!(row.get::<String, _>("stop_name"), "Test stop");
    assert_eq!(row.get::<String, _>("assigned_bus"), "REQUEST-BUS");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM route_trigger_audit")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let activity = sqlx::query("SELECT * FROM device_activity")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(activity.get::<i64, _>("route_id"), 7);
    assert_eq!(activity.get::<Option<String>, _>("users_in_track"), None);
    for _ in 0..2 {
        let message = tokio::time::timeout(Duration::from_secs(2), zmq.recv())
            .await
            .unwrap()
            .unwrap();
        let value: Value = serde_json::from_str(&String::try_from(message).unwrap()).unwrap();
        assert_eq!(value["content"]["traffic_label"], "ROUTE_MAJOR_TRAFFIC");
        assert_eq!(value["content"]["typeTrafficId"], 3);
        assert_eq!(value["content"]["traffic_date"], "2026-10-21 08:00:00");
        assert_eq!(value["content"]["channels"].as_array().unwrap().len(), 2);
        assert!(value["content"]["driver_password"].is_null());
    }
    let mut bytes = [0u8; 1024];
    let n = tokio::time::timeout(Duration::from_secs(2), udp.recv(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes[2..5], &[0xca, 0xfe, 1]);
    assert_eq!(&bytes[8..14], b"gps-42");
    assert_eq!(&bytes[14..16], &[0x8e, 1]);
    // New York 08:00 in October corresponds to 12:00 UTC.
    assert_eq!(
        u64::from_be_bytes(bytes[16..24].try_into().unwrap()),
        1792584000000
    );
    assert_eq!(&bytes[n - 9..n - 5], &7u32.to_be_bytes());
    sqlx::raw_sql("CREATE TRIGGER reject_activity BEFORE INSERT ON device_activity FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='test rollback'").execute(&pool).await.unwrap();
    let result = post(
        app.clone(),
        "/DeviceTraffic/beginARoute",
        sample(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 3);
    for table in ["device_traffic", "device_activity", "route_trigger_audit"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(50), udp.recv(&mut bytes))
            .await
            .is_err()
    );
    sqlx::raw_sql("DROP TRIGGER reject_activity")
        .execute(&pool)
        .await
        .unwrap();
    // A missing dashboard join is a delivery failure, not a claim that saved rows vanished.
    sqlx::query("DELETE FROM traffic_type")
        .execute(&pool)
        .await
        .unwrap();
    let result = post(
        app,
        "/school_bus_cloud/DeviceTraffic/beginARoute",
        sample(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 3);
    assert_eq!(result["ReaxiumResponse"]["object"]["database_saved"], true);
    assert_eq!(
        result["ReaxiumResponse"]["object"]["broadcast_status"],
        "failed"
    );
    assert_eq!(result["ReaxiumResponse"]["object"]["vision_status"], "sent");
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_activity")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    sqlx::query("INSERT INTO traffic_type VALUES (3,'ROUTE START')")
        .execute(&pool)
        .await
        .unwrap();
    let result = post(
        unconfigured,
        "/DeviceTraffic/beginARoute",
        sample(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 3);
    assert_eq!(
        result["ReaxiumResponse"]["object"]["vision_status"],
        "not_configured"
    );
    assert_eq!(
        result["ReaxiumResponse"]["object"]["broadcast_status"],
        "sent"
    );
    assert_eq!(result["ReaxiumResponse"]["object"]["database_saved"], true);
    assert_eq!(row.get::<String, _>("bus_info"), "2");
    // End-route lookup must also work without any route-to-bus assignment.
    sqlx::query("DELETE FROM bus_routes_relationship")
        .execute(&pool)
        .await
        .unwrap();
    // End-route rollback must preserve activity and undo the traffic/trigger inserts.
    sqlx::query("INSERT INTO traffic_type VALUES (4,'ROUTE END')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO device_activity (device_id,route_id) VALUES (43,7)")
        .execute(&pool)
        .await
        .unwrap();
    let mut end = sample();
    end["ReaxiumParameters"]["DeviceTraffic"]["traffic_situation_id"] = json!("2");
    let mut end_receiver = zeromq::PullSocket::new();
    let end_endpoint = end_receiver
        .bind("tcp://127.0.0.1:0")
        .await
        .unwrap()
        .to_string();
    let end_udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let end_app = make_app(
        pool.clone(),
        &end_endpoint,
        Some(end_udp.local_addr().unwrap()),
    );
    sqlx::raw_sql("CREATE TRIGGER reject_delete BEFORE DELETE ON device_activity FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='test end rollback'").execute(&pool).await.unwrap();
    let result = post(
        end_app.clone(),
        "/DeviceTraffic/endARoute",
        end.clone(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 3);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_activity WHERE device_id=42")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3);
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM device_traffic WHERE traffic_type_id=4")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM route_trigger_audit")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), end_udp.recv(&mut bytes))
            .await
            .is_err()
    );
    sqlx::raw_sql("DROP TRIGGER reject_delete")
        .execute(&pool)
        .await
        .unwrap();
    let result = post(
        end_app,
        "/school_bus_cloud/DeviceTraffic/endARoute",
        end.clone(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
    let situation: i32 = sqlx::query_scalar(
        "SELECT traffic_situation_id FROM device_traffic WHERE traffic_type_id=4",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(situation, 2);
    let assigned: String =
        sqlx::query_scalar("SELECT assigned_bus FROM device_traffic WHERE traffic_type_id=4")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(assigned, "REQUEST-BUS");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_activity WHERE device_id=42")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_activity WHERE device_id=43")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let n = tokio::time::timeout(Duration::from_secs(2), end_udp.recv(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&bytes[n - 21..n - 19], &402u16.to_be_bytes());
    assert_eq!(&bytes[n - 11..n - 9], &402u16.to_be_bytes());
    for _ in 0..2 {
        let msg = tokio::time::timeout(Duration::from_secs(2), end_receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let value: Value = serde_json::from_str(&String::try_from(msg).unwrap()).unwrap();
        assert_eq!(value["content"]["typeTrafficId"], 4);
        assert_eq!(value["content"]["traffic_label"], "ROUTE_MAJOR_TRAFFIC");
    }
    // Ending with no activity is allowed; unavailable dashboards still report saved data.
    let timeout_app = make_app(
        pool.clone(),
        "tcp://127.0.0.1:1",
        Some(end_udp.local_addr().unwrap()),
    );
    let result = post(
        timeout_app,
        "/DeviceTraffic/endARoute",
        end,
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 3);
    assert_eq!(result["ReaxiumResponse"]["object"]["database_saved"], true);
    assert_eq!(result["ReaxiumResponse"]["object"]["activities_deleted"], 0);
    assert!(result["ReaxiumResponse"]["object"]["device_activity_id"].is_null());
    assert_ne!(
        result["ReaxiumResponse"]["object"]["broadcast_status"],
        "sent"
    );
    // Disabling both integrations must skip broadcast data queries and network IO.
    sqlx::query("DELETE FROM traffic_type")
        .execute(&pool)
        .await
        .unwrap();
    let disabled = router(Metrics::new().unwrap())
        .layer(Extension(pool.clone()))
        .layer(Extension(Arc::new(Config {
            username: "test".into(),
            password: "test-password".into(),
            previous_password: String::new(),
            broadcast_enabled: false,
            broadcast_endpoint: "tcp://127.0.0.1:1".into(),
            broadcast_timeout: Duration::from_millis(300),
        })))
        .layer(Extension(Arc::new(VisionConfig {
            enabled: false,
            destination: None,
            timezone: chrono_tz::America::New_York,
        })));
    for path in ["/DeviceTraffic/beginARoute", "/DeviceTraffic/endARoute"] {
        let mut input = sample();
        input["ReaxiumParameters"]["DeviceTraffic"]["traffic_situation_id"] = json!(1);
        let response = post(disabled.clone(), path, input, Some("test-password")).await;
        assert_eq!(response["ReaxiumResponse"]["code"], 0, "{response}");
    }
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM device_activity WHERE device_id=42")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn end_route_requires_authentication_and_situation_on_both_paths() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = make_app(pool, "tcp://127.0.0.1:1", None);
    for path in [
        "/DeviceTraffic/endARoute",
        "/school_bus_cloud/DeviceTraffic/endARoute",
    ] {
        let result = post(app.clone(), path, sample(), None).await;
        assert_eq!(
            result["ReaxiumResponse"]["message"],
            "Invalid Authentication"
        );
        for situation in [
            None,
            Some(json!(0)),
            Some(json!(-1)),
            Some(json!("invalid")),
        ] {
            let mut body = sample();
            if let Some(situation) = situation {
                body["ReaxiumParameters"]["DeviceTraffic"]["traffic_situation_id"] = situation;
            }
            let result = post(app.clone(), path, body, Some("test-password")).await;
            assert!(result["ReaxiumResponse"]["message"]
                .as_str()
                .unwrap()
                .starts_with("Invalid Parameters"));
        }
    }
}

#[test]
fn bus_info_requires_a_positive_bus_id_not_a_number_or_mysql_coercible_prefix() {
    for id in ["364", "000364"] {
        let mut input = sample()["ReaxiumParameters"]["DeviceTraffic"].clone();
        input["bus_info"] = json!(id);
        assert!(serde_json::from_value::<BeginRoute>(input).unwrap().valid());
    }
    for id in ["", "0", "-1", "364BUS", "364.5", "ZZQA1014B"] {
        let mut input = sample()["ReaxiumParameters"]["DeviceTraffic"].clone();
        input["bus_info"] = json!(id);
        assert!(!serde_json::from_value::<BeginRoute>(input).unwrap().valid());
    }
}
