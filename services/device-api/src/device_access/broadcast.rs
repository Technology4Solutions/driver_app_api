use crate::device_status::broadcast::row_json;
use sqlx::MySqlPool;
use serde_json::Value;
pub(super) async fn payload(pool: &MySqlPool, traffic_id: u64) -> Result<Option<Value>, sqlx::Error> {
    let row = sqlx::query("SELECT t.trace_id,t.bus_info,t.device_id,CAST(t.datetime AS CHAR) AS datetime,t.latitude,t.longitude,t.traffic_type_id AS typeTrafficId,tt.traffic_type_name AS typeTrafficName,t.access_situation_id AS accessSituationId,u.user_id,CONCAT(u.first_name,' ',u.first_last_name) AS passenger_name,u.document_id AS passenger_id,u.user_photo AS passenger_photo,CAST(u.birthdate AS CHAR) AS passenger_birthdate,r.id_route,r.route_number,r.route_name,r.route_type,rt.route_type_name,s.id_stop,s.stop_number,s.stop_name,'ACCESS_TRAFFIC' AS traffic_label FROM traffic t INNER JOIN traffic_type tt ON t.traffic_type_id=tt.traffic_type_id INNER JOIN routes r ON t.id_route=r.id_route INNER JOIN route_type rt ON r.route_type=rt.route_type_id INNER JOIN stops s ON t.id_stop_by_distance=s.id_stop INNER JOIN users u ON t.user_id=u.user_id WHERE t.traffic_id=?")
        .bind(traffic_id).fetch_optional(pool).await?;
    row.map(row_json).transpose()
}
