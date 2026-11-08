use crate::device_status::broadcast::{channels, row_json, send};
use serde_json::json;
use sqlx::MySqlPool;

pub(super) async fn dispatch(
    pool: &MySqlPool,
    config: &crate::device_status::Config,
    device_id: i64,
    traffic_id: u64,
) -> Result<&'static str, super::Error> {
    // Match GET_TRAFFIC_INFO_BY_ID, including its distance-stop INNER JOIN.
    let row=sqlx::query("SELECT t.trace_id,t.bus_info,t.device_id,CAST(t.datetime AS CHAR) AS datetime,t.latitude,t.longitude,t.traffic_type_id AS typeTrafficId,tt.traffic_type_name AS typeTrafficName,t.access_situation_id AS accessSituationId,u.user_id,CONCAT(u.first_name,' ',u.first_last_name) AS passenger_name,u.document_id AS passenger_id,u.user_photo AS passenger_photo,CAST(u.birthdate AS CHAR) AS passenger_birthdate,r.id_route,r.route_number,r.route_name,r.route_type,rt.route_type_name,s.id_stop,s.stop_number,s.stop_name FROM traffic t INNER JOIN traffic_type tt ON t.traffic_type_id=tt.traffic_type_id INNER JOIN routes r ON t.id_route=r.id_route INNER JOIN route_type rt ON r.route_type=rt.route_type_id INNER JOIN stops s ON t.id_stop_by_distance=s.id_stop INNER JOIN users u ON t.user_id=u.user_id WHERE t.traffic_id=?")
        .bind(traffic_id).fetch_one(pool).await?;
    let mut content = row_json(row)?;
    content["traffic_label"] = json!(crate::device_traffic::traffic_label(
        content["typeTrafficId"].as_i64().unwrap_or_default()
    ));
    let channels = channels(pool, device_id).await?;
    if channels.is_empty() {
        return Ok("skipped_no_businesses");
    }
    content["channels"] = json!(channels);
    send(config, &content).await?;
    Ok("sent")
}
