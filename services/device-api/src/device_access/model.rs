use chrono::{Datelike, NaiveDateTime};
use serde::{de::Error, Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};

fn scalar<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<Value>::deserialize(d)?.map(|v| match v {
        Value::String(s) => Ok(s), Value::Number(n) => Ok(n.to_string()),
        _ => Err(D::Error::custom("expected scalar")),
    }).transpose()
}
fn integer<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i32>, D::Error> {
    scalar(d)?.map(|s| s.parse().map_err(D::Error::custom)).transpose()
}
fn coordinate<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    scalar(d)?.map(|s| s.parse().map_err(D::Error::custom)).transpose()
}

// Java accepts nullable fields at the HTTP boundary; missing required references
// fail during background processing, not through the stricter PHP access model.
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Access {
    #[serde(deserialize_with="integer")] pub user_id: Option<i32>,
    #[serde(deserialize_with="integer")] pub device_id: Option<i32>,
    #[serde(deserialize_with="integer")] pub access_type: Option<i32>,
    #[serde(deserialize_with="integer")] pub traffic_type: Option<i32>,
    #[serde(deserialize_with="integer")] pub route_id: Option<i32>,
    #[serde(deserialize_with="integer")] pub stop_id: Option<i32>,
    #[serde(deserialize_with="integer")] pub stop_id_by_distance: Option<i32>,
    #[serde(deserialize_with="integer")] pub access_situation_id: Option<i32>,
    #[serde(deserialize_with="scalar")] pub traffic_info: Option<String>,
    #[serde(deserialize_with="scalar")] pub trace_id: Option<String>,
    #[serde(deserialize_with="scalar")] pub bus_info: Option<String>,
    #[serde(rename="accessDate", deserialize_with="scalar")] pub access_date: Option<String>,
    #[serde(deserialize_with="coordinate")] pub latitude: Option<f64>,
    #[serde(deserialize_with="coordinate")] pub longitude: Option<f64>,
    #[serde(deserialize_with="integer")] pub driver_id: Option<i32>,
    #[serde(deserialize_with="integer")] pub business_id: Option<i32>,
}
impl Access {
    pub fn date(&self) -> Option<NaiveDateTime> {
        let millis: i64 = self.access_date.as_ref()?.parse().ok()?;
        let date = chrono::DateTime::from_timestamp_millis(millis)?
            .with_timezone(&chrono_tz::America::New_York).naive_local();
        (1000..=9999).contains(&date.year()).then(|| date.with_nanosecond(0)).flatten()
    }
}
use chrono::Timelike;

pub(super) fn parse(body: &Value) -> Result<Vec<Option<Access>>, String> {
    let events = body["ReaxiumParameters"]["BulkUserAccess"].as_array()
        .filter(|a| !a.is_empty()).ok_or("nonempty BulkUserAccess required")?;
    events.iter().map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string())).collect()
}
pub(super) fn message(events: &[Option<Access>]) -> Value {
    // Lombok isABulk()/setABulk() is the Jackson bean property "abulk".
    json!({"ReaxiumParameters":{"BulkUserAccess":events},"abulk":true})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn java_envelope_validation_and_scalar_coercion() {
        for value in [json!({}), json!({"ReaxiumParameters":{"BulkUserAccess":[]}}), json!({"ReaxiumParameters":{"BulkUserAccess":{}}})] { assert!(parse(&value).is_err()); }
        let events = parse(&json!({"ReaxiumParameters":{"BulkUserAccess":[{},null,{"user_id":"6","device_id":"42","accessDate":1792584000999_i64,"trace_id":99}]}})).unwrap();
        assert_eq!(events.len(), 3);
        assert!(events[1].is_none());
        let event = events[2].as_ref().unwrap();
        assert_eq!(event.user_id, Some(6));
        assert_eq!(event.trace_id.as_deref(), Some("99"));
        assert_eq!(event.date().unwrap().to_string(), "2026-10-21 08:00:00");
        assert_eq!(message(&events)["abulk"], true);
        assert!(parse(&json!({"ReaxiumParameters":{"BulkUserAccess":[{"user_id":"bad"}]}})).is_err());
    }
}
