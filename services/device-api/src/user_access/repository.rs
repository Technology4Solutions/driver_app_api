use super::model::Access;
use sqlx::{MySql, MySqlPool, QueryBuilder, Row};

pub(super) async fn store(pool: &MySqlPool, request: &Access) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let _: i64 = sqlx::query_scalar(
        "SELECT CAST(device_id AS SIGNED) FROM reaxium_device WHERE device_id=? FOR UPDATE",
    )
    .bind(request.device_id)
    .fetch_one(&mut *tx)
    .await?;
    // Match GET_USER_ACCESS_ID: first matching user and method, no status filter.
    let access_id:Option<i64>=sqlx::query_scalar("SELECT CAST(user_access_data_id AS SIGNED) FROM user_access_data WHERE user_id=? AND access_type_id=? LIMIT 1")
        .bind(request.user_id).bind(request.access_type).fetch_optional(&mut *tx).await?;
    let assigned_bus:Option<String>=sqlx::query_scalar("SELECT CAST(b.bus_number AS CHAR) FROM bus_routes_relationship rel LEFT JOIN bus b ON b.bus_id=rel.bus_id WHERE rel.id_route=? ORDER BY rel.last_update DESC LIMIT 1")
        .bind(request.route_id).fetch_optional(&mut *tx).await?.flatten();
    let stamp_id = if request.stop_id_by_distance > 0 {
        request.stop_id_by_distance
    } else {
        request.stop_id
    };
    let stop = sqlx::query(
        "SELECT CAST(stop_number AS CHAR) AS stop_number,stop_name FROM stops WHERE id_stop=?",
    )
    .bind(stamp_id)
    .fetch_optional(&mut *tx)
    .await?;
    let stop_number: Option<String> = stop
        .as_ref()
        .map(|row| row.try_get("stop_number"))
        .transpose()?
        .flatten();
    let stop_name: Option<String> = stop
        .as_ref()
        .map(|row| row.try_get("stop_name"))
        .transpose()?
        .flatten();
    // Historic schemas omit these newer stop snapshot columns. Include only the
    // fixed known fields supported by the installed schema, never request identifiers.
    let columns:Vec<String>=sqlx::query_scalar("SELECT COLUMN_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA=DATABASE() AND TABLE_NAME='traffic'")
        .fetch_all(&mut *tx).await?;
    let has_number = columns
        .iter()
        .any(|name| name.eq_ignore_ascii_case("stop_number"));
    let has_name = columns
        .iter()
        .any(|name| name.eq_ignore_ascii_case("stop_name"));
    let mut query=QueryBuilder::<MySql>::new("INSERT INTO traffic (traffic_type_id,user_id,access_id,device_id,datetime,traffic_info,trace_id,id_route,latitude,longitude,id_stop,id_stop_by_distance,bus_info,access_situation_id,assigned_bus");
    if has_number {
        query.push(",stop_number");
    }
    if has_name {
        query.push(",stop_name");
    }
    query.push(") VALUES (");
    let mut values = query.separated(",");
    values
        .push_bind(request.traffic_type)
        .push_bind(request.user_id)
        .push_bind(access_id)
        .push_bind(request.device_id)
        .push_bind(request.date().expect("validated access date"))
        .push_bind(&request.traffic_info)
        .push_bind(&request.trace_id)
        .push_bind(request.route_id)
        .push_bind(request.latitude)
        .push_bind(request.longitude)
        .push_bind(request.stop_id)
        .push_bind(request.stop_id_by_distance)
        .push_bind(&request.bus_info)
        .push_bind(request.access_situation_id)
        .push_bind(assigned_bus);
    if has_number {
        values.push_bind(stop_number);
    }
    if has_name {
        values.push_bind(stop_name);
    }
    query.push(")");
    let result = query.build().execute(&mut *tx).await?;
    let traffic_id = result.last_insert_id();
    tx.commit().await?;
    tracing::info!(
        traffic_id,
        user_id = request.user_id,
        device_id = request.device_id,
        traffic_type = request.traffic_type,
        "passenger traffic saved"
    );
    Ok(traffic_id)
}
