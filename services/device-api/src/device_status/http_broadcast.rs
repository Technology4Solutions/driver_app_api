use super::broadcast::row_json;
use serde_json::{json, Value};
use sqlx::MySqlPool;
use std::{env, time::Duration};
type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone)]
pub(super) struct Broadcaster {
    pub enabled: bool,
    client: reqwest::Client,
    url: String,
    username: String,
    password: String,
}
impl Broadcaster {
    pub fn from_env() -> Result<Self, Error> {
        let local = env::var("APP_ENV").unwrap_or_else(|_| "local".into()) == "local";
        let enabled = env::var("BROADCAST_ENABLED")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| (!local).to_string())
            .parse()?;
        let url = env::var("DEVICES_API_EXPORT_DEVICE_STATUS_URL").unwrap_or_default();
        let username = env::var("DEVICE_STATUS_BROADCAST_USERNAME").unwrap_or_default();
        let password = env::var("DEVICE_STATUS_BROADCAST_PASSWORD").unwrap_or_default();
        if enabled && (url.is_empty() || username.is_empty() || password.is_empty()) {
            return Err("enabled status broadcasts require URL, username and password".into());
        }
        if enabled {
            reqwest::Url::parse(&url)?;
        }
        Ok(Self {
            enabled,
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(3))
                .user_agent("DeviceApiWebSocketClient/1.0")
                .build()?,
            url,
            username,
            password,
        })
    }
    pub async fn send(&self, content: Value) {
        let result = self
            .client
            .post(&self.url)
            .basic_auth(&self.username, Some(&self.password))
            .json(&json!({"ReaxiumParameters":{"WebSocketPusher":{"content":content}}}))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status);
        if let Err(error) = result {
            tracing::error!(%error, "Error dispatching WebSocket message asynchronously");
        }
    }
}

async fn one(pool: &MySqlPool, query: &str, id: i64) -> Result<Value, sqlx::Error> {
    sqlx::query(query)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .map(row_json)
        .transpose()
        .map(|value| value.unwrap_or(Value::Null))
}
fn as_id(value: &Value) -> i64 {
    value.as_i64().unwrap_or(0)
}
fn text(value: &Value) -> Value {
    if value.is_null() || value.is_string() {
        value.clone()
    } else {
        json!(value.to_string())
    }
}

// Build the Java DeviceLocationDTO shape, rather than the legacy ZeroMQ payload.
pub(super) async fn payload(pool: &MySqlPool, device_id: i64) -> Result<Value, sqlx::Error> {
    let serial: String =
        sqlx::query_scalar("SELECT device_serial FROM reaxium_device WHERE device_id=?")
            .bind(device_id)
            .fetch_one(pool)
            .await?;
    let stored = one(pool, "SELECT s.temperature, s.battery_power_level, s.is_charging, s.bus_info, s.installed_app_version, DATE_FORMAT(s.last_status_update, '%Y-%m-%d %H:%i:%s') AS last_status_update, DATE_FORMAT(s.last_sync, '%Y-%m-%d %H:%i:%s') AS last_sync, l.user_id, l.latitude, l.longitude, l.speed, l.accuracy, l.bearing, l.lag_time FROM device_status s JOIN device_location l ON l.device_location_id=s.device_location_id WHERE s.device_id=? LIMIT 1", device_id).await?;
    let user = one(
        pool,
        "SELECT * FROM users WHERE user_id=? LIMIT 1",
        as_id(&stored["user_id"]),
    )
    .await?;
    let driver = if user.is_null() {
        Value::Null
    } else {
        let mut fields = serde_json::Map::new();
        for name in [
            "user_id",
            "document_id",
            "first_name",
            "second_name",
            "first_last_name",
            "status_id",
            "user_type_id",
            "user_photo",
            "business_id",
            "email",
            "birthdate",
            "fingerprint",
            "rfid",
            "business_master_id",
            "gender",
        ] {
            fields.insert(name.into(), user[name].clone());
        }
        fields.insert("second_last_name".into(), Value::Null);
        for name in [
            "user_id",
            "status_id",
            "user_type_id",
            "business_id",
            "business_master_id",
        ] {
            fields.insert(name.into(), json!(as_id(&user[name])));
        }
        Value::Object(fields)
    };
    let mut activity = one(
        pool,
        "SELECT *, CAST(UNIX_TIMESTAMP(activity_date)*1000 AS SIGNED) AS activity_date_millis FROM device_activity WHERE device_id=? ORDER BY device_activity_id DESC LIMIT 1",
        device_id,
    )
    .await?;
    if !activity.is_null() {
        let route = one(
            pool,
            "SELECT * FROM routes WHERE id_route=? LIMIT 1",
            as_id(&activity["route_id"]),
        )
        .await?;
        let route = if route.is_null() {
            Value::Null
        } else {
            let stops = sqlx::query("SELECT s.* FROM stops s JOIN routes_stops_relationship r ON r.id_stop=s.id_stop WHERE r.id_route=?")
                .bind(as_id(&route["id_route"])).fetch_all(pool).await?
                .into_iter().map(row_json).collect::<Result<Vec<_>, _>>()?;
            let stops: Vec<Value> = stops.into_iter().map(|stop| json!({
                "id_stop":as_id(&stop["id_stop"]), "stop_number":stop["stop_number"],
                "stop_name":stop["stop_name"], "stop_latitude":text(&stop["stop_latitude"]),
                "stop_longitude":text(&stop["stop_longitude"]), "stop_address":stop["stop_address"],
                "status_id":as_id(&stop["status_id"]), "stop_type_id":as_id(&stop["stop_type_id"])
            })).collect();
            json!({"id_route":route["id_route"],"route_number":route["route_number"],"route_name":null,"route_address":route["route_name"],
                "route_type":route["route_type"],"start_time":route["start_time"],"end_time":route["end_time"],
                "shuttle_id":route["shuttle_id"],"overview_polyline":route["overview_polyline"],"stops":stops})
        };
        let users: Vec<i64> = activity["users_in_track"]
            .as_str()
            .unwrap_or("")
            .split(',')
            .filter_map(|id| id.trim().parse().ok())
            .collect();
        activity = json!({"device_activity_id":activity["device_activity_id"],"device_id":0,
            "route_id":if route.is_null() {json!(0)} else {route["id_route"].clone()},
            "route":route,"users_in_track":users,"activity_date":activity["activity_date_millis"]});
    }
    let status = json!({
        "device_id":0,"bus_info":stored["bus_info"],"last_sync":stored["last_sync"],"status_date":null,
        "last_status_update":stored["last_status_update"],"temperature":text(&stored["temperature"]),
        "battery_power_level":stored["battery_power_level"].as_f64().map(|v| v as i64),
        "is_charging":as_id(&stored["is_charging"]) != 0,
        "user_id":as_id(&stored["user_id"]),"latitude":text(&stored["latitude"]),"longitude":text(&stored["longitude"]),
        "speed":text(&stored["speed"]),"accuracy":text(&stored["accuracy"]),"bearing":text(&stored["bearing"]),
        "lag_time":text(&stored["lag_time"]),"installed_app_version":stored["installed_app_version"]
    });
    Ok(
        json!({"driver":driver,"device_id":device_id,"gps_serial":serial,
        "device":{"device_id":device_id,"activity":activity,"status":status,"imei":serial},
        "last_associated_traffic":3,"latitude":null,"longitude":null,"altitude":null,"bearing":null,"accuracy":null,"speed":null,"lag_time":null}),
    )
}
