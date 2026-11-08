mod broadcast;
mod model;
mod repository;

use crate::device_status::{response, Config};
use axum::{extract::rejection::JsonRejection, http::HeaderMap, Extension, Json};
use serde_json::{json, Value};
use sqlx::MySqlPool;
use std::sync::Arc;
type Error = Box<dyn std::error::Error + Send + Sync>;

#[tracing::instrument(skip_all)]
pub async fn execute_access(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Json<Value> {
    if !config.authorized(&headers) {
        return response(3, "Invalid Authentication");
    }
    let Ok(Json(body)) = body else {
        return response(13, "Invalid Json Object");
    };
    let Some(parameters) = body.get("ReaxiumParameters") else {
        return response(13, "Invalid Json Header");
    };
    let Some(request) = parameters
        .get("UserAccess")
        .cloned()
        .and_then(|value| serde_json::from_value::<model::Access>(value).ok())
        .filter(model::Access::valid)
    else {
        return response(
            13,
            "Invalid Parameters received, please checkout the api documentation",
        );
    };
    let traffic_id = match repository::store(&pool, &request).await {
        Ok(id) => id,
        Err(error) => {
            tracing::error!(%error,user_id=request.user_id,device_id=request.device_id,"passenger traffic transaction failed");
            return response(13,"Passenger traffic could not be saved. Check API logs and database before retrying.");
        }
    };
    let status = if !config.broadcast_enabled {
        "disabled"
    } else {
        match tokio::time::timeout(
            config.broadcast_timeout,
            broadcast::dispatch(
                &pool,
                &config.broadcast_endpoint,
                request.device_id,
                traffic_id,
            ),
        )
        .await
        {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => {
                tracing::warn!(%error,traffic_id,"passenger traffic publication failed");
                "failed"
            }
            Err(_) => {
                tracing::warn!(traffic_id, "passenger traffic publication timed out");
                "timed_out"
            }
        }
    };
    if matches!(status, "sent" | "skipped_no_businesses" | "disabled") {
        return response(0, "SUCCESSFUL REQUEST");
    }
    Json(
        json!({"ReaxiumResponse":{"code":13,"message":"Passenger traffic saved, but downstream delivery is incomplete. Do not resubmit this access event.","object":{"database_saved":true,"traffic_id":traffic_id,"device_id":request.device_id,"broadcast_status":status}}}),
    )
}
