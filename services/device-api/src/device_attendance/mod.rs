pub mod queue;

use crate::{
    device_status::{response, Config},
    json_body::LegacyJson,
};
use axum::{
    extract::rejection::JsonRejection,
    http::{HeaderMap, StatusCode},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::Value;
use sqlx::MySqlPool;
use std::sync::Arc;
type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Deserialize)]
struct Event {
    device_id: String,
    traffic_attendance_date: Option<String>,
    trace_id: Option<String>,
    activity_id: Option<i32>,
    place_id: Option<i32>,
    traffic_type_id: Option<i32>,
    user_id: Option<i32>,
}
fn parse(body: &Value) -> Result<Event, Error> {
    let mut device = body["ReaxiumParameters"]["Device"].clone();
    if let Some(fields) = device.as_object_mut() {
        for name in ["device_id", "traffic_attendance_date", "trace_id"] {
            if let Some(value @ Value::Number(_)) = fields.get_mut(name) {
                *value = Value::String(value.to_string());
            }
        }
        for name in ["activity_id", "place_id", "traffic_type_id", "user_id"] {
            if let Some(Value::String(s)) = fields.get(name) {
                let number: i32 = s.parse()?;
                fields.insert(name.into(), Value::from(number));
            }
        }
    }
    Ok(serde_json::from_value(device)?)
}
async fn store(pool: &MySqlPool, event: &Event) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO device_traffic_attendance (device_id, traffic_attendance_date, traffic_type_id, activity_id, place_id, trace_id, user_id) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(&event.device_id).bind(&event.traffic_attendance_date)
        .bind(event.traffic_type_id).bind(event.activity_id).bind(event.place_id)
        .bind(&event.trace_id).bind(event.user_id).execute(pool).await?;
    Ok(())
}
pub async fn save(
    publisher: Option<Extension<queue::Publisher>>,
    Extension(config): Extension<Arc<Config>>,
    headers: HeaderMap,
    body: Result<LegacyJson, JsonRejection>,
) -> (StatusCode, Json<Value>) {
    tracing::info!("device attendance request received");
    if !config.authorized(&headers) {
        return (StatusCode::OK, response(3, "Invalid Authentication"));
    }
    let failure = || {
        (
            StatusCode::BAD_REQUEST,
            response(500, "Error reporting Device attendance record"),
        )
    };
    let body = match body {
        Ok(LegacyJson(body)) => body,
        Err(error) => {
            tracing::error!(%error, "Invalid device attendance JSON");
            return failure();
        }
    };
    if let Err(error) = parse(&body) {
        tracing::error!(%error, "Invalid device attendance parameters");
        return failure();
    }
    let Some(Extension(publisher)) = publisher else {
        tracing::error!("Device attendance publisher unavailable");
        return failure();
    };
    match publisher.publish(&body).await {
        Ok(()) => {
            tracing::info!("device attendance record queued");
            (
                StatusCode::OK,
                response(0, "Device Attendance record queued"),
            )
        }
        Err(error) => {
            tracing::error!(%error, "Error queueing Device Attendance Record");
            failure()
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn minimum_validation_matches_java() {
        for body in [
            json!({}),
            json!({"ReaxiumParameters":null}),
            json!({"ReaxiumParameters":{"Device":{}}}),
            json!({"ReaxiumParameters":{"Device":{"device_id":null}}}),
        ] {
            assert!(parse(&body).is_err());
        }
        let event = parse(&json!({"ReaxiumParameters":{"Device":{"device_id":""}}})).unwrap();
        assert_eq!(event.device_id, "");
        assert!(event.activity_id.is_none());
    }
    #[test]
    fn preserves_values_and_accepts_numeric_strings() {
        let event = parse(&json!({"ReaxiumParameters":{"Device":{
            "device_id":123,"activity_id":"4","place_id":5,"traffic_type_id":21,
            "traffic_attendance_date":"2026-11-15 10:20:30","trace_id":"session-1","user_id":"6"
        }}}))
        .unwrap();
        assert_eq!(event.device_id, "123");
        assert_eq!(event.activity_id, Some(4));
        assert_eq!(event.place_id, Some(5));
        assert_eq!(event.traffic_type_id, Some(21));
        assert_eq!(event.user_id, Some(6));
        assert_eq!(event.trace_id.as_deref(), Some("session-1"));
        assert_eq!(
            event.traffic_attendance_date.as_deref(),
            Some("2026-11-15 10:20:30")
        );
    }
}
