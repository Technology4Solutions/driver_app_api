use chrono::NaiveDateTime;
use serde::{de::Error, Deserialize, Deserializer};
use std::{fmt::Display, str::FromStr};

// Android clients also send numeric values as JSON strings.
pub(crate) fn number<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: Display,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let text = match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        _ => return Err(D::Error::custom("expected a number or numeric string")),
    };
    text.parse().map_err(D::Error::custom)
}

#[derive(Deserialize)]
pub struct Update {
    pub device_imei: String,
    #[serde(deserialize_with = "number")]
    pub user_id: i64,
    #[serde(deserialize_with = "number")]
    pub business_id: i64,
    pub status_date: String,
    pub device_status: Status,
    pub device_location: Location,
    pub device_activity: Option<Activity>,
    pub trace_id: Option<String>,
    #[serde(default = "one", deserialize_with = "number")]
    pub traffic_type_id: i64,
}

fn one() -> i64 {
    1
}

impl Update {
    pub fn valid(&self) -> bool {
        let l = &self.device_location;
        let s = &self.device_status;
        !self.device_imei.trim().is_empty()
            && self.user_id >= 0
            && self.business_id > 0
            && NaiveDateTime::parse_from_str(&self.status_date, "%Y-%m-%d %H:%M:%S").is_ok()
            && self.status_date.len() == 19
            && (-90.0..=90.0).contains(&l.latitude)
            && (-180.0..=180.0).contains(&l.longitude)
            && [
                l.speed,
                l.bearing,
                l.accuracy,
                l.lag_time,
                s.temperature,
                s.battery_power_level,
            ]
            .iter()
            .all(|n| n.is_finite())
            && (0..=1).contains(&s.is_charging)
            && self
                .device_activity
                .as_ref()
                .is_none_or(|a| a.route_id >= 0 && a.users_in_track.iter().all(|id| *id >= 0))
    }
}

#[derive(Deserialize)]
pub struct Status {
    #[serde(deserialize_with = "number")]
    pub temperature: f64,
    #[serde(deserialize_with = "number")]
    pub battery_power_level: f64,
    #[serde(default, deserialize_with = "charging")]
    pub is_charging: i8,
    pub last_sync: Option<String>,
    pub installed_app_version: String,
    pub bus_info: Option<String>,
}

#[derive(Deserialize)]
pub struct Location {
    #[serde(deserialize_with = "number")]
    pub latitude: f64,
    #[serde(deserialize_with = "number")]
    pub longitude: f64,
    #[serde(deserialize_with = "number")]
    pub speed: f64,
    #[serde(deserialize_with = "number")]
    pub bearing: f64,
    #[serde(deserialize_with = "number")]
    pub accuracy: f64,
    #[serde(deserialize_with = "number")]
    pub lag_time: f64,
}

#[derive(Deserialize)]
pub struct Activity {
    #[serde(deserialize_with = "number")]
    pub route_id: i64,
    #[serde(deserialize_with = "ids")]
    pub users_in_track: Vec<i64>,
}

fn ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<i64>, D::Error> {
    #[derive(Deserialize)]
    struct Id(#[serde(deserialize_with = "number")] i64);
    Ok(Vec::<Id>::deserialize(d)?
        .into_iter()
        .map(|id| id.0)
        .collect())
}

fn charging<'de, D: Deserializer<'de>>(d: D) -> Result<i8, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    match value {
        serde_json::Value::Null => Ok(0),
        serde_json::Value::Bool(v) => Ok(i8::from(v)),
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(|v| i8::from(v != 0))
            .ok_or_else(|| D::Error::custom("expected integer or boolean")),
        _ => Err(D::Error::custom("expected integer or boolean")),
    }
}
