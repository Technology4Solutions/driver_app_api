mod broadcast;
mod model;
mod repository;
mod vision;

use crate::device_status::{response, Config};
use axum::{extract::rejection::JsonRejection, http::HeaderMap, Extension, Json};
pub use model::BeginRoute;
use serde_json::{json, Value};
use sqlx::MySqlPool;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
pub use vision::Config as VisionConfig;
type Error = Box<dyn std::error::Error + Send + Sync>;
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Action {
    Begin,
    End,
}
impl Action {
    fn traffic_type(self) -> i64 {
        match self {
            Self::Begin => 3,
            Self::End => 4,
        }
    }
    fn vision_event(self) -> u16 {
        match self {
            Self::Begin => 401,
            Self::End => 402,
        }
    }
}

#[tracing::instrument(skip_all, fields(request_id=NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)))]
pub async fn begin_route(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(vision): Extension<Arc<VisionConfig>>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Json<Value> {
    handle(pool, config, vision, headers, body, Action::Begin).await
}

#[tracing::instrument(skip_all, fields(request_id=NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)))]
pub async fn end_route(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(vision): Extension<Arc<VisionConfig>>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Json<Value> {
    handle(pool, config, vision, headers, body, Action::End).await
}

async fn handle(
    pool: MySqlPool,
    config: Arc<Config>,
    vision: Arc<VisionConfig>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
    action: Action,
) -> Json<Value> {
    if !config.authorized(&headers) {
        return response(3, "Invalid Authentication");
    }
    let Ok(Json(body)) = body else {
        return response(3, "Invalid Json Object");
    };
    let Some(parameters) = body.get("ReaxiumParameters") else {
        return response(3, "Invalid Json Header");
    };
    let request = parameters
        .get("DeviceTraffic")
        .cloned()
        .and_then(|v| serde_json::from_value::<BeginRoute>(v).ok());
    let Some(request) = request.filter(|r| {
        r.valid() && (action == Action::Begin || r.traffic_situation_id.is_some_and(|id| id > 0))
    }) else {
        return response(
            3,
            "Invalid Parameters received, please checkout the api documentation",
        );
    };
    let saved = match repository::store(&pool, &request, action).await {
        Ok(saved) => saved,
        Err(error) => {
            tracing::error!(%error, device_id=request.device_id, "route traffic transaction failed; no success reported");
            // A connection error during COMMIT can leave the outcome unknown. Do not
            // claim database_saved=false or encourage automatic retries.
            return response(
                3,
                "Database transaction failed. Check API logs and database before retrying.",
            );
        }
    };
    let vision_status = match tokio::time::timeout(
        config.broadcast_timeout,
        vision::dispatch(&pool, &vision, &request, action),
    )
    .await
    {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            tracing::warn!(%error, "route traffic Vision send failed");
            "failed"
        }
        Err(_) => {
            tracing::warn!("route traffic Vision send timed out");
            "timed_out"
        }
    };
    // Attempt dashboard publication even if Vision failed.
    let broadcast_status = if !config.broadcast_enabled {
        "disabled"
    } else {
        match tokio::time::timeout(
            config.broadcast_timeout,
            broadcast::dispatch(
                &pool,
                &config.broadcast_endpoint,
                request.device_id,
                saved.traffic_id,
            ),
        )
        .await
        {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => {
                tracing::warn!(%error, "route traffic broadcast failed");
                "failed"
            }
            Err(_) => {
                tracing::warn!("route traffic broadcast timed out");
                "timed_out"
            }
        }
    };
    tracing::info!(
        device_id = request.device_id,
        device_traffic_id = saved.traffic_id,
        vision_status,
        broadcast_status,
        database_saved = true,
        "route traffic completed"
    );
    if matches!(
        vision_status,
        "sent" | "skipped_inactive_device" | "disabled"
    ) && matches!(
        broadcast_status,
        "sent" | "skipped_no_businesses" | "disabled"
    ) {
        return response(0, "SUCCESSFUL REQUEST");
    }
    Json(json!({"ReaxiumResponse":{
        "code":3,
        "message":"Database changes saved, but downstream delivery is incomplete. Do not resubmit this route event.",
        "object":{"database_saved":true,"device_id":request.device_id,"device_traffic_id":saved.traffic_id,"device_activity_id":saved.activity_id,"activities_deleted":saved.activities_deleted,"vision_status":vision_status,"broadcast_status":broadcast_status}
    }}))
}
