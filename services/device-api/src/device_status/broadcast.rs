use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use serde_json::{json, Map, Value};
use sqlx::{mysql::MySqlRow, Column, MySqlPool, Row, TypeInfo, ValueRef};

type Error = Box<dyn std::error::Error + Send + Sync>;

// Convert the legacy relational records to the same nested JSON shape used by the dashboard.
pub(crate) fn row_json(row: MySqlRow) -> Result<Value, sqlx::Error> {
    let mut object = Map::new();
    for column in row.columns() {
        let i = column.ordinal();
        let raw = row.try_get_raw(i)?;
        let kind = column.type_info().name();
        let value = if raw.is_null() {
            Value::Null
        } else if kind.contains("UNSIGNED") {
            json!(row.try_get::<u64, _>(i)?)
        } else {
            match kind {
                "BOOLEAN" | "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" => {
                    json!(row.try_get::<i64, _>(i)?)
                }
                "FLOAT" => json!(row.try_get::<f32, _>(i)?),
                "DOUBLE" => json!(row.try_get::<f64, _>(i)?),
                "DECIMAL" => json!(row.try_get::<sqlx::types::BigDecimal, _>(i)?.to_string()),
                "DATETIME" | "TIMESTAMP" => json!(row
                    .try_get::<NaiveDateTime, _>(i)?
                    .format("%Y-%m-%dT%H:%M:%S+00:00")
                    .to_string()),
                "DATE" => json!(row.try_get::<NaiveDate, _>(i)?.to_string()),
                "TIME" => json!(row.try_get::<NaiveTime, _>(i)?.to_string()),
                "JSON" => row.try_get::<sqlx::types::Json<Value>, _>(i)?.0,
                _ => json!(row.try_get::<String, _>(i)?),
            }
        };
        object.insert(column.name().to_owned(), value);
    }
    Ok(Value::Object(object))
}

pub(crate) async fn channels(pool: &MySqlPool, device_id: i64) -> Result<Vec<String>, sqlx::Error> {
    // Channels come from the last traffic's route, not the untrusted request business_id.
    let last_route: Option<i64> = sqlx::query_scalar("SELECT CAST(id_route AS SIGNED) FROM device_traffic WHERE device_id=? ORDER BY traffic_date DESC LIMIT 1")
        .bind(device_id).fetch_optional(pool).await?.flatten();
    let Some(route_id) = last_route else {
        return Ok(Vec::new());
    };
    let business_ids: Vec<i64> = sqlx::query_scalar("SELECT DISTINCT CAST(u.business_id AS SIGNED) FROM routes r INNER JOIN route_users ru ON r.id_route=ru.id_route INNER JOIN users u ON ru.user_id=u.user_id WHERE r.id_route=? UNION SELECT DISTINCT CAST(br.business_id AS SIGNED) FROM routes r INNER JOIN business_routes br ON r.id_route=br.id_route WHERE r.id_route=? UNION SELECT CAST(business_id AS SIGNED) FROM business WHERE business_type_id=3")
        .bind(route_id).bind(route_id).fetch_all(pool).await?;
    if business_ids.is_empty() {
        return Ok(Vec::new());
    }
    let channels: Vec<String> = business_ids
        .iter()
        .map(|id| format!("CHANNEL_REAXIUM_SCHOOL_BUSINESS_{id}"))
        .collect();
    Ok(channels)
}

pub(crate) async fn send(config: &super::Config, content: &Value) -> Result<(), Error> {
    let response = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(3))
        .timeout(std::time::Duration::from_secs(3))
        .user_agent("DeviceApiWebSocketClient/1.0")
        .build()?
        .post(&config.broadcast_endpoint)
        .basic_auth(&config.username, Some(&config.password))
        .json(&json!({"ReaxiumParameters":{"WebSocketPusher":{"content":content}}}))
        .send()
        .await?
        .error_for_status()?;
    let body: Value = response.json().await?;
    if body["ReaxiumResponse"]["code"].as_i64() != Some(0) {
        return Err("HTTP dashboard dispatcher did not report success".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        http::{HeaderMap, StatusCode},
        routing::post,
        Json, Router,
    };
    use std::time::Duration;

    #[tokio::test]
    async fn http_dispatch_uses_shared_credentials_and_checks_gateway_result() {
        for (status, body, succeeds) in [
            (StatusCode::OK, json!({"ReaxiumResponse":{"code":0}}), true),
            (StatusCode::OK, json!({"ReaxiumResponse":{"code":3}}), false),
            (StatusCode::SERVICE_UNAVAILABLE, json!({}), false),
        ] {
            let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
            let app = Router::new().route(
                "/dispatch",
                post(move |headers: HeaderMap, Json(request): Json<Value>| {
                    let sender = sender.clone();
                    let body = body.clone();
                    async move {
                        sender.send((headers, request)).unwrap();
                        (status, Json(body))
                    }
                }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let config = super::super::Config {
                username: "api".into(),
                password: "password".into(),
                previous_password: String::new(),
                broadcast_enabled: true,
                broadcast_endpoint: format!("http://{}/dispatch", listener.local_addr().unwrap()),
                broadcast_timeout: Duration::from_secs(5),
            };
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let content = json!({"device_id":935,"traffic_label":"ROUTE_MAJOR_TRAFFIC"});
            assert_eq!(send(&config, &content).await.is_ok(), succeeds);
            let (headers, request) = receiver.recv().await.unwrap();
            assert_eq!(headers["authorization"], "Basic YXBpOnBhc3N3b3Jk");
            assert_eq!(
                request,
                json!({"ReaxiumParameters":{"WebSocketPusher":{"content":content}}})
            );
            server.abort();
        }
    }
}
