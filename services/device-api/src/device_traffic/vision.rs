use super::{Action, BeginRoute, Error};
use chrono::TimeZone;
use sqlx::MySqlPool;
use std::{
    env,
    net::SocketAddr,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone)]
pub struct Config {
    pub enabled: bool,
    pub destination: Option<SocketAddr>,
    pub timezone: chrono_tz::Tz,
}
impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let enabled: bool = env::var("VISION_ENABLED")
            .unwrap_or_else(|_| "true".into())
            .parse()?;
        let destination = env::var("VISION_UDP_DESTINATION").unwrap_or_default();
        let destination = if destination.trim().is_empty() {
            None
        } else {
            Some(destination.parse()?)
        };
        let timezone = env::var("VISION_TIMEZONE")
            .unwrap_or_else(|_| "America/New_York".into())
            .parse()?;
        if !enabled {
            tracing::info!("Vision delivery disabled by VISION_ENABLED=false");
        } else if destination.is_none() {
            tracing::warn!("VISION_UDP_DESTINATION is empty; route traffic endpoints will report Vision as not configured after saving");
        }
        Ok(Self {
            enabled,
            destination,
            timezone,
        })
    }
}

static PACKET_ID: AtomicU64 = AtomicU64::new(0);

pub(super) async fn dispatch(
    pool: &MySqlPool,
    config: &Config,
    request: &BeginRoute,
    action: Action,
) -> Result<&'static str, Error> {
    if !config.enabled {
        return Ok("disabled");
    }
    let Some(destination) = config.destination else {
        return Ok("not_configured");
    };
    let serial: Option<Option<String>> = sqlx::query_scalar("SELECT COALESCE((SELECT b.gps_serial FROM bus b INNER JOIN bus_device_relationship rel ON b.bus_id=rel.bus_id WHERE rel.device_id=d.device_id LIMIT 1), d.device_serial) FROM reaxium_device d WHERE d.device_id=? AND d.status_id=1")
        .bind(request.device_id).fetch_optional(pool).await?;
    let Some(serial) = serial else {
        return Ok("skipped_inactive_device");
    };
    let serial = serial.ok_or("Active device has no Vision serial")?;
    if serial.is_empty() || serial.len() > u16::MAX as usize {
        return Err("Invalid Vision device serial".into());
    }
    let timestamp = config
        .timezone
        .from_local_datetime(&request.date()?)
        .single()
        .ok_or("Vision traffic_date is ambiguous or nonexistent in VISION_TIMEZONE")?
        .timestamp_millis();
    if timestamp < 0 {
        return Err("Vision traffic_date must be after the Unix epoch".into());
    }
    let packet_id = (PACKET_ID.fetch_add(1, Ordering::Relaxed) % 99 + 1) as u8;
    let packet = encode(&serial, request, timestamp as u64, packet_id, action);
    let bind = if destination.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = tokio::net::UdpSocket::bind(bind).await?;
    socket.send_to(&packet, destination).await?;
    tracing::info!(
        device_id = request.device_id,
        bytes = packet.len(),
        "Vision UDP packet sent (delivery is not acknowledged)"
    );
    Ok("sent")
}

// Preserve ReaxiumCloud's coordinate padding and header-length calculation for wire
// compatibility, including their nonstandard behavior. This is not a general codec encoder.
fn coordinate(value: f64) -> i32 {
    let mut digits = value.to_string().replace('.', "");
    let width = if value < 0.0 { 10 } else { 9 };
    while digits.len() < width {
        digits.push('0');
    }
    digits.truncate(width);
    digits.parse().expect("validated coordinate")
}

pub(super) fn encode(
    serial: &str,
    request: &BeginRoute,
    timestamp_ms: u64,
    packet_id: u8,
    action: Action,
) -> Vec<u8> {
    let mut avl = vec![0x8e, 1];
    avl.extend(timestamp_ms.to_be_bytes());
    avl.push(1);
    avl.extend(coordinate(request.longitude).to_be_bytes());
    avl.extend(coordinate(request.latitude).to_be_bytes());
    avl.extend([0, 0, 0, 0, 2, 0, 0]); // altitude, angle, satellites, speed
    avl.extend(action.vision_event().to_be_bytes());
    avl.extend(1u16.to_be_bytes()); // total IO elements
    avl.extend([0, 0, 0, 0, 0, 1]); // 1-byte, 2-byte and 4-byte element counts
    avl.extend(action.vision_event().to_be_bytes());
    avl.extend((request.route_id as u32).to_be_bytes());
    avl.extend([0, 0, 0, 0]); // 8-byte and variable-sized element counts
    let hex_length = avl.len() * 2;
    let length = (hex_length + hex_length.to_string().len()) as u16;
    let mut packet = length.to_be_bytes().to_vec();
    packet.extend([0xca, 0xfe, 1, packet_id]);
    packet.extend((serial.len() as u16).to_be_bytes());
    packet.extend(serial.as_bytes());
    packet.extend(avl);
    packet.push(1);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn disabled_vision_skips_database_and_udp() {
        let pool = sqlx::mysql::MySqlPoolOptions::new()
            .connect_lazy("mysql://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let receiver = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let config = Config {
            enabled: false,
            destination: Some(receiver.local_addr().unwrap()),
            timezone: chrono_tz::America::New_York,
        };
        let envelope: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../run-and-install/json/begin-route.example.json"
        ))
        .unwrap();
        let request: BeginRoute =
            serde_json::from_value(envelope["ReaxiumParameters"]["DeviceTraffic"].clone()).unwrap();
        for action in [Action::Begin, Action::End] {
            assert_eq!(
                dispatch(&pool, &config, &request, action).await.unwrap(),
                "disabled"
            );
        }
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(50),
            receiver.recv(&mut [0; 100])
        )
        .await
        .is_err());
    }
    #[test]
    fn packet_matches_original_php_encoder() {
        let envelope: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../run-and-install/json/begin-route.example.json"
        ))
        .unwrap();
        let request: BeginRoute =
            serde_json::from_value(envelope["ReaxiumParameters"]["DeviceTraffic"].clone()).unwrap();
        let packet = encode("gps-42", &request, 1792584000000, 7, Action::Begin);
        let hex: String = packet.iter().map(|b| format!("{b:02x}")).collect();
        // Generated from ReaxiumCloud's generateHeader + generateAvlData, packet ID 7.
        assert_eq!(hex,"005ecafe010700066770732d34328e01000001a15e55220001d02a65001036025800000000020000019100010000000000010191000015d60000000001");
    }
}
