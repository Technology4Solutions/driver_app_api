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
    let mut assigned_bus: Option<String> = if matches!(
        action,
        Action::Notification(_)
            | Action::SkippedStop
            | Action::ManualSkippedStop
            | Action::BeginMerged
    ) {
        // Alarm/sendNotification follows PHP's latest route assignment lookup.
        sqlx::query_scalar("SELECT CAST(b.bus_number AS CHAR) FROM bus_routes_relationship rel LEFT JOIN bus b ON b.bus_id=rel.bus_id WHERE rel.id_route=? ORDER BY rel.last_update DESC LIMIT 1")
            .bind(request.route_id).fetch_optional(&mut *tx).await?.flatten()
    } else {
        None
    };
    if !matches!(
        action,
        Action::SkippedStop | Action::ManualSkippedStop | Action::BeginMerged
    ) && assigned_bus
        .as_ref()
        .is_none_or(|number| number.trim().is_empty())
    {
        // Fall back to the reported bus when the route assignment is unavailable.
        assigned_bus =
            sqlx::query_scalar("SELECT CAST(bus_number AS CHAR) FROM bus WHERE bus_id=?")
                .bind(bus_id)
                .fetch_optional(&mut *tx)
                .await?
                .flatten();
    }
    if assigned_bus.is_none() {
        tracing::warn!(
            bus_id,
            route_id = request.route_id,
            assignment_source = match action {
                Action::SkippedStop | Action::ManualSkippedStop | Action::BeginMerged =>
                    "latest_route_assignment",
                Action::Notification(_) => "latest_route_assignment_or_request_bus",
                _ => "request_bus",
            },
            "bus number could not be resolved; assigned_bus will be NULL"
        );
    }
    let date = request.date().expect("validated route date");
    let traffic = sqlx::query("INSERT INTO device_traffic (device_id,user_id,id_route,id_stop,stop_number,stop_name,latitude,longitude,traffic_type_id,traffic_situation_id,traffic_date,trace_id,bus_info,assigned_bus) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(request.device_id).bind(request.driver_id).bind(request.route_id).bind(request.stop_id)
        .bind(stop_number).bind(stop_name).bind(request.latitude).bind(request.longitude)
        .bind(action.traffic_type()).bind(if action == Action::Begin { 1 } else { request.traffic_situation_id.unwrap_or(1) })
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
    let (activity_id, activities_deleted) = if action.traffic_type() == 3 {
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
    } else if matches!(action.traffic_type(), 4 | 14) {
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
    } else {
        (None, 0)
    };
    let alert_name = match action.traffic_type() {
        9 => Some("TRAFFIC DELAY ALERT"),
        17 => Some("STOP DELAY"),
        22 => Some("LATE ATTENDANT ALERT"),
        _ => None,
    };
    if let (Some(name), Some(message)) = (
        alert_name,
        request
            .alert_message
            .as_deref()
            .filter(|m| !m.is_empty() && *m != "0"),
    ) {
        // Device requests use Basic API credentials rather than a user session.
        // Attribute the required integer audit field to the reporting driver.
        sqlx::query("INSERT INTO alerts (alert_name,alert_message,trace_id,traffic_type_id,alert_date,bus_id,id_route,device_traffic_id,session_id) VALUES (?,?,?,?,?,?,?,?,?)")
            .bind(name).bind(message).bind(&request.trace_id).bind(action.traffic_type())
            .bind(date).bind(bus_id).bind(request.route_id).bind(traffic.last_insert_id()).bind(request.driver_id)
            .execute(&mut *tx).await.map_err(|error| {
                tracing::error!(table="alerts", operation="insert", %error, "alert insert failed; transaction will be rolled back"); error
            })?;
    }
    if action == Action::Interrupt {
        // Match dropPassengersByTraceID.sql: a boarding remains aboard when no
        // later drop-off exists for that passenger and trace. Keep legacy metadata
        // and server-time semantics; bind all request values as SQL parameters.
        // Deployments differ in their denormalized passenger metadata columns.
        // Include each snapshot only when traffic actually stores it; required
        // traffic fields still fail normally if the schema is incompatible.
        let columns: Vec<String> = sqlx::query_scalar(
            "SELECT COLUMN_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA=DATABASE() AND TABLE_NAME='traffic'",
        ).fetch_all(&mut *tx).await?;
        let mut optional_columns = String::new();
        let mut optional_values = String::new();
        for (column, expression) in [
            ("stop_number", "s.stop_number"),
            ("stop_name", "s.stop_name"),
            ("session_uid", "t.session_uid"),
            ("assigned_bus", "t.assigned_bus"),
        ] {
            if columns.iter().any(|name| name.eq_ignore_ascii_case(column)) {
                optional_columns.push_str(&format!(", {column}"));
                optional_values.push_str(&format!(", {expression}"));
            }
        }
        // Only the fixed identifiers above enter SQL text, never request data.
        let dropoff_sql = include_str!("drop_passengers.sql")
            .replace("/* optional_columns */", &optional_columns)
            .replace("/* optional_values */", &optional_values);
        let result = sqlx::query(&dropoff_sql)
            .bind(request.stop_id)
            .bind(request.latitude)
            .bind(request.longitude)
            .bind(request.stop_id)
            .bind(request.stop_id)
            .bind(&request.trace_id)
            .execute(&mut *tx)
            .await.map_err(|error| {
                tracing::error!(table="traffic", operation="insert", %error, "passenger drop-off failed; transaction will be rolled back"); error
            })?;
        tracing::info!(
            table = "traffic",
            operation = "insert",
            rows_affected = result.rows_affected(),
            pending_commit = true,
            "automatic passenger drop-offs saved"
        );
    }
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
