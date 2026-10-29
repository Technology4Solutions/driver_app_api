use super::Update;
use sqlx::MySqlPool;

pub async fn store(pool: &MySqlPool, request: &Update) -> Result<Option<i64>, sqlx::Error> {
    tracing::info!(business_id = request.business_id, status_date = %request.status_date, "device status transaction starting");
    let mut tx = pool.begin().await?;
    // Serialize updates for one device even if no status/activity row exists yet.
    let device_id: Option<i64> = sqlx::query_scalar("SELECT CAST(device_id AS SIGNED) FROM reaxium_device WHERE device_serial = ? AND status_id = 1 LIMIT 1 FOR UPDATE")
        .bind(&request.device_imei).fetch_optional(&mut *tx).await?;
    let Some(device_id) = device_id else {
        tracing::warn!(
            table = "reaxium_device",
            "no active device matches the supplied IMEI; no data written"
        );
        tx.rollback().await?;
        return Ok(None);
    };
    tracing::info!(table = "reaxium_device", device_id, "active device found");
    let user_id: Option<i64> =
        sqlx::query_scalar("SELECT CAST(user_id AS SIGNED) FROM users WHERE user_id=? LIMIT 1")
            .bind(request.user_id)
            .fetch_optional(&mut *tx)
            .await?;
    let l = &request.device_location;
    let location_result = sqlx::query("INSERT INTO device_location (device_id, user_id, latitude, longitude, speed, bearing, accuracy, lag_time, date_location) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(device_id).bind(user_id).bind(l.latitude).bind(l.longitude).bind(l.speed)
        .bind(l.bearing).bind(l.accuracy).bind(l.lag_time).bind(&request.status_date)
        .execute(&mut *tx).await.map_err(|error| write_error(error, "device_location", "insert"))?;
    let location_id = location_result.last_insert_id();
    tracing::info!(
        table = "device_location",
        operation = "insert",
        device_id,
        device_location_id = location_id,
        rows_affected = location_result.rows_affected(),
        pending_commit = true,
        "table write executed; awaiting transaction commit"
    );
    let status_id: Option<i64> = sqlx::query_scalar(
        "SELECT CAST(device_status_id AS SIGNED) FROM device_status WHERE device_id = ? LIMIT 1",
    )
    .bind(device_id)
    .fetch_optional(&mut *tx)
    .await?;
    let s = &request.device_status;
    // Match Java: last_sync is replaced by the supplied value (or NULL).
    let last_sync = s
        .last_sync
        .as_deref()
        .and_then(|value| chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S").ok());
    let statement = if status_id.is_some() {
        "UPDATE device_status SET business_id=?, temperature=?, battery_power_level=?, is_charging=?, device_location_id=?, last_status_update=?, installed_app_version=?, bus_info=?, last_sync=? WHERE device_status_id=?"
    } else {
        "INSERT INTO device_status (business_id, temperature, battery_power_level, is_charging, device_location_id, last_status_update, installed_app_version, bus_info, last_sync, device_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    };
    let status_result = sqlx::query(statement)
        .bind(request.business_id)
        .bind(s.temperature)
        .bind(s.battery_power_level)
        .bind(s.is_charging)
        .bind(location_id)
        .bind(&request.status_date)
        .bind(&s.installed_app_version)
        .bind(s.bus_info.as_deref())
        .bind(last_sync)
        .bind(status_id.unwrap_or(device_id))
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            write_error(
                error,
                "device_status",
                if status_id.is_some() {
                    "update"
                } else {
                    "insert"
                },
            )
        })?;
    let saved_status_id = status_id
        .map(|id| id as u64)
        .unwrap_or_else(|| status_result.last_insert_id());
    tracing::info!(
        table = "device_status",
        operation = if status_id.is_some() {
            "update"
        } else {
            "insert"
        },
        device_id,
        device_status_id = saved_status_id,
        device_location_id = location_id,
        rows_affected = status_result.rows_affected(),
        pending_commit = true,
        "table write executed; awaiting transaction commit"
    );
    if let Some(activity) = &request.device_activity {
        let activity_id: Option<i64> = sqlx::query_scalar("SELECT CAST(device_activity_id AS SIGNED) FROM device_activity WHERE device_id=? LIMIT 1")
            .bind(device_id).fetch_optional(&mut *tx).await?;
        let users = activity
            .users_in_track
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let statement = if activity_id.is_some() {
            "UPDATE device_activity SET route_id=?, users_in_track=?, activity_date=? WHERE device_activity_id=?"
        } else {
            "INSERT INTO device_activity (route_id, users_in_track, activity_date, device_id) VALUES (?, ?, ?, ?)"
        };
        let activity_result = sqlx::query(statement)
            .bind(activity.route_id)
            .bind(users)
            .bind(chrono::Utc::now().naive_utc())
            .bind(activity_id.unwrap_or(device_id))
            .execute(&mut *tx)
            .await
            .map_err(|error| {
                write_error(
                    error,
                    "device_activity",
                    if activity_id.is_some() {
                        "update"
                    } else {
                        "insert"
                    },
                )
            })?;
        tracing::info!(
            table = "device_activity",
            operation = if activity_id.is_some() {
                "update"
            } else {
                "insert"
            },
            device_id,
            device_activity_id = activity_id
                .map(|id| id as u64)
                .unwrap_or_else(|| activity_result.last_insert_id()),
            rows_affected = activity_result.rows_affected(),
            pending_commit = true,
            "table write executed; awaiting transaction commit"
        );
    } else {
        let activity_result = sqlx::query("DELETE FROM device_activity WHERE device_id=?")
            .bind(device_id)
            .execute(&mut *tx)
            .await
            .map_err(|error| write_error(error, "device_activity", "delete"))?;
        tracing::info!(
            table = "device_activity",
            operation = "delete",
            device_id,
            rows_affected = activity_result.rows_affected(),
            pending_commit = true,
            "activity omitted; delete executed, awaiting transaction commit"
        );
    }
    tx.commit().await?;
    tracing::info!(
        device_id,
        device_location_id = location_id,
        device_status_id = saved_status_id,
        "device status transaction committed"
    );
    Ok(Some(device_id))
}

fn write_error(error: sqlx::Error, table: &str, operation: &str) -> sqlx::Error {
    tracing::error!(table, operation, %error, "table write failed; transaction will be rolled back");
    error
}
