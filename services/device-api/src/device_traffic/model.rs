use crate::device_status::model::number;
use chrono::NaiveDateTime;
use serde::Deserialize;

#[derive(Deserialize)]
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
