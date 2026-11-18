use super::{cache::Cache, model::Access, Error};
use sqlx::{MySql, MySqlPool, QueryBuilder, Row};

pub(super) async fn store(pool: &MySqlPool, cache: &Cache, events: Vec<Option<Access>>) -> Result<Vec<u64>, Error> {
    let mut prepared = Vec::new();
    for event in events {
        let Some(event) = event else { tracing::warn!("null bulk access skipped"); continue };
        let credentials = match cache.credentials(pool, event.user_id, event.access_type).await {
            Ok(value) => value,
            Err(error) => { tracing::error!(%error,"bulk credential lookup failed"); None }
        };
        let Some(access_id) = credentials else {
            tracing::warn!(user_id=?event.user_id,"bulk access skipped: credentials not found"); continue
        };
        let Some(date) = event.date() else {
            tracing::warn!(user_id=?event.user_id,"bulk access skipped: invalid accessDate"); continue
        };
        let bus = match cache.bus(pool, event.route_id).await {
            Ok(value) => value,
            Err(error) => { tracing::error!(%error,"bulk bus lookup failed"); None }
        };
        prepared.push((event,access_id,date,bus));
    }
    if prepared.is_empty() { return Err("Failed to save any access from batch".into()) }
    let mut tx = pool.begin().await?;
    let columns: Vec<String> = sqlx::query_scalar("SELECT COLUMN_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA=DATABASE() AND TABLE_NAME='traffic'").fetch_all(&mut *tx).await?;
    let has = |name: &str| columns.iter().any(|column| column.eq_ignore_ascii_case(name));
    let mut saved = Vec::new();
    for (event,access_id,date,bus) in prepared {
        let stamp = event.stop_id_by_distance.filter(|id| *id>0).or(event.stop_id);
        let stop = if has("stop_number") || has("stop_name") {
            sqlx::query("SELECT CAST(stop_number AS CHAR) AS stop_number, stop_name FROM stops WHERE id_stop=?").bind(stamp).fetch_optional(&mut *tx).await?
        } else {None};
        let number: Option<String> = stop.as_ref().map(|r| r.try_get("stop_number")).transpose()?.flatten();
        let name: Option<String> = stop.as_ref().map(|r| r.try_get("stop_name")).transpose()?.flatten();
        let mut query=QueryBuilder::<MySql>::new("INSERT INTO traffic (access_id,device_id,access_situation_id,bus_info,assigned_bus,datetime,id_route,id_stop,id_stop_by_distance,latitude,longitude,trace_id,traffic_type_id,traffic_info,user_id");
        for column in ["stop_number","stop_name","last_update"] {if has(column) {query.push(",").push(column);}}
        query.push(") VALUES (");
        let mut values=query.separated(",");
        values.push_bind(access_id).push_bind(event.device_id).push_bind(event.access_situation_id)
            .push_bind(&event.bus_info).push_bind(bus).push_bind(date).push_bind(event.route_id)
            .push_bind(event.stop_id).push_bind(event.stop_id_by_distance)
            .push_bind(event.latitude.map(|n| n.to_string()).unwrap_or_else(|| "null".into()))
            .push_bind(event.longitude.map(|n| n.to_string()).unwrap_or_else(|| "null".into()))
            .push_bind(&event.trace_id).push_bind(event.traffic_type).push_bind(&event.traffic_info).push_bind(event.user_id);
        if has("stop_number") {values.push_bind(number);}
        if has("stop_name") {values.push_bind(name);}
        if has("last_update") {values.push_bind(chrono::Local::now().naive_local());}
        query.push(")");
        // An SQL error aborts the transaction, matching Spring's rollback-only
        // transaction after a repository persistence failure. Never commit a
        // subset after a constraint error or bypass the existing unique key.
        saved.push(query.build().execute(&mut *tx).await?.last_insert_id());
    }
    tx.commit().await?;
    tracing::info!(saved_count=saved.len(), "bulk passenger traffic committed");
    Ok(saved)
}

pub(super) async fn record_failure(pool: &MySqlPool, body: &str, error: &str) -> Result<(), sqlx::Error> {
    let device = serde_json::from_str::<serde_json::Value>(body).ok()
        .and_then(|v| v["ReaxiumParameters"]["BulkUserAccess"][0]["device_id"].as_i64().map(|id| id.to_string()))
        .unwrap_or_else(|| "ERROR_GETTING_DEVICE_ID".into());
    sqlx::query("INSERT INTO messages (title,message,data,stack_trace,topic,device_id,status,created_at,created_by,last_modified_at,last_modified_by) VALUES ('Error processing a device Access after retries',?,?,?,'DEVICE_ACCESS',?,'ERROR',?,'Reaxium Admin',?,'Reaxium Admin')")
        .bind(error).bind(body).bind(error).bind(device)
        .bind(chrono::Local::now().naive_local()).bind(chrono::Local::now().naive_local())
        .execute(pool).await?;
    Ok(())
}
