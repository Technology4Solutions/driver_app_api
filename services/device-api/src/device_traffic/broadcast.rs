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
    content["traffic_label"] = json!("ROUTE_MAJOR_TRAFFIC");
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
