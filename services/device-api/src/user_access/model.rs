use crate::device_status::model::number;
use chrono::{Datelike, NaiveDateTime};
use serde::Deserialize;

#[derive(Clone, Deserialize)]
pub(super) struct Access {
    #[serde(deserialize_with = "number")]
    pub user_id: i64,
    #[serde(deserialize_with = "number")]
    pub device_id: i64,
    #[serde(deserialize_with = "number")]
    pub traffic_type: i64,
    #[serde(deserialize_with = "number")]
    pub access_type: i64,
    pub traffic_info: String,
    #[serde(deserialize_with = "number")]
    pub trace_id: String,
    #[serde(deserialize_with = "number")]
    pub latitude: f64,
    #[serde(deserialize_with = "number")]
    pub longitude: f64,
    #[serde(deserialize_with = "number")]
    pub bus_info: String,
    #[serde(rename = "accessDate", deserialize_with = "number")]
    pub access_date: i64,
    #[serde(deserialize_with = "number")]
    pub stop_id: i64,
    #[serde(deserialize_with = "number")]
    pub stop_id_by_distance: i64,
    #[serde(deserialize_with = "number")]
    pub route_id: i64,
    #[serde(deserialize_with = "number")]
    pub access_situation_id: i64,
}

impl Access {
    pub fn date(&self) -> Option<NaiveDateTime> {
        // PHP divides milliseconds by 1000 and stores whole seconds in New York.
        let date = chrono::DateTime::from_timestamp(self.access_date / 1000, 0)?
            .with_timezone(&chrono_tz::America::New_York)
            .naive_local();
        (1000..=9999).contains(&date.year()).then_some(date)
    }
    pub fn valid(&self) -> bool {
        [self.user_id, self.device_id, self.route_id]
            .iter()
            .all(|id| (1..=u32::MAX as i64).contains(id))
            && [
                self.traffic_type,
                self.access_type,
                self.access_situation_id,
            ]
            .iter()
            .all(|id| (1..=i32::MAX as i64).contains(id))
            && self.stop_id >= 0
            && self.stop_id_by_distance >= 0
            && self.stop_id <= u32::MAX as i64
            && self.stop_id_by_distance <= u32::MAX as i64
            && !self.trace_id.trim().is_empty()
            && self.trace_id.chars().count() <= 150
            && self.traffic_info.chars().count() <= 2500
            && self.bus_info.chars().count() <= 45
            && self.bus_info.trim().parse::<u64>().is_ok_and(|id| id > 0)
            && (-90.0..=90.0).contains(&self.latitude)
            && (-180.0..=180.0).contains(&self.longitude)
            && self.date().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn milliseconds_numeric_strings_and_new_york_dst_match_php() {
        let mut input = json!({"user_id":"1","device_id":"42","traffic_type":"1","access_type":"3","traffic_info":"Boarding","trace_id":501,"latitude":"27.1975","longitude":"-80.2528","bus_info":2,"accessDate":"1792584000123","stop_id":"9","stop_id_by_distance":"10","route_id":"7","access_situation_id":"2"});
        let request: Access = serde_json::from_value(input.clone()).unwrap();
        assert!(request.valid());
        assert_eq!(request.date().unwrap().to_string(), "2026-10-21 08:00:00");
        assert_eq!(request.trace_id, "501");
        assert_eq!(request.bus_info, "2");
        for (utc, expected) in [
            ("2026-01-21T13:00:00Z", "2026-01-21 08:00:00"),
            ("2026-07-21T12:00:00Z", "2026-07-21 08:00:00"),
        ] {
            input["accessDate"] = json!(chrono::DateTime::parse_from_rfc3339(utc)
                .unwrap()
                .timestamp_millis());
            let request: Access = serde_json::from_value(input.clone()).unwrap();
            assert_eq!(request.date().unwrap().to_string(), expected);
        }
        input["accessDate"] = json!(i64::MAX);
        assert!(!serde_json::from_value::<Access>(input).unwrap().valid());
    }
}
