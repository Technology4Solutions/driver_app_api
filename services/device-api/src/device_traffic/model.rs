use crate::device_status::model::number;
use chrono::NaiveDateTime;
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub struct BeginRoute {
    #[serde(deserialize_with = "number")]
    pub device_id: i64,
    #[serde(deserialize_with = "number")]
    pub stop_id: i64,
    #[serde(deserialize_with = "number")]
    pub route_id: i64,
    #[serde(deserialize_with = "number")]
    pub driver_id: i64,
    pub trace_id: String,
    #[serde(deserialize_with = "number")]
    pub latitude: f64,
    #[serde(deserialize_with = "number")]
    pub longitude: f64,
    pub bus_info: String,
    pub traffic_date: String,
    #[serde(default, deserialize_with = "optional_number")]
    pub traffic_situation_id: Option<i64>,
    #[serde(default, deserialize_with = "optional_number")]
    pub traffic_type_id: Option<i64>,
    pub alert_message: Option<String>,
    pub users_in_track: Option<String>,
}

impl BeginRoute {
    pub fn date(&self) -> Result<NaiveDateTime, chrono::ParseError> {
        NaiveDateTime::parse_from_str(&self.traffic_date.replace("::", ":"), "%Y-%m-%d %H:%M:%S")
    }
    pub fn valid(&self) -> bool {
        self.device_id > 0
            && self.stop_id > 0
            && self.route_id > 0
            && self.route_id <= u32::MAX as i64
            && self.driver_id > 0
            && !self.trace_id.trim().is_empty()
            && self.bus_info.trim().parse::<u64>().is_ok_and(|id| id > 0)
            && (-90.0..=90.0).contains(&self.latitude)
            && (-180.0..=180.0).contains(&self.longitude)
            && self.traffic_date.replace("::", ":").len() == 19
            && self.date().is_ok()
    }
}

fn optional_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    number(d).map(Some)
}

/// Alarm requests use server time and cannot override their type or situation
/// through the generic DeviceTraffic fields.
pub(super) fn notification_request(mut value: serde_json::Value) -> Option<BeginRoute> {
    let traffic_type: i64 = number(value.get("notification_type")?.clone()).ok()?;
    if !(1..=i32::MAX as i64).contains(&traffic_type) {
        return None;
    }
    let object = value.as_object_mut()?;
    object.insert("traffic_type_id".into(), traffic_type.into());
    object.insert("traffic_situation_id".into(), 1.into());
    object.insert(
        "traffic_date".into(),
        chrono::Utc::now()
            .with_timezone(&chrono_tz::America::New_York)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
            .into(),
    );
    // PHP accepts identifiers as JSON numbers as well as strings.
    for field in ["trace_id", "bus_info"] {
        if let Some(serde_json::Value::Number(number)) = object.get(field) {
            object.insert(field.into(), number.to_string().into());
        }
    }
    serde_json::from_value::<BeginRoute>(value)
        .ok()
        .filter(BeginRoute::valid)
}

/// PHP requires notification_type to be present but ignores its value here.
pub(super) fn fixed_notification_request(
    mut value: serde_json::Value,
    traffic_type: i64,
) -> Option<BeginRoute> {
    if value.get("notification_type")?.is_null() {
        return None;
    }
    value
        .as_object_mut()?
        .insert("notification_type".into(), traffic_type.into());
    notification_request(value)
}

/// Manual reports preserve the supplied event date and need no notification_type.
pub(super) fn manual_skipped_stop_request(mut value: serde_json::Value) -> Option<BeginRoute> {
    let object = value.as_object_mut()?;
    object.insert("traffic_type_id".into(), 27.into());
    object.insert("traffic_situation_id".into(), 2.into());
    for field in ["trace_id", "bus_info"] {
        if let Some(serde_json::Value::Number(number)) = object.get(field) {
            object.insert(field.into(), number.to_string().into());
        }
    }
    serde_json::from_value::<BeginRoute>(value)
        .ok()
        .filter(BeginRoute::valid)
}

#[cfg(test)]
mod notification_tests {
    use super::notification_request;
    use serde_json::json;

    #[test]
    fn manual_report_preserves_date_and_ignores_caller_type_and_situation() {
        let input = json!({
            "device_id": 42, "stop_id": 9, "route_id": 7, "driver_id": 1,
            "trace_id": 501, "bus_info": 2, "latitude": 27.1, "longitude": -80.2,
            "traffic_date": "2026-11-08 08::30::00", "traffic_type_id": 3,
            "traffic_situation_id": 1, "notification_type": 9
        });
        let report = super::manual_skipped_stop_request(input).unwrap();
        assert_eq!(report.traffic_type_id, Some(27));
        assert_eq!(report.traffic_situation_id, Some(2));
        assert_eq!(report.date().unwrap().to_string(), "2026-11-08 08:30:00");
    }

    #[test]
    fn notification_uses_server_date_and_forces_type_and_regular_situation() {
        let now = chrono::Utc::now()
            .with_timezone(&chrono_tz::America::New_York)
            .naive_local();
        let request = notification_request(json!({
            "notification_type": "9", "device_id": "42", "stop_id": 9,
            "route_id": 7, "driver_id": 1, "trace_id": 501,
            "latitude": "27.1975", "longitude": -80.2528, "bus_info": 2,
            "traffic_date": "invalid ignored date", "traffic_type_id": 3,
            "traffic_situation_id": 2, "alert_message": "Traffic delay"
        }))
        .unwrap();
        assert_eq!(request.traffic_type_id, Some(9));
        assert_eq!(request.traffic_situation_id, Some(1));
        assert_eq!(request.trace_id, "501");
        assert_eq!(request.bus_info, "2");
        assert!(request.date().unwrap() >= now - chrono::Duration::seconds(1));
        assert_eq!(request.alert_message.as_deref(), Some("Traffic delay"));
    }
}
