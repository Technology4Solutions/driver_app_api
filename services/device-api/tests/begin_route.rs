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
#[path = "support/http_dispatcher.rs"]
mod http_dispatcher;

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
    let app = make_app(pool, "http://127.0.0.1:1", None);
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
async fn legacy_volley_duplicate_json_headers_reach_payload_validation() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = make_app(pool, "http://127.0.0.1:1", None);
    // Invalid latitude ensures successful JSON decoding stops before DB access.
    let mut input = sample();
    input["ReaxiumParameters"]["DeviceTraffic"]["latitude"] = json!(91);
    for path in [
        "/DeviceTraffic/beginARoute",
        "/school_bus_cloud/DeviceTraffic/beginARoute",
    ] {
        for content_types in [
            vec!["application/json; charset=UTF-8"],
            vec![
                "application/json; charset=UTF-8",
                "application/json; charset=utf-8",
            ],
            vec!["application/json; charset=UTF-8, application/json; charset=utf-8"],
        ] {
            let mut request = Request::post(path).header(
                "authorization",
                format!("Basic {}", STANDARD.encode("test:test-password")),
            );
            for content_type in content_types {
                request = request.header("content-type", content_type);
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::from(input.to_string())).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), 200);
            let result: Value =
                serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                    .unwrap();
            assert!(result["ReaxiumResponse"]["message"]
                .as_str()
                .unwrap()
                .starts_with("Invalid Parameters"));
        }
    }
}

#[tokio::test]
async fn legacy_json_header_compatibility_preserves_auth_parser_and_size_limit() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = make_app(pool, "http://127.0.0.1:1", None);
    for (content_type, body, authenticated, expected) in [
        (
            "application/json, application/json",
            "{}".to_owned(),
            false,
            "Invalid Authentication",
        ),
        (
            "application/json, application/json",
            "{".to_owned(),
            true,
            "Invalid Json Object",
        ),
        (
            "application/json, text/plain",
            "{}".to_owned(),
            true,
            "Invalid Json Object",
        ),
        ("text/plain", "{}".to_owned(), true, "Invalid Json Object"),
        (
            "application/json, application/json",
            " ".repeat(2 * 1024 * 1024 + 1),
            true,
            "Invalid Json Object",
        ),
    ] {
        let mut request =
            Request::post("/DeviceTraffic/beginARoute").header("content-type", content_type);
        if authenticated {
            request = request.header(
                "authorization",
                format!("Basic {}", STANDARD.encode("test:test-password")),
            );
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let result: Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(result["ReaxiumResponse"]["message"], expected);
    }
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
    let mut dispatcher = http_dispatcher::Receiver::new();
    let endpoint = dispatcher
        .bind("http://127.0.0.1:0")
        .await
        .unwrap()
        .to_string();
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
        let message = tokio::time::timeout(Duration::from_secs(2), dispatcher.recv())
            .await
            .unwrap()
            .unwrap();
        let value: Value = serde_json::from_str(&message).unwrap();
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
    let mut end_receiver = http_dispatcher::Receiver::new();
    let end_endpoint = end_receiver
        .bind("http://127.0.0.1:0")
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
        let value: Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(value["content"]["typeTrafficId"], 4);
        assert_eq!(value["content"]["traffic_label"], "ROUTE_MAJOR_TRAFFIC");
    }
    // Ending with no activity is allowed; unavailable dashboards still report saved data.
    let timeout_app = make_app(
        pool.clone(),
        "http://127.0.0.1:1",
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
            broadcast_endpoint: "http://127.0.0.1:1".into(),
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

    // Interruptions preserve boarded metadata and leave already dropped-off
    // passengers and other traces untouched.
    sqlx::raw_sql(include_str!("fixtures/interrupt_route.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let mut input = sample();
    input["ReaxiumParameters"]["DeviceTraffic"]["trace_id"] = json!("interrupt-test");
    input["ReaxiumParameters"]["DeviceTraffic"]["traffic_situation_id"] = json!("2");
    for path in [
        "/DeviceTraffic/interruptARoute",
        "/school_bus_cloud/DeviceTraffic/interruptARoute",
    ] {
        let result = post(disabled.clone(), path, input.clone(), Some("test-password")).await;
        assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
    }
    let rows = sqlx::query("SELECT * FROM traffic WHERE access_situation_id=20")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    let dropoff = &rows[0];
    assert_eq!(dropoff.get::<i64, _>("user_id"), 10);
    assert_eq!(dropoff.get::<i64, _>("access_id"), 100);
    assert_eq!(dropoff.get::<i64, _>("id_stop"), 9);
    assert_eq!(dropoff.get::<i64, _>("id_stop_by_distance"), 9);
    assert_eq!(dropoff.get::<String, _>("stop_number"), "STOP9");
    assert_eq!(dropoff.get::<String, _>("session_uid"), "session-10");
    assert_eq!(dropoff.get::<String, _>("assigned_bus"), "ORIGINAL-BUS");
    assert_eq!(
        dropoff.get::<f64, _>("latitude"),
        input["ReaxiumParameters"]["DeviceTraffic"]["latitude"]
            .as_f64()
            .unwrap()
    );
    assert!(dropoff
        .get::<Option<chrono::NaiveDateTime>, _>("datetime")
        .is_some());
    let types: Vec<i32> = sqlx::query_scalar(
        "SELECT traffic_type_id FROM device_traffic WHERE trace_id='interrupt-test'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(types, vec![14, 14]);
    let activities: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM device_activity WHERE device_id=42")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(activities, 0);

    // A failing passenger insert rolls back the route event, trigger writes,
    // and device activity deletion as well.
    sqlx::raw_sql("INSERT INTO device_activity (device_id,route_id) VALUES (42,7); DELETE FROM traffic WHERE access_situation_id=20; CREATE TRIGGER reject_dropoff BEFORE INSERT ON traffic FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='test dropoff failure';")
        .execute(&pool).await.unwrap();
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM route_trigger_audit")
        .fetch_one(&pool)
        .await
        .unwrap();
    let result = post(
        disabled.clone(),
        "/DeviceTraffic/interruptARoute",
        input.clone(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 3);
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM route_trigger_audit")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    let activities: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM device_activity WHERE device_id=42")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(activities, 1);
    sqlx::raw_sql("DROP TRIGGER reject_dropoff")
        .execute(&pool)
        .await
        .unwrap();

    // Older traffic schemas may omit any of the denormalized snapshots.
    // Exercise a missing stop_number alone, then all optional metadata absent.
    for alteration in [
        "ALTER TABLE traffic DROP COLUMN stop_number",
        "ALTER TABLE traffic DROP COLUMN stop_name, DROP COLUMN session_uid, DROP COLUMN assigned_bus",
    ] {
        sqlx::raw_sql(alteration).execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM traffic WHERE access_situation_id=20").execute(&pool).await.unwrap();
        let result = post(disabled.clone(), "/DeviceTraffic/interruptARoute", input.clone(), Some("test-password")).await;
        assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
        let passengers: Vec<i64> = sqlx::query_scalar("SELECT user_id FROM traffic WHERE access_situation_id=20")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(passengers, vec![10]);
    }
    check_send_traffic(&pool, disabled).await;
}

#[tokio::test]
async fn finishing_routes_require_authentication_and_situation_on_both_paths() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = make_app(pool, "http://127.0.0.1:1", None);
    for path in [
        "/DeviceTraffic/endARoute",
        "/school_bus_cloud/DeviceTraffic/endARoute",
        "/DeviceTraffic/interruptARoute",
        "/school_bus_cloud/DeviceTraffic/interruptARoute",
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

#[tokio::test]
async fn send_traffic_authenticates_and_validates_types_on_both_paths() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = make_app(pool, "http://127.0.0.1:1", None);
    for path in [
        "/DeviceTraffic/sendTraffic",
        "/school_bus_cloud/DeviceTraffic/sendTraffic",
    ] {
        assert_eq!(
            post(app.clone(), path, sample(), None).await["ReaxiumResponse"]["message"],
            "Invalid Authentication"
        );
        for value in [
            Value::Null,
            json!(0),
            json!(-1),
            json!("bad"),
            json!(1.5),
            json!(2147483648_i64),
        ] {
            let mut input = sample();
            if !value.is_null() {
                input["ReaxiumParameters"]["DeviceTraffic"]["traffic_type_id"] = value;
            }
            let response = post(app.clone(), path, input, Some("test-password")).await;
            assert!(response["ReaxiumResponse"]["message"]
                .as_str()
                .unwrap()
                .starts_with("Invalid Parameters"));
        }
        let mut input = sample();
        input["ReaxiumParameters"]["DeviceTraffic"]["traffic_type_id"] = json!(15);
        input["ReaxiumParameters"]["DeviceTraffic"]["traffic_situation_id"] = json!(0);
        assert!(
            post(app.clone(), path, input, Some("test-password")).await["ReaxiumResponse"]
                ["message"]
                .as_str()
                .unwrap()
                .starts_with("Invalid Parameters")
        );
    }
}

async fn check_send_traffic(pool: &MySqlPool, disabled: Router) {
    sqlx::raw_sql("DROP TABLE IF EXISTS alerts; CREATE TABLE alerts (alert_name VARCHAR(100), alert_message TEXT, trace_id VARCHAR(100), traffic_type_id BIGINT, alert_date DATETIME, bus_id BIGINT, id_route BIGINT, device_traffic_id BIGINT UNSIGNED, session_id INT NOT NULL) ENGINE=InnoDB; DELETE FROM device_activity WHERE device_id=42;")
        .execute(pool).await.unwrap();
    let passenger_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM traffic")
        .fetch_one(pool)
        .await
        .unwrap();
    for (id, expected_activity) in [
        (3, 1),
        (15, 1),
        (16, 1),
        (9, 1),
        (17, 1),
        (22, 1),
        (4, 0),
        (3, 1),
        (14, 0),
    ] {
        let mut input = sample();
        input["ReaxiumParameters"]["DeviceTraffic"]["traffic_type_id"] = json!(id.to_string());
        input["ReaxiumParameters"]["DeviceTraffic"]["alert_message"] = json!("Test delay");
        let result = post(
            disabled.clone(),
            "/DeviceTraffic/sendTraffic",
            input,
            Some("test-password"),
        )
        .await;
        assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
        let row = sqlx::query("SELECT traffic_type_id,traffic_situation_id FROM device_traffic ORDER BY device_traffic_id DESC LIMIT 1").fetch_one(pool).await.unwrap();
        assert_eq!(row.get::<i32, _>("traffic_type_id"), id);
        assert_eq!(row.get::<i32, _>("traffic_situation_id"), 1);
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM device_activity WHERE device_id=42")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(count, expected_activity);
    }
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM traffic")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        after, passenger_count,
        "sendTraffic type 14 must not auto-drop passengers"
    );
    let names: Vec<String> =
        sqlx::query_scalar("SELECT alert_name FROM alerts ORDER BY traffic_type_id")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(
        names,
        vec!["TRAFFIC DELAY ALERT", "STOP DELAY", "LATE ATTENDANT ALERT"]
    );

    // Stop events publish minor traffic and succeed without a Vision destination.
    sqlx::query("INSERT INTO traffic_type VALUES (15,'STOP ARRIVE')")
        .execute(pool)
        .await
        .unwrap();
    let mut dispatcher = http_dispatcher::Receiver::new();
    let endpoint = dispatcher
        .bind("http://127.0.0.1:0")
        .await
        .unwrap()
        .to_string();
    let app = make_app(pool.clone(), &endpoint, None);
    let mut input = sample();
    input["ReaxiumParameters"]["DeviceTraffic"]["traffic_type_id"] = json!(15);
    input["ReaxiumParameters"]["DeviceTraffic"]["traffic_situation_id"] = json!(2);
    let result = post(
        app,
        "/school_bus_cloud/DeviceTraffic/sendTraffic",
        input.clone(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
    let message = tokio::time::timeout(Duration::from_secs(2), dispatcher.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(message.contains("ROUTE_MINOR_TRAFFIC"));
    let situation: i32 = sqlx::query_scalar(
        "SELECT traffic_situation_id FROM device_traffic ORDER BY device_traffic_id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(situation, 2);

    sqlx::raw_sql("CREATE TRIGGER reject_alert BEFORE INSERT ON alerts FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='test alert failure'").execute(pool).await.unwrap();
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_traffic")
        .fetch_one(pool)
        .await
        .unwrap();
    input["ReaxiumParameters"]["DeviceTraffic"]["traffic_type_id"] = json!(9);
    input["ReaxiumParameters"]["DeviceTraffic"]["alert_message"] = json!("Test delay");
    assert_eq!(
        post(
            disabled,
            "/DeviceTraffic/sendTraffic",
            input,
            Some("test-password")
        )
        .await["ReaxiumResponse"]["code"],
        3
    );
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_traffic")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    sqlx::raw_sql("DROP TRIGGER reject_alert")
        .execute(pool)
        .await
        .unwrap();
}

fn notification_sample() -> Value {
    let mut request = sample()["ReaxiumParameters"]["DeviceTraffic"].clone();
    request.as_object_mut().unwrap().remove("traffic_date");
    request["notification_type"] = json!(9);
    json!({"ReaxiumParameters": {"Notification": request}})
}

#[tokio::test]
async fn notifications_require_authentication_and_validate_the_notification_namespace() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = make_app(pool, "http://127.0.0.1:1", None);
    for path in [
        "/Alarm/sendNotification",
        "/school_bus_cloud/Alarm/sendNotification",
    ] {
        assert_eq!(
            post(app.clone(), path, notification_sample(), None).await["ReaxiumResponse"]
                ["message"],
            "Invalid Authentication"
        );
        assert!(
            post(app.clone(), path, sample(), Some("test-password")).await["ReaxiumResponse"]
                ["message"]
                .as_str()
                .unwrap()
                .starts_with("Invalid Parameters")
        );
        for invalid in [
            Value::Null,
            json!(0),
            json!(-1),
            json!(1.5),
            json!("bad"),
            json!(2147483648_i64),
        ] {
            let mut input = notification_sample();
            input["ReaxiumParameters"]["Notification"]["notification_type"] = invalid;
            assert!(
                post(app.clone(), path, input, Some("test-password")).await["ReaxiumResponse"]
                    ["message"]
                    .as_str()
                    .unwrap()
                    .starts_with("Invalid Parameters")
            );
        }
    }
}

async fn check_notifications(pool: &MySqlPool, disabled: Router) {
    sqlx::query("INSERT INTO bus_routes_relationship VALUES (1,7,'2026-01-01 00:00:00')")
        .execute(pool)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alerts")
        .fetch_one(pool)
        .await
        .unwrap();
    let mut expected = count;
    for (id, message, creates_alert) in [
        (9, Some("Traffic delay"), true),
        (9, None, false),
        (17, Some("Stop delay"), true),
        (22, Some("Attendant late"), true),
        (8, Some("Emergency"), false),
        (9, Some(""), false),
        (9, Some("0"), false),
    ] {
        let mut input = notification_sample();
        let r = &mut input["ReaxiumParameters"]["Notification"];
        r["notification_type"] = json!(id.to_string());
        r["traffic_date"] = json!("2000-01-01 00:00:00");
        r["traffic_situation_id"] = json!(2);
        r["traffic_type_id"] = json!(3);
        if let Some(message) = message {
            r["alert_message"] = json!(message);
        }
        let before = chrono::Utc::now()
            .with_timezone(&chrono_tz::America::New_York)
            .naive_local()
            - chrono::Duration::seconds(1);
        let result = post(
            disabled.clone(),
            "/school_bus_cloud/Alarm/sendNotification",
            input,
            Some("test-password"),
        )
        .await;
        assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
        let row =
            sqlx::query("SELECT *,CAST(traffic_date AS CHAR) AS event_date FROM device_traffic ORDER BY device_traffic_id DESC LIMIT 1")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(row.get::<i32, _>("traffic_type_id"), id);
        assert_eq!(row.get::<i32, _>("traffic_situation_id"), 1);
        assert_eq!(row.get::<String, _>("assigned_bus"), "ROUTE-BUS");
        assert_eq!(row.get::<String, _>("bus_info"), "2");
        assert!(
            chrono::NaiveDateTime::parse_from_str(
                &row.get::<String, _>("event_date"),
                "%Y-%m-%d %H:%M:%S"
            )
            .unwrap()
                >= before
        );
        if creates_alert {
            expected += 1;
            let linked: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM alerts WHERE device_traffic_id=? AND bus_id=2 AND id_route=7 AND session_id=1",
            )
            .bind(row.get::<u64, _>("device_traffic_id"))
            .fetch_one(pool)
            .await
            .unwrap();
            assert_eq!(linked, 1);
        }
        let actual: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alerts")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(actual, expected);
    }
    // Missing, dangling and blank route assignments fall back to the reported bus.
    for setup in [
        "DELETE FROM bus_routes_relationship WHERE id_route=7",
        "INSERT INTO bus_routes_relationship VALUES (999,7,'2026-01-01 00:00:00')",
        "DELETE FROM bus_routes_relationship WHERE id_route=7; UPDATE bus SET bus_number='' WHERE bus_id=1; INSERT INTO bus_routes_relationship VALUES (1,7,'2026-01-01 00:00:00')",
    ] {
        sqlx::raw_sql(setup).execute(pool).await.unwrap();
        let result = post(disabled.clone(), "/Alarm/sendNotification", notification_sample(), Some("test-password")).await;
        assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
        let assigned: String = sqlx::query_scalar("SELECT assigned_bus FROM device_traffic ORDER BY device_traffic_id DESC LIMIT 1").fetch_one(pool).await.unwrap();
        assert_eq!(assigned, "REQUEST-BUS");
    }
    // Alarm publication uses the same enriched event and business channels.
    sqlx::query("INSERT INTO traffic_type VALUES (9,'TRAFFIC DELAY')")
        .execute(pool)
        .await
        .unwrap();
    let mut receiver = http_dispatcher::Receiver::new();
    let endpoint = receiver
        .bind("http://127.0.0.1:0")
        .await
        .unwrap()
        .to_string();
    let app = make_app(pool.clone(), &endpoint, None);
    let result = post(
        app,
        "/Alarm/sendNotification",
        notification_sample(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
    let message = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let value: Value = serde_json::from_str(&message).unwrap();
    assert_eq!(value["content"]["traffic_label"], "ALARM_TRAFFIC");
    assert_eq!(value["content"]["typeTrafficId"], 9);
    sqlx::query("INSERT INTO traffic_type VALUES (5,'BUS ARRIVING'),(18,'SKIPPED STOP'),(27,'MANUAL SKIPPED STOP')")
        .execute(pool)
        .await
        .unwrap();
    let alerts_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alerts")
        .fetch_one(pool)
        .await
        .unwrap();
    let activity_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_activity")
        .fetch_one(pool)
        .await
        .unwrap();
    for (path, traffic_type) in [
        ("/Alarm/sendNextStopNotification", 5),
        ("/school_bus_cloud/Alarm/sendNextStopNotification", 5),
        ("/Alarm/sendSkippedStopNotification", 18),
        ("/school_bus_cloud/Alarm/sendSkippedStopNotification", 18),
        ("/Alarm/sendManualSkippedStopNotification", 27),
        (
            "/school_bus_cloud/Alarm/sendManualSkippedStopNotification",
            27,
        ),
    ] {
        if matches!(traffic_type, 18 | 27) {
            sqlx::query("UPDATE bus SET bus_number='ROUTE-BUS' WHERE bus_id=1")
                .execute(pool)
                .await
                .unwrap();
        }
        let mut receiver = http_dispatcher::Receiver::new();
        let endpoint = receiver
            .bind("http://127.0.0.1:0")
            .await
            .unwrap()
            .to_string();
        let app = make_app(pool.clone(), &endpoint, None);
        let mut input = notification_sample();
        let r = &mut input["ReaxiumParameters"]["Notification"];
        r["notification_type"] = json!(9);
        r["traffic_type_id"] = json!(3);
        r["traffic_situation_id"] = json!(2);
        r["traffic_date"] = json!("2000-01-01 00:00:00");
        r["alert_message"] = json!("Must not create an alert");
        if traffic_type == 27 {
            r.as_object_mut().unwrap().remove("notification_type");
            r["traffic_situation_id"] = json!(1);
            r["traffic_date"] = json!("2026-11-08 08::30::00");
        }
        let before = chrono::Utc::now()
            .with_timezone(&chrono_tz::America::New_York)
            .naive_local()
            - chrono::Duration::seconds(1);
        let result = post(app, path, input, Some("test-password")).await;
        assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
        let row = sqlx::query("SELECT traffic_type_id,traffic_situation_id,CAST(traffic_date AS CHAR) AS event_date,assigned_bus FROM device_traffic ORDER BY device_traffic_id DESC LIMIT 1").fetch_one(pool).await.unwrap();
        assert_eq!(row.get::<i32, _>("traffic_type_id"), traffic_type);
        assert_eq!(
            row.get::<i32, _>("traffic_situation_id"),
            if traffic_type == 27 { 2 } else { 1 }
        );
        assert_eq!(
            row.get::<String, _>("assigned_bus"),
            if matches!(traffic_type, 18 | 27) {
                "ROUTE-BUS"
            } else {
                "REQUEST-BUS"
            }
        );
        if traffic_type == 27 {
            assert_eq!(row.get::<String, _>("event_date"), "2026-11-08 08:30:00");
        } else {
            assert!(
                chrono::NaiveDateTime::parse_from_str(
                    &row.get::<String, _>("event_date"),
                    "%Y-%m-%d %H:%M:%S"
                )
                .unwrap()
                    >= before
            );
        }
        let message = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let value: Value = serde_json::from_str(&message).unwrap();
        assert_eq!(
            value["content"]["traffic_label"],
            if traffic_type == 27 {
                "ALARM_TRAFFIC"
            } else {
                "ROUTE_MINOR_TRAFFIC"
            }
        );
        assert_eq!(value["content"]["typeTrafficId"], traffic_type);
    }
    let alerts_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alerts")
        .fetch_one(pool)
        .await
        .unwrap();
    let activity_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_activity")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(alerts_after, alerts_before);
    assert_eq!(activity_after, activity_before);
    // PHP skipped-stop behavior keeps assigned_bus NULL when no route assignment exists.
    sqlx::query("DELETE FROM bus_routes_relationship WHERE id_route=7")
        .execute(pool)
        .await
        .unwrap();
    let result = post(
        disabled,
        "/Alarm/sendSkippedStopNotification",
        notification_sample(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
    let assigned: Option<String> = sqlx::query_scalar(
        "SELECT assigned_bus FROM device_traffic ORDER BY device_traffic_id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(assigned, None);
}

#[tokio::test]
#[ignore = "requires disposable TEST_DATABASE_URL named device_api_migration_test"]
async fn alarm_notifications_save_optional_alerts_and_broadcast() {
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
    sqlx::raw_sql("DROP TABLE IF EXISTS alerts; CREATE TABLE alerts (alert_name VARCHAR(100), alert_message TEXT, trace_id VARCHAR(100), traffic_type_id BIGINT, alert_date DATETIME, bus_id BIGINT, id_route BIGINT, device_traffic_id BIGINT UNSIGNED, session_id INT NOT NULL) ENGINE=InnoDB;")
        .execute(&pool).await.unwrap();
    let disabled = router(Metrics::new().unwrap())
        .layer(Extension(pool.clone()))
        .layer(Extension(Arc::new(Config {
            username: "test".into(),
            password: "test-password".into(),
            previous_password: String::new(),
            broadcast_enabled: false,
            broadcast_endpoint: "http://127.0.0.1:1".into(),
            broadcast_timeout: Duration::from_millis(300),
        })))
        .layer(Extension(Arc::new(VisionConfig {
            enabled: false,
            destination: None,
            timezone: chrono_tz::America::New_York,
        })));
    check_notifications(&pool, disabled).await;
}

#[tokio::test]
async fn next_stop_notifications_require_authentication_and_notification_type() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = make_app(pool, "http://127.0.0.1:1", None);
    for path in [
        "/Alarm/sendNextStopNotification",
        "/school_bus_cloud/Alarm/sendNextStopNotification",
        "/Alarm/sendSkippedStopNotification",
        "/school_bus_cloud/Alarm/sendSkippedStopNotification",
    ] {
        assert_eq!(
            post(app.clone(), path, notification_sample(), None).await["ReaxiumResponse"]
                ["message"],
            "Invalid Authentication"
        );
        for missing in [true, false] {
            let mut input = notification_sample();
            let r = &mut input["ReaxiumParameters"]["Notification"];
            if missing {
                r.as_object_mut().unwrap().remove("notification_type");
            } else {
                r["notification_type"] = Value::Null;
            }
            assert!(
                post(app.clone(), path, input, Some("test-password")).await["ReaxiumResponse"]
                    ["message"]
                    .as_str()
                    .unwrap()
                    .starts_with("Invalid Parameters")
            );
        }
    }
}

#[tokio::test]
async fn manual_skipped_stop_requires_authentication_and_valid_event_date() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = make_app(pool, "http://127.0.0.1:1", None);
    for path in [
        "/Alarm/sendManualSkippedStopNotification",
        "/school_bus_cloud/Alarm/sendManualSkippedStopNotification",
    ] {
        assert_eq!(
            post(app.clone(), path, notification_sample(), None).await["ReaxiumResponse"]
                ["message"],
            "Invalid Authentication"
        );
        for date in [Value::Null, json!("bad-date"), json!("2026-02-30 08:30:00")] {
            let mut input = notification_sample();
            let r = &mut input["ReaxiumParameters"]["Notification"];
            r.as_object_mut().unwrap().remove("notification_type");
            if !date.is_null() {
                r["traffic_date"] = date;
            }
            assert!(
                post(app.clone(), path, input, Some("test-password")).await["ReaxiumResponse"]
                    ["message"]
                    .as_str()
                    .unwrap()
                    .starts_with("Invalid Parameters")
            );
        }
    }
}

fn merged_sample() -> Value {
    let mut input = sample();
    let r = &mut input["ReaxiumParameters"]["DeviceTraffic"];
    r.as_object_mut().unwrap().remove("route_id");
    r.as_object_mut().unwrap().remove("stop_id");
    r["route_ids"] = json!("7,8");
    r["latitude"] = json!(0);
    r["longitude"] = json!(0);
    r["traffic_type_id"] = json!(14);
    r["traffic_situation_id"] = json!(2);
    input
}

#[tokio::test]
async fn merged_routes_validate_authentication_and_comma_separated_ids() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = make_app(pool, "http://127.0.0.1:1", None);
    for path in [
        "/DeviceTraffic/beginAMergeRoute",
        "/school_bus_cloud/DeviceTraffic/beginAMergeRoute",
    ] {
        assert_eq!(
            post(app.clone(), path, merged_sample(), None).await["ReaxiumResponse"]["message"],
            "Invalid Authentication"
        );
        for ids in [
            Value::Null,
            json!(""),
            json!("7,"),
            json!([7, 8]),
            json!("7,-1"),
            json!("bad"),
        ] {
            let mut input = merged_sample();
            input["ReaxiumParameters"]["DeviceTraffic"]["route_ids"] = ids;
            assert!(
                post(app.clone(), path, input, Some("test-password")).await["ReaxiumResponse"]
                    ["message"]
                    .as_str()
                    .unwrap()
                    .starts_with("Invalid Parameters")
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires disposable TEST_DATABASE_URL named device_api_migration_test; run with --test-threads=1"]
async fn merged_route_starts_select_stops_save_each_route_and_publish() {
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
    sqlx::raw_sql("CREATE TABLE routes_stops_relationship (id_route BIGINT,id_stop BIGINT,order_stop INT,stop_ratio DOUBLE);
        UPDATE stops SET stop_latitude=0,stop_longitude=0.001 WHERE id_stop=9;
        INSERT INTO stops (id_stop,stop_name,stop_number,stop_latitude,stop_longitude) VALUES (10,'Containing stop','S10',0,0.002),(11,'Nearest outside','S11',0,0.003);
        INSERT INTO routes_stops_relationship VALUES (7,9,1,1),(7,10,2,500),(8,11,1,1);
        INSERT INTO routes (id_route,status_id,route_number,route_name,route_type) VALUES (8,1,'8','Route 8',1);
        INSERT INTO bus_routes_relationship VALUES (2,8,'2026-01-01 00:00:00');
        INSERT INTO business_routes VALUES (8,1);")
        .execute(&pool).await.unwrap();
    let mut receiver = http_dispatcher::Receiver::new();
    let endpoint = receiver
        .bind("http://127.0.0.1:0")
        .await
        .unwrap()
        .to_string();
    let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let app = make_app(pool.clone(), &endpoint, Some(udp.local_addr().unwrap()));
    let result = post(
        app.clone(),
        "/DeviceTraffic/beginAMergeRoute",
        merged_sample(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
    let rows = sqlx::query("SELECT *,CAST(traffic_date AS CHAR) AS event_date FROM device_traffic ORDER BY device_traffic_id").fetch_all(&pool).await.unwrap();
    assert_eq!(rows.len(), 2);
    for (row, route_id, stop_id, assigned_bus) in [
        (&rows[0], 7, 10, "ROUTE-BUS"),
        (&rows[1], 8, 11, "REQUEST-BUS"),
    ] {
        assert_eq!(row.get::<i64, _>("id_route"), route_id);
        assert_eq!(row.get::<i64, _>("id_stop"), stop_id);
        assert_eq!(row.get::<i32, _>("traffic_type_id"), 3);
        assert_eq!(row.get::<i32, _>("traffic_situation_id"), 1);
        assert_eq!(row.get::<String, _>("assigned_bus"), assigned_bus);
        assert_eq!(row.get::<String, _>("event_date"), "2026-10-21 08:00:00");
    }
    let active: Vec<i64> =
        sqlx::query_scalar("SELECT route_id FROM device_activity ORDER BY device_activity_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(active, [7, 8]);
    let mut published = Vec::new();
    for _ in 0..4 {
        let message = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        let value: Value = serde_json::from_str(&message).unwrap();
        assert_eq!(value["content"]["traffic_label"], "ROUTE_MAJOR_TRAFFIC");
        assert_eq!(value["content"]["typeTrafficId"], 3);
        published.push(value["content"]["id_route"].as_i64().unwrap());
    }
    published.sort();
    assert_eq!(published, [7, 7, 8, 8]);
    for route_id in [7_u32, 8] {
        let mut bytes = [0u8; 1024];
        let n = tokio::time::timeout(Duration::from_secs(2), udp.recv(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&bytes[n - 9..n - 5], &route_id.to_be_bytes());
        assert_eq!(&bytes[n - 11..n - 9], &401_u16.to_be_bytes());
    }
    // A missing second route leaves the first route saved and reports partial work.
    let mut input = merged_sample();
    input["ReaxiumParameters"]["DeviceTraffic"]["route_ids"] = json!("7,999");
    let result = post(
        app,
        "/school_bus_cloud/DeviceTraffic/beginAMergeRoute",
        input,
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 3);
    assert_eq!(
        result["ReaxiumResponse"]["object"]["routes"][0]["response"]["code"],
        0
    );
    assert_eq!(
        result["ReaxiumResponse"]["object"]["routes"][1]["route_id"],
        999
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_traffic")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 3);
    // Delivery failures must not prevent the other requested routes being saved.
    let app = make_app(pool.clone(), &endpoint, None);
    let result = post(
        app,
        "/DeviceTraffic/beginAMergeRoute",
        merged_sample(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 3);
    assert_eq!(
        result["ReaxiumResponse"]["object"]["routes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    for item in result["ReaxiumResponse"]["object"]["routes"]
        .as_array()
        .unwrap()
    {
        assert_eq!(item["response"]["object"]["database_saved"], true);
        assert_eq!(
            item["response"]["object"]["vision_status"],
            "not_configured"
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_traffic")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 5);
    check_merged_end(&pool).await;
    check_merged_interrupt(&pool).await;
}

#[tokio::test]
async fn merged_end_requires_authentication_and_a_positive_situation() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = make_app(pool, "http://127.0.0.1:1", None);
    for path in [
        "/DeviceTraffic/endAMergedRoute",
        "/school_bus_cloud/DeviceTraffic/endAMergedRoute",
        "/DeviceTraffic/interruptAMergedRoute",
        "/school_bus_cloud/DeviceTraffic/interruptAMergedRoute",
    ] {
        assert_eq!(
            post(app.clone(), path, merged_sample(), None).await["ReaxiumResponse"]["message"],
            "Invalid Authentication"
        );
        for invalid in [Value::Null, json!(0), json!(-1), json!("bad"), json!(1.5)] {
            let mut input = merged_sample();
            let object = input["ReaxiumParameters"]["DeviceTraffic"]
                .as_object_mut()
                .unwrap();
            if invalid.is_null() {
                object.remove("traffic_situation_id");
            } else {
                object.insert("traffic_situation_id".into(), invalid);
            }
            assert!(
                post(app.clone(), path, input, Some("test-password")).await["ReaxiumResponse"]
                    ["message"]
                    .as_str()
                    .unwrap()
                    .starts_with("Invalid Parameters")
            );
        }
    }
}

async fn check_merged_end(pool: &MySqlPool) {
    sqlx::query("INSERT INTO traffic_type VALUES (4,'ROUTE END')")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO device_activity (device_id,route_id) VALUES (43,999)")
        .execute(pool)
        .await
        .unwrap();
    let passengers_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM traffic")
        .fetch_one(pool)
        .await
        .unwrap();
    for path in [
        "/DeviceTraffic/endAMergedRoute",
        "/school_bus_cloud/DeviceTraffic/endAMergedRoute",
    ] {
        sqlx::query("INSERT INTO device_activity (device_id,route_id) VALUES (42,999)")
            .execute(pool)
            .await
            .unwrap();
        let mut receiver = http_dispatcher::Receiver::new();
        let endpoint = receiver
            .bind("http://127.0.0.1:0")
            .await
            .unwrap()
            .to_string();
        let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let app = make_app(pool.clone(), &endpoint, Some(udp.local_addr().unwrap()));
        let mut input = merged_sample();
        let r = &mut input["ReaxiumParameters"]["DeviceTraffic"];
        r["traffic_date"] = json!("2026-11-08 09::30::00");
        r["traffic_situation_id"] = json!("2");
        r["traffic_type_id"] = json!(3);
        let result = post(app.clone(), path, input.clone(), Some("test-password")).await;
        assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
        let rows = sqlx::query("SELECT id_route,id_stop,traffic_type_id,traffic_situation_id,CAST(traffic_date AS CHAR) AS event_date,assigned_bus FROM device_traffic ORDER BY device_traffic_id DESC LIMIT 2").fetch_all(pool).await.unwrap();
        for (row, route, stop, bus) in [
            (&rows[0], 8, 11, "REQUEST-BUS"),
            (&rows[1], 7, 10, "ROUTE-BUS"),
        ] {
            assert_eq!(row.get::<i64, _>("id_route"), route);
            assert_eq!(row.get::<i64, _>("id_stop"), stop);
            assert_eq!(row.get::<i32, _>("traffic_type_id"), 4);
            assert_eq!(row.get::<i32, _>("traffic_situation_id"), 2);
            assert_eq!(row.get::<String, _>("event_date"), "2026-11-08 09:30:00");
            assert_eq!(row.get::<String, _>("assigned_bus"), bus);
        }
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM device_activity WHERE device_id=42")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(
            remaining, 0,
            "clears all device activity, including route 999"
        );
        let other: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM device_activity WHERE device_id=43")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(other, 1);
        let mut routes = Vec::new();
        for _ in 0..4 {
            let message = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            let value: Value = serde_json::from_str(&message).unwrap();
            assert_eq!(value["content"]["typeTrafficId"], 4);
            assert_eq!(value["content"]["traffic_label"], "ROUTE_MAJOR_TRAFFIC");
            routes.push(value["content"]["id_route"].as_i64().unwrap());
        }
        routes.sort();
        assert_eq!(routes, [7, 7, 8, 8]);
        for route in [7_u32, 8] {
            let mut bytes = [0u8; 1024];
            let n = tokio::time::timeout(Duration::from_secs(2), udp.recv(&mut bytes))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&bytes[n - 9..n - 5], &route.to_be_bytes());
            assert_eq!(&bytes[n - 11..n - 9], &402_u16.to_be_bytes());
        }
        // An activity-delete failure must roll back the current route's traffic insert.
        sqlx::query("INSERT INTO device_activity (device_id,route_id) VALUES (42,7)")
            .execute(pool)
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TRIGGER reject_merged_end_delete BEFORE DELETE ON device_activity FOR EACH ROW SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='test merged end rollback'").execute(pool).await.unwrap();
        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_traffic")
            .fetch_one(pool)
            .await
            .unwrap();
        let result = post(app, path, input, Some("test-password")).await;
        assert_eq!(result["ReaxiumResponse"]["code"], 3);
        assert_eq!(
            result["ReaxiumResponse"]["object"]["routes"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_traffic")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(after, before);
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM device_activity WHERE device_id=42")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(remaining, 1);
        sqlx::raw_sql("DROP TRIGGER reject_merged_end_delete")
            .execute(pool)
            .await
            .unwrap();
    }
    let passengers_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM traffic")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        passengers_after, passengers_before,
        "end does not automatically drop off passengers"
    );
}

async fn check_merged_interrupt(pool: &MySqlPool) {
    sqlx::raw_sql(include_str!("fixtures/interrupt_route.sql"))
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO traffic_type VALUES (14,'ROUTE INTERRUPTED')")
        .execute(pool)
        .await
        .unwrap();
    // Another device/route sharing the trace is deliberately included by PHP's SQL.
    sqlx::query("INSERT INTO traffic (traffic_type_id,user_id,access_id,datetime,device_id,trace_id,id_route,bus_info,session_uid,assigned_bus) VALUES (1,13,103,'2020-01-01 08:00:00',43,'interrupt-test',8,'1','session-13','BUS-13')").execute(pool).await.unwrap();
    // Prove that passenger insertion happens after BOTH interruption events exist.
    sqlx::raw_sql("CREATE TRIGGER require_merged_events BEFORE INSERT ON traffic FOR EACH ROW BEGIN IF NEW.access_situation_id=20 AND (SELECT COUNT(*) FROM device_traffic WHERE trace_id=NEW.trace_id AND traffic_type_id=14)<2 THEN SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='passenger phase ran too early'; END IF; END").execute(pool).await.unwrap();
    for path in [
        "/DeviceTraffic/interruptAMergedRoute",
        "/school_bus_cloud/DeviceTraffic/interruptAMergedRoute",
    ] {
        let mut receiver = http_dispatcher::Receiver::new();
        let endpoint = receiver
            .bind("http://127.0.0.1:0")
            .await
            .unwrap()
            .to_string();
        let udp = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let app = make_app(pool.clone(), &endpoint, Some(udp.local_addr().unwrap()));
        let mut input = merged_sample();
        let r = &mut input["ReaxiumParameters"]["DeviceTraffic"];
        r["trace_id"] = json!("interrupt-test");
        r["traffic_date"] = json!("2026-11-08 09::15::00");
        r["traffic_situation_id"] = json!("2");
        r["traffic_type_id"] = json!(3);
        let before = chrono::Utc::now()
            .with_timezone(&chrono_tz::America::New_York)
            .naive_local()
            - chrono::Duration::seconds(1);
        let result = post(app, path, input, Some("test-password")).await;
        assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
        let rows = sqlx::query("SELECT id_route,id_stop,traffic_type_id,traffic_situation_id,CAST(traffic_date AS CHAR) AS event_date,assigned_bus FROM device_traffic ORDER BY device_traffic_id DESC LIMIT 2").fetch_all(pool).await.unwrap();
        for (row, route, stop, bus) in [
            (&rows[0], 8, 11, "REQUEST-BUS"),
            (&rows[1], 7, 10, "ROUTE-BUS"),
        ] {
            assert_eq!(row.get::<i64, _>("id_route"), route);
            assert_eq!(row.get::<i64, _>("id_stop"), stop);
            assert_eq!(row.get::<i32, _>("traffic_type_id"), 14);
            assert_eq!(row.get::<i32, _>("traffic_situation_id"), 2);
            assert_eq!(row.get::<String, _>("event_date"), "2026-11-08 09:15:00");
            assert_eq!(row.get::<String, _>("assigned_bus"), bus);
        }
        let activity: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM device_activity WHERE device_id=42")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(activity, 0);
        let other: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM device_activity WHERE device_id=43")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(other, 1);
        let drops = sqlx::query("SELECT *,CAST(datetime AS CHAR) AS drop_date FROM traffic WHERE access_situation_id=20 ORDER BY user_id").fetch_all(pool).await.unwrap();
        assert_eq!(
            drops.len(),
            2,
            "no drop-offs for already dropped passengers, other traces, or repeated requests"
        );
        for (drop, user, device, route, session, bus) in [
            (&drops[0], 10, 42, 7, "session-10", "ORIGINAL-BUS"),
            (&drops[1], 13, 43, 8, "session-13", "BUS-13"),
        ] {
            assert_eq!(drop.get::<i64, _>("user_id"), user);
            assert_eq!(drop.get::<i64, _>("device_id"), device);
            assert_eq!(drop.get::<i64, _>("id_route"), route);
            assert_eq!(
                drop.get::<i64, _>("id_stop"),
                10,
                "both passengers use FIRST route's stop"
            );
            assert_eq!(drop.get::<i64, _>("id_stop_by_distance"), 10);
            assert_eq!(drop.get::<i32, _>("traffic_type_id"), 2);
            assert_eq!(drop.get::<String, _>("session_uid"), session);
            assert_eq!(drop.get::<String, _>("assigned_bus"), bus);
            assert_eq!(drop.get::<String, _>("stop_name"), "Containing stop");
            assert_eq!(drop.get::<f64, _>("latitude"), 0.0);
            assert_eq!(
                drop.get::<String, _>("traffic_info"),
                "Automatic drop off due to route interruption"
            );
        }
        if path == "/DeviceTraffic/interruptAMergedRoute" {
            let date = chrono::NaiveDateTime::parse_from_str(
                &drops[0].get::<String, _>("drop_date"),
                "%Y-%m-%d %H:%M:%S",
            )
            .unwrap();
            assert!(date >= before, "passenger dates use server time");
        }
        for _ in 0..4 {
            let message = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            let value: Value = serde_json::from_str(&message).unwrap();
            assert_eq!(value["content"]["typeTrafficId"], 14);
            assert_eq!(value["content"]["traffic_label"], "ROUTE_MAJOR_TRAFFIC");
        }
        for route in [7_u32, 8] {
            let mut bytes = [0u8; 1024];
            let n = tokio::time::timeout(Duration::from_secs(2), udp.recv(&mut bytes))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&bytes[n - 9..n - 5], &route.to_be_bytes());
            assert_eq!(&bytes[n - 11..n - 9], &402_u16.to_be_bytes());
        }
    }
    sqlx::raw_sql("DROP TRIGGER require_merged_events")
        .execute(pool)
        .await
        .unwrap();
    // Passenger failures must be reported without claiming committed route events vanished.
    sqlx::query("INSERT INTO traffic (traffic_type_id,user_id,datetime,device_id,trace_id,id_route,bus_info) VALUES (1,20,'2020-01-01 08:00:00',42,'passenger-failure',7,'2')").execute(pool).await.unwrap();
    sqlx::raw_sql("CREATE TRIGGER reject_merged_drop BEFORE INSERT ON traffic FOR EACH ROW BEGIN IF NEW.access_situation_id=20 THEN SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='test passenger failure'; END IF; END").execute(pool).await.unwrap();
    let mut receiver = http_dispatcher::Receiver::new();
    let endpoint = receiver
        .bind("http://127.0.0.1:0")
        .await
        .unwrap()
        .to_string();
    let app = make_app(pool.clone(), &endpoint, None);
    let mut input = merged_sample();
    input["ReaxiumParameters"]["DeviceTraffic"]["trace_id"] = json!("passenger-failure");
    let result = post(
        app.clone(),
        "/DeviceTraffic/interruptAMergedRoute",
        input.clone(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 3);
    assert_eq!(
        result["ReaxiumResponse"]["object"]["routes"][0]["passenger_dropoffs"]["code"],
        3
    );
    let events:i64=sqlx::query_scalar("SELECT COUNT(*) FROM device_traffic WHERE trace_id='passenger-failure' AND traffic_type_id=14").fetch_one(pool).await.unwrap();
    assert_eq!(events, 2);
    let drops: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM traffic WHERE trace_id='passenger-failure' AND traffic_type_id=2",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(drops, 0);
    sqlx::raw_sql("DROP TRIGGER reject_merged_drop")
        .execute(pool)
        .await
        .unwrap();
    // A failed route resolution prevents the entire passenger phase.
    input["ReaxiumParameters"]["DeviceTraffic"]["route_ids"] = json!("7,999");
    let result = post(
        app.clone(),
        "/DeviceTraffic/interruptAMergedRoute",
        input.clone(),
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 3);
    let drops: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM traffic WHERE trace_id='passenger-failure' AND traffic_type_id=2",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(drops, 0);
    // Vision delivery failure still permits the passenger phase after all route saves.
    input["ReaxiumParameters"]["DeviceTraffic"]["route_ids"] = json!("7,8");
    let result = post(
        app,
        "/DeviceTraffic/interruptAMergedRoute",
        input,
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 3);
    assert_eq!(
        result["ReaxiumResponse"]["object"]["routes"][0]["passenger_dropoffs"]["code"],
        0
    );
    let drops: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM traffic WHERE trace_id='passenger-failure' AND traffic_type_id=2",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(drops, 1);
}
