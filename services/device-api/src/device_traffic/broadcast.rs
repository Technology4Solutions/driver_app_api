use crate::device_status::broadcast::{channels, row_json, send};
use serde_json::json;
use sqlx::MySqlPool;

pub(super) async fn dispatch(
    pool: &MySqlPool,
    endpoint: &str,
    device_id: i64,
    traffic_id: u64,
) -> Result<&'static str, super::Error> {
    // Equivalent to GET_DEVICE_TRAFFIC_INFO_BY_ID without depending on its definer.
    // PDO returns date strings here; cast explicitly to support both TIMESTAMP and DATETIME.
    let row = sqlx::query("SELECT dt.trace_id,dt.bus_info,dt.device_id,CAST(dt.traffic_date AS CHAR) AS traffic_date,dt.traffic_type_id AS typeTrafficId,tt.traffic_type_name AS typeTrafficName,u.user_id AS driver_user_id,u.first_name AS driver_first_name,u.first_last_name AS driver_first_last_name,u.document_id AS driver_document_id,u.user_photo AS driver_photo,CAST(u.birthdate AS CHAR) AS driver_birthdate,r.id_route,r.route_number,r.route_name,r.route_type,rt.route_type_name,s.id_stop,s.stop_number,s.stop_name,s.stop_latitude,s.stop_longitude FROM device_traffic dt INNER JOIN traffic_type tt ON dt.traffic_type_id=tt.traffic_type_id INNER JOIN routes r ON dt.id_route=r.id_route INNER JOIN route_type rt ON r.route_type=rt.route_type_id INNER JOIN stops s ON dt.id_stop=s.id_stop INNER JOIN users u ON dt.user_id=u.user_id WHERE dt.device_traffic_id=?")
        .bind(traffic_id).fetch_one(pool).await?;
    let mut content = row_json(row)?;
    content["traffic_label"] = json!(traffic_label(
        content["typeTrafficId"].as_i64().unwrap_or_default()
    ));
    let channels = channels(pool, device_id).await?;
    if channels.is_empty() {
        tracing::info!(
            device_id,
            "route broadcast skipped: no interested businesses"
        );
        return Ok("skipped_no_businesses");
    }
    content["channels"] = json!(channels);
    send(endpoint, &channels, &content).await?;
    tracing::info!(
        device_id,
        channel_count = channels.len(),
        "route broadcast handed to ZeroMQ"
    );
    Ok("sent")
}

fn traffic_label(traffic_type: i64) -> Option<&'static str> {
    match traffic_type {
        3 | 4 | 14 => Some("ROUTE_MAJOR_TRAFFIC"),
        5 | 15 | 16 | 18 => Some("ROUTE_MINOR_TRAFFIC"),
        1 | 2 => Some("ACCESS_TRAFFIC"),
        6..=9 | 17 | 22..=27 => Some("ALARM_TRAFFIC"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::traffic_label;
    #[test]
    fn labels_match_php_traffic_groups() {
        for id in [3, 4, 14] {
            assert_eq!(traffic_label(id), Some("ROUTE_MAJOR_TRAFFIC"));
        }
        for id in [5, 15, 16, 18] {
            assert_eq!(traffic_label(id), Some("ROUTE_MINOR_TRAFFIC"));
        }
        for id in [1, 2] {
            assert_eq!(traffic_label(id), Some("ACCESS_TRAFFIC"));
        }
        for id in [6, 7, 8, 9, 17, 22, 23, 24, 25, 26, 27] {
            assert_eq!(traffic_label(id), Some("ALARM_TRAFFIC"));
        }
        assert_eq!(traffic_label(99), None);
    }
}
