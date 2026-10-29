use super::{Action, BeginRoute};
use sqlx::{MySqlPool, Row};

pub(super) struct Saved {
    pub traffic_id: u64,
    pub activity_id: Option<u64>,
    pub activities_deleted: u64,
}

pub(super) async fn store(
    pool: &MySqlPool,
    request: &BeginRoute,
    action: Action,
) -> Result<Saved, sqlx::Error> {
    let mut tx = pool.begin().await?;
    // Serialize with DeviceStatus updates for this device. Do not require active status:
    // the PHP persistence path allows inactive devices (Vision checks status separately).
    let _: i64 = sqlx::query_scalar(
        "SELECT CAST(device_id AS SIGNED) FROM reaxium_device WHERE device_id=? FOR UPDATE",
    )
    .bind(request.device_id)
    .fetch_one(&mut *tx)
    .await?;
    let stop = sqlx::query(
        "SELECT CAST(stop_number AS CHAR) AS stop_number, stop_name FROM stops WHERE id_stop=?",
    )
    .bind(request.stop_id)
    .fetch_optional(&mut *tx)
    .await?;
    let stop_number: Option<String> = stop
        .as_ref()
        .map(|r| r.try_get("stop_number"))
        .transpose()?
        .flatten();
    let stop_name: Option<String> = stop
        .as_ref()
        .map(|r| r.try_get("stop_name"))
        .transpose()?
        .flatten();
    // The request's bus_info is a bus ID, not a bus number or route assignment.
    let bus_id = request
        .bus_info
        .trim()
        .parse::<u64>()
        .expect("validated bus ID");
    let assigned_bus: Option<String> =
        sqlx::query_scalar("SELECT CAST(bus_number AS CHAR) FROM bus WHERE bus_id=?")
            .bind(bus_id)
            .fetch_optional(&mut *tx)
            .await?
            .flatten();
    if assigned_bus.is_none() {
        tracing::warn!(
            bus_id,
            "request bus has no bus number; assigned_bus will be NULL"
        );
    }
    let date = request.date().expect("validated route date");
    let traffic = sqlx::query("INSERT INTO device_traffic (device_id,user_id,id_route,id_stop,stop_number,stop_name,latitude,longitude,traffic_type_id,traffic_situation_id,traffic_date,trace_id,bus_info,assigned_bus) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(request.device_id).bind(request.driver_id).bind(request.route_id).bind(request.stop_id)
        .bind(stop_number).bind(stop_name).bind(request.latitude).bind(request.longitude)
        .bind(action.traffic_type()).bind(if action == Action::Begin { 1 } else { request.traffic_situation_id.expect("validated situation") })
        .bind(date).bind(&request.trace_id).bind(&request.bus_info).bind(assigned_bus)
        .execute(&mut *tx).await.map_err(|error| {
            tracing::error!(table="device_traffic", operation="insert", %error, "table write failed; transaction will be rolled back"); error
        })?;
    tracing::info!(
        table = "device_traffic",
        operation = "insert",
        record_id = traffic.last_insert_id(),
        rows_affected = traffic.rows_affected(),
        pending_commit = true,
        "table write completed"
    );
    let (activity_id, activities_deleted) = if action == Action::Begin {
        let activity = sqlx::query("INSERT INTO device_activity (device_id,route_id,activity_date,users_in_track) VALUES (?,?,?,?)")
        .bind(request.device_id).bind(request.route_id).bind(date).bind(&request.users_in_track)
        .execute(&mut *tx).await.map_err(|error| {
            tracing::error!(table="device_activity", operation="insert", %error, "table write failed; transaction will be rolled back"); error
        })?;
        tracing::info!(
            table = "device_activity",
            operation = "insert",
            record_id = activity.last_insert_id(),
            rows_affected = activity.rows_affected(),
            pending_commit = true,
            "table write completed"
        );
        (Some(activity.last_insert_id()), 0)
    } else {
        // Legacy deleteDeviceActivity clears ALL activity rows for this device.
        let result = sqlx::query("DELETE FROM device_activity WHERE device_id=?")
            .bind(request.device_id).execute(&mut *tx).await.map_err(|error| {
                tracing::error!(table="device_activity", operation="delete", %error, "table write failed; transaction will be rolled back"); error
            })?;
        tracing::info!(
            table = "device_activity",
            operation = "delete",
            device_id = request.device_id,
            rows_affected = result.rows_affected(),
            pending_commit = true,
            "table write completed"
        );
        (None, result.rows_affected())
    };
    tx.commit().await?;
    let saved = Saved {
        traffic_id: traffic.last_insert_id(),
        activity_id,
        activities_deleted,
    };
    tracing::info!(
        device_id = request.device_id,
        route_id = request.route_id,
        device_traffic_id = saved.traffic_id,
        device_activity_id = saved.activity_id,
        activities_deleted,
        traffic_type_id = action.traffic_type(),
        "route traffic transaction committed"
    );
    Ok(saved)
}
