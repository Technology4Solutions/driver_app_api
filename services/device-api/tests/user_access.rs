use axum::{body::Body, http::Request, Extension, Router};
use base64::{engine::general_purpose::STANDARD, Engine};
use device_api::{device_status::Config, router, telemetry::Metrics};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::{mysql::MySqlPoolOptions, MySqlPool, Row};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;
#[path = "support/http_dispatcher.rs"]
mod http_dispatcher;

fn sample() -> Value {
    json!({"ReaxiumParameters":{"UserAccess":{
        "user_id":1,"device_id":42,"traffic_type":1,"access_type":3,
        "traffic_info":"Test boarding","trace_id":"passenger-test",
        "latitude":27.1975,"longitude":-80.2528,"bus_info":"2",
        "accessDate":1792584000123_i64,"stop_id":9,"stop_id_by_distance":10,
        "route_id":7,"access_situation_id":2
    }}})
}
fn app(pool: MySqlPool, endpoint: &str, enabled: bool) -> Router {
    router(Metrics::new().unwrap())
        .layer(Extension(pool))
        .layer(Extension(Arc::new(Config {
            username: "test".into(),
            password: "test-password".into(),
            previous_password: "previous".into(),
            broadcast_enabled: enabled,
            broadcast_endpoint: endpoint.into(),
            broadcast_timeout: Duration::from_millis(300),
        })))
}
async fn post(app: Router, path: &str, input: Value, password: Option<&str>) -> Value {
    let mut request = Request::post(path).header("content-type", "application/json");
    if let Some(password) = password {
        request = request.header(
            "authorization",
            format!("Basic {}", STANDARD.encode(format!("test:{password}"))),
        );
    }
    let response = app
        .oneshot(request.body(Body::from(input.to_string())).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
#[tokio::test]
async fn access_paths_require_authentication_and_complete_valid_input() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = app(pool, "http://127.0.0.1:1", true);
    for path in [
        "/UserAccess/executeAnAccessOfAUserInABus",
        "/school_bus_cloud/UserAccess/executeAnAccessOfAUserInABus",
    ] {
        assert_eq!(
            post(app.clone(), path, sample(), None).await["ReaxiumResponse"]["message"],
            "Invalid Authentication"
        );
        for password in ["test-password", "previous"] {
            for field in [
                "user_id",
                "device_id",
                "traffic_type",
                "access_type",
                "traffic_info",
                "trace_id",
                "latitude",
                "longitude",
                "bus_info",
                "accessDate",
                "stop_id",
                "stop_id_by_distance",
                "route_id",
                "access_situation_id",
            ] {
                let mut input = sample();
                input["ReaxiumParameters"]["UserAccess"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
                assert_eq!(
                    post(app.clone(), path, input, Some(password)).await["ReaxiumResponse"]["code"],
                    13,
                    "missing {field}"
                );
            }
            for (field, value) in [
                ("latitude", json!(91)),
                ("longitude", json!(-181)),
                ("accessDate", json!("not-a-date")),
                ("traffic_type", json!(0)),
                ("bus_info", json!("BUS-2")),
                ("access_situation_id", json!(0)),
            ] {
                let mut input = sample();
                input["ReaxiumParameters"]["UserAccess"][field] = value;
                assert_eq!(
                    post(app.clone(), path, input, Some(password)).await["ReaxiumResponse"]["code"],
                    13
                );
            }
        }
    }
}

#[tokio::test]
#[ignore = "requires disposable TEST_DATABASE_URL named device_api_migration_test; run database tests serially"]
async fn passenger_boarding_and_dropoff_save_metadata_trigger_rows_and_websocket_events() {
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
    for fixture in [
        include_str!("fixtures/device_status.sql"),
        include_str!("fixtures/begin_route.sql"),
        include_str!("fixtures/user_access.sql"),
    ] {
        sqlx::raw_sql(fixture).execute(&pool).await.unwrap();
    }
    // Passenger channels follow latest device traffic, independently of passenger route.
    sqlx::query("INSERT INTO device_traffic (device_id,id_route,traffic_date) VALUES (42,7,'2026-10-21 07:00:00')").execute(&pool).await.unwrap();
    let mut receiver = http_dispatcher::Receiver::new();
    let endpoint = receiver
        .bind("http://127.0.0.1:0")
        .await
        .unwrap()
        .to_string();
    let enabled = app(pool.clone(), &endpoint, true);
    let disabled = app(pool.clone(), &endpoint, false);
    for (path, traffic_type, password) in [
        (
            "/UserAccess/executeAnAccessOfAUserInABus",
            1,
            "test-password",
        ),
        (
            "/school_bus_cloud/UserAccess/executeAnAccessOfAUserInABus",
            2,
            "previous",
        ),
    ] {
        let mut input = sample();
        input["ReaxiumParameters"]["UserAccess"]["traffic_type"] = json!(traffic_type.to_string());
        let result = post(enabled.clone(), path, input.clone(), Some(password)).await;
        assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
        let row=sqlx::query("SELECT *,CAST(datetime AS CHAR) AS event_date FROM traffic ORDER BY traffic_id DESC LIMIT 1").fetch_one(&pool).await.unwrap();
        assert_eq!(row.get::<i32, _>("traffic_type_id"), traffic_type);
        assert_eq!(
            row.get::<i32, _>("access_id"),
            100,
            "matching inactive credential is retained like PHP"
        );
        assert_eq!(row.get::<i32, _>("access_situation_id"), 2);
        assert_eq!(row.get::<i64, _>("id_stop"), 9);
        assert_eq!(row.get::<i64, _>("id_stop_by_distance"), 10);
        assert_eq!(row.get::<String, _>("event_date"), "2026-10-21 08:00:00");
        assert_eq!(row.get::<String, _>("stop_number"), "STOP10");
        assert_eq!(row.get::<String, _>("stop_name"), "Distance stop");
        assert_eq!(row.get::<String, _>("assigned_bus"), "ROUTE-BUS");
        assert_eq!(row.get::<String, _>("bus_info"), "2");
        assert_eq!(row.get::<i32, _>("session_uid"), 0);
        for _ in 0..2 {
            let message = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
                .await
                .unwrap()
                .unwrap();
            let value: Value = serde_json::from_str(&message).unwrap();
            let c = &value["content"];
            assert_eq!(c["traffic_label"], "ACCESS_TRAFFIC");
            assert_eq!(c["typeTrafficId"], traffic_type);
            assert_eq!(c["id_stop"], 10);
            assert_eq!(c["user_id"], 1);
            assert_eq!(c["trace_id"], "passenger-test");
            assert_eq!(c["datetime"], "2026-10-21 08:00:00");
            assert_eq!(c["accessSituationId"], 2);
            assert_eq!(c["passenger_name"], "Test Driver");
            assert!(c.get("password").is_none());
            assert!(c.get("rfid_code").is_none());
        }
        // Production uniqueness is enforced; a repeated event is not reported as saved.
        assert_eq!(
            post(enabled.clone(), path, input, Some(password)).await["ReaxiumResponse"]["code"],
            13
        );
    }
    let triggers: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM traffic_view")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(triggers, 2);
    let activity: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_activity")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(activity, 0);
    let device_traffic: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_traffic")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(device_traffic, 1);
    // Missing credentials fail the real NOT NULL constraint and leave no traffic/trigger rows.
    let mut input = sample();
    input["ReaxiumParameters"]["UserAccess"]["access_type"] = json!(4);
    assert_eq!(
        post(
            disabled.clone(),
            "/UserAccess/executeAnAccessOfAUserInABus",
            input,
            Some("test-password")
        )
        .await["ReaxiumResponse"]["code"],
        13
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM traffic")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
    // Fallback snapshot uses id_stop only when distance stop is nonpositive.
    let mut input = sample();
    let r = &mut input["ReaxiumParameters"]["UserAccess"];
    r["trace_id"] = json!("fallback-stop");
    r["stop_id_by_distance"] = json!(0);
    assert_eq!(
        post(
            disabled.clone(),
            "/UserAccess/executeAnAccessOfAUserInABus",
            input.clone(),
            Some("test-password")
        )
        .await["ReaxiumResponse"]["code"],
        0
    );
    let stop: String =
        sqlx::query_scalar("SELECT stop_number FROM traffic WHERE trace_id='fallback-stop'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stop, "STOP9");
    // Enrichment still INNER JOINs on the distance stop; publication failure must say saved.
    input["ReaxiumParameters"]["UserAccess"]["trace_id"] = json!("missing-distance-stop");
    let result = post(
        enabled.clone(),
        "/UserAccess/executeAnAccessOfAUserInABus",
        input,
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 13);
    assert_eq!(result["ReaxiumResponse"]["object"]["database_saved"], true);
    assert_eq!(
        result["ReaxiumResponse"]["object"]["broadcast_status"],
        "failed"
    );
    // A missing preferred distance stop does not fall back to the reported stop for snapshots.
    let mut input = sample();
    input["ReaxiumParameters"]["UserAccess"]["trace_id"] = json!("unknown-stop");
    input["ReaxiumParameters"]["UserAccess"]["stop_id_by_distance"] = json!(999);
    assert_eq!(
        post(
            disabled.clone(),
            "/UserAccess/executeAnAccessOfAUserInABus",
            input,
            Some("test-password")
        )
        .await["ReaxiumResponse"]["code"],
        0
    );
    let stop: Option<String> =
        sqlx::query_scalar("SELECT stop_number FROM traffic WHERE trace_id='unknown-stop'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stop, None);
    // No route assignment means NULL assigned_bus, despite a valid reported bus ID.
    sqlx::query("DELETE FROM bus_routes_relationship")
        .execute(&pool)
        .await
        .unwrap();
    let mut input = sample();
    input["ReaxiumParameters"]["UserAccess"]["trace_id"] = json!("no-assignment");
    assert_eq!(
        post(
            disabled.clone(),
            "/UserAccess/executeAnAccessOfAUserInABus",
            input,
            Some("test-password")
        )
        .await["ReaxiumResponse"]["code"],
        0
    );
    let bus: Option<String> =
        sqlx::query_scalar("SELECT assigned_bus FROM traffic WHERE trace_id='no-assignment'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(bus, None);
    // Older schemas omit snapshots; base event storage still works.
    sqlx::raw_sql("ALTER TABLE traffic DROP COLUMN stop_number,DROP COLUMN stop_name")
        .execute(&pool)
        .await
        .unwrap();
    let mut input = sample();
    input["ReaxiumParameters"]["UserAccess"]["trace_id"] = json!("legacy-schema");
    assert_eq!(
        post(
            disabled.clone(),
            "/UserAccess/executeAnAccessOfAUserInABus",
            input,
            Some("test-password")
        )
        .await["ReaxiumResponse"]["code"],
        0
    );
    // Boarding here supplies the passenger record used by interruption auto-dropoffs.
    let mut input = sample();
    input["ReaxiumParameters"]["UserAccess"]["trace_id"] = json!("interrupt-link");
    assert_eq!(
        post(
            disabled.clone(),
            "/UserAccess/executeAnAccessOfAUserInABus",
            input,
            Some("test-password")
        )
        .await["ReaxiumResponse"]["code"],
        0
    );
    let interrupt_app = disabled.layer(Extension(Arc::new(
        device_api::device_traffic::VisionConfig {
            enabled: false,
            destination: None,
            timezone: chrono_tz::America::New_York,
        },
    )));
    let input = json!({"ReaxiumParameters":{"DeviceTraffic":{
        "device_id":42,"driver_id":1,"stop_id":9,"route_id":7,"trace_id":"interrupt-link",
        "latitude":27.1975,"longitude":-80.2528,"bus_info":"2",
        "traffic_date":"2026-11-08 09:15:00","traffic_situation_id":2
    }}});
    let result = post(
        interrupt_app,
        "/DeviceTraffic/interruptARoute",
        input,
        Some("test-password"),
    )
    .await;
    assert_eq!(result["ReaxiumResponse"]["code"], 0, "{result}");
    let drops:i64=sqlx::query_scalar("SELECT COUNT(*) FROM traffic WHERE trace_id='interrupt-link' AND traffic_type_id=2 AND access_situation_id=20 AND user_id=1 AND access_id=100")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(drops, 1);
}

#[tokio::test]
async fn legacy_volley_headers_reach_access_validation_on_both_paths() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = app(pool, "http://127.0.0.1:1", true);
    let mut input = sample();
    // Force validation to stop before DB access after decoding the legacy body.
    input["ReaxiumParameters"]["UserAccess"]["latitude"] = json!(91);
    for path in [
        "/UserAccess/executeAnAccessOfAUserInABus",
        "/school_bus_cloud/UserAccess/executeAnAccessOfAUserInABus",
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
            assert_eq!(result["ReaxiumResponse"]["code"], 13);
            assert!(result["ReaxiumResponse"]["message"]
                .as_str()
                .unwrap()
                .starts_with("Invalid Parameters"));
        }
    }
}

#[tokio::test]
async fn legacy_access_headers_preserve_auth_json_and_body_size_checks() {
    let pool = MySqlPoolOptions::new()
        .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let app = app(pool, "http://127.0.0.1:1", true);
    for path in [
        "/UserAccess/executeAnAccessOfAUserInABus",
        "/school_bus_cloud/UserAccess/executeAnAccessOfAUserInABus",
    ] {
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
            let mut request = Request::post(path).header("content-type", content_type);
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
            assert_eq!(
                result["ReaxiumResponse"]["code"],
                if authenticated { 13 } else { 3 }
            );
        }
    }
}
