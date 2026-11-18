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
    device_id: Option<String>,
    traffic_info: Option<String>,
    access_type: Option<i32>,
    trace_id: Option<String>,
    activity_id: Option<i32>,
    place_id: Option<i32>,
    traffic_type: Option<i32>,
    user_id: i32,
}
fn parse(body: &Value) -> Result<Event, Error> {
    let mut device = body["ReaxiumParameters"]["UserAccess"].clone();
    if let Some(fields) = device.as_object_mut() {
        for name in ["device_id", "traffic_info", "trace_id"] {
            if let Some(value @ Value::Number(_)) = fields.get_mut(name) {
                *value = Value::String(value.to_string());
            }
        }
        for name in [
            "activity_id",
            "place_id",
            "traffic_type",
            "access_type",
            "user_id",
        ] {
            if let Some(Value::String(s)) = fields.get(name) {
                let number: i32 = s.parse()?;
                fields.insert(name.into(), Value::from(number));
            }
        }
    }
    Ok(serde_json::from_value(device)?)
}
async fn store(pool: &MySqlPool, event: &Event) -> Result<(), sqlx::Error> {
    // Java uses the server's local processing time, not a device-supplied date.
    let datetime = chrono::Local::now()
        .naive_local()
        .format("%Y-%m-%dT%H:%M:%S%.f")
        .to_string();
    let result = sqlx::query("INSERT INTO traffic_attendance (user_id, traffic_info, device_id, access_id, activity_id, traffic_type_id, trace_id, datetime, place_id, session_uid) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0)")
        .bind(event.user_id).bind(&event.traffic_info).bind(&event.device_id)
        .bind(event.access_type).bind(event.activity_id).bind(event.traffic_type)
        .bind(&event.trace_id).bind(&datetime).bind(event.place_id).execute(pool).await?;
    tracing::info!(user_id = event.user_id, trace_id = ?event.trace_id, traffic_attendance_id = result.last_insert_id(), "traffic attendance record saved");
    Ok(())
}
pub async fn save(
    publisher: Option<Extension<queue::Publisher>>,
    Extension(config): Extension<Arc<Config>>,
    headers: HeaderMap,
    body: Result<LegacyJson, JsonRejection>,
) -> (StatusCode, Json<Value>) {
    tracing::info!("traffic attendance request received");
    if !config.authorized(&headers) {
        return (StatusCode::OK, response(3, "Invalid Authentication"));
    }
    let failure = || {
        (
            StatusCode::BAD_REQUEST,
            response(500, "Error reporting Traffic attendance record"),
        )
    };
    let body = match body {
        Ok(LegacyJson(body)) => body,
        Err(error) => {
            tracing::error!(%error, "Invalid traffic attendance JSON");
            return failure();
        }
    };
    if let Err(error) = parse(&body) {
        tracing::error!(%error, "Invalid traffic attendance parameters");
        return failure();
    }
    let Some(Extension(publisher)) = publisher else {
        tracing::error!("Traffic attendance publisher unavailable");
        return failure();
    };
    match publisher.publish(&body).await {
        Ok(()) => {
            tracing::info!("traffic attendance record queued");
            (
                StatusCode::OK,
                response(0, "traffic Attendance record queued"),
            )
        }
        Err(error) => {
            tracing::error!(%error, "Error queueing Traffic Attendance Record");
            failure()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn requires_only_user_id_and_envelope() {
        for body in [
            json!({}),
            json!({"ReaxiumParameters":null}),
            json!({"ReaxiumParameters":{"UserAccess":{}}}),
            json!({"ReaxiumParameters":{"UserAccess":{"user_id":null}}}),
        ] {
            assert!(parse(&body).is_err());
        }
        let event = parse(&json!({"ReaxiumParameters":{"UserAccess":{"user_id":0}}})).unwrap();
        assert_eq!(event.user_id, 0);
        assert!(event.device_id.is_none());
        assert!(event.access_type.is_none());
        assert!(event.traffic_type.is_none());
    }
    #[test]
    fn preserves_supplied_type_and_access_without_lookup_or_transition_rules() {
        let event = parse(&json!({"ReaxiumParameters":{"UserAccess":{
            "user_id":"6", "device_id":42,"traffic_type":"1002","access_type":"2",
            "traffic_info":"Check out","trace_id":"session-1","activity_id":"4","place_id":5,
            "datetime":"2000-01-01 00:00:00", "session_uid":99
        }}}))
        .unwrap();
        assert_eq!(event.user_id, 6);
        assert_eq!(event.device_id.as_deref(), Some("42"));
        assert_eq!(event.traffic_type, Some(1002));
        assert_eq!(event.access_type, Some(2));
        assert_eq!(event.activity_id, Some(4));
        assert_eq!(event.place_id, Some(5));
        assert_eq!(event.trace_id.as_deref(), Some("session-1"));
        assert_eq!(event.traffic_info.as_deref(), Some("Check out"));
    }
}
