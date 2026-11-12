mod broadcast;
mod json_body;
pub(crate) use broadcast::traffic_label;
mod merged;
mod model;
pub use merged::{begin_merged_route, end_merged_route, interrupt_merged_route};
mod repository;
mod vision;

use crate::device_status::{response, Config};
use axum::{extract::rejection::JsonRejection, http::HeaderMap, Extension, Json};
use json_body::RouteJson;
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
    BeginMerged,
    EndMerged,
    InterruptMerged,
    End,
    Interrupt,
    Send(i64),
    Notification(i64),
    NextStop,
    SkippedStop,
    ManualSkippedStop,
}
impl Action {
    fn traffic_type(self) -> i64 {
        match self {
            Self::Begin | Self::BeginMerged => 3,
            Self::NextStop => 5,
            Self::SkippedStop => 18,
            Self::ManualSkippedStop => 27,
            Self::End | Self::EndMerged => 4,
            Self::Interrupt | Self::InterruptMerged => 14,
            Self::Send(id) | Self::Notification(id) => id,
        }
    }
    fn vision_event(self) -> Option<u16> {
        match self.traffic_type() {
            3 => Some(401),
            4 | 14 => Some(402),
            _ => None,
        }
    }
}

#[tracing::instrument(skip_all, fields(request_id=NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)))]
pub async fn begin_route(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(vision): Extension<Arc<VisionConfig>>,
    headers: HeaderMap,
    body: Result<RouteJson, JsonRejection>,
) -> Json<Value> {
    handle(pool, config, vision, headers, body, Action::Begin).await
}

#[tracing::instrument(skip_all, fields(request_id=NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)))]
pub async fn end_route(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(vision): Extension<Arc<VisionConfig>>,
    headers: HeaderMap,
    body: Result<RouteJson, JsonRejection>,
) -> Json<Value> {
    handle(pool, config, vision, headers, body, Action::End).await
}

#[tracing::instrument(skip_all, fields(request_id=NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)))]
pub async fn interrupt_route(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(vision): Extension<Arc<VisionConfig>>,
    headers: HeaderMap,
    body: Result<RouteJson, JsonRejection>,
) -> Json<Value> {
    handle(pool, config, vision, headers, body, Action::Interrupt).await
}

#[tracing::instrument(skip_all, fields(request_id=NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)))]
pub async fn send_traffic(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(vision): Extension<Arc<VisionConfig>>,
    headers: HeaderMap,
    body: Result<RouteJson, JsonRejection>,
) -> Json<Value> {
    handle(pool, config, vision, headers, body, Action::Send(0)).await
}

#[tracing::instrument(skip_all, fields(request_id=NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)))]
pub async fn send_notification(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(vision): Extension<Arc<VisionConfig>>,
    headers: HeaderMap,
    body: Result<RouteJson, JsonRejection>,
) -> Json<Value> {
    handle(pool, config, vision, headers, body, Action::Notification(0)).await
}

#[tracing::instrument(skip_all, fields(request_id=NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)))]
pub async fn send_next_stop_notification(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(vision): Extension<Arc<VisionConfig>>,
    headers: HeaderMap,
    body: Result<RouteJson, JsonRejection>,
) -> Json<Value> {
    handle(pool, config, vision, headers, body, Action::NextStop).await
}

#[tracing::instrument(skip_all, fields(request_id=NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)))]
pub async fn send_skipped_stop_notification(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(vision): Extension<Arc<VisionConfig>>,
    headers: HeaderMap,
    body: Result<RouteJson, JsonRejection>,
) -> Json<Value> {
    handle(pool, config, vision, headers, body, Action::SkippedStop).await
}

#[tracing::instrument(skip_all, fields(request_id=NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)))]
pub async fn send_manual_skipped_stop_notification(
    Extension(pool): Extension<MySqlPool>,
    Extension(config): Extension<Arc<Config>>,
    Extension(vision): Extension<Arc<VisionConfig>>,
    headers: HeaderMap,
    body: Result<RouteJson, JsonRejection>,
) -> Json<Value> {
    handle(
        pool,
        config,
        vision,
        headers,
        body,
        Action::ManualSkippedStop,
    )
    .await
}

async fn handle(
    pool: MySqlPool,
    config: Arc<Config>,
    vision: Arc<VisionConfig>,
    headers: HeaderMap,
    body: Result<RouteJson, JsonRejection>,
    action: Action,
) -> Json<Value> {
    tracing::info!(?action, "route traffic request received");
    if !config.authorized(&headers) {
        tracing::warn!(
            ?action,
            reason = "authentication",
            "route traffic request rejected"
        );
        return response(3, "Invalid Authentication");
    }
    let body = match body {
        Ok(RouteJson(body)) => body,
        Err(error) => {
            let rejection = match &error {
                JsonRejection::MissingJsonContentType(_) => "missing_or_invalid_content_type",
                JsonRejection::JsonSyntaxError(_) => "invalid_json_syntax",
                JsonRejection::JsonDataError(_) => "invalid_json_data",
                JsonRejection::BytesRejection(_) => "body_read_or_size_limit",
                _ => "unknown_json_rejection",
            };
            tracing::warn!(
                ?action,
                reason = "json_body",
                rejection,
                status = error.status().as_u16(),
                "route traffic request rejected"
            );
            return response(3, "Invalid Json Object");
        }
    };
    let Some(parameters) = body.get("ReaxiumParameters") else {
        tracing::warn!(
            ?action,
            reason = "missing_ReaxiumParameters",
            "route traffic request rejected"
        );
        return response(3, "Invalid Json Header");
    };
    let request = if matches!(
        action,
        Action::Notification(_)
            | Action::NextStop
            | Action::SkippedStop
            | Action::ManualSkippedStop
    ) {
        parameters.get("Notification").cloned().and_then(|value| {
            if action == Action::NextStop {
                model::fixed_notification_request(value, 5)
            } else if action == Action::SkippedStop {
                model::fixed_notification_request(value, 18)
            } else if action == Action::ManualSkippedStop {
                model::manual_skipped_stop_request(value)
            } else {
                model::notification_request(value)
            }
        })
    } else {
        parameters
            .get("DeviceTraffic")
            .cloned()
            .and_then(|v| match serde_json::from_value::<BeginRoute>(v) {
                Ok(request) => Some(request),
                Err(error) => {
                    tracing::warn!(?action, error_category = ?error.classify(), "route traffic payload could not be decoded; check field types and required fields");
                    None
                }
            })
    };
    if let Some(r) = &request {
        tracing::info!(
            ?action,
            device_id = r.device_id,
            route_id = r.route_id,
            valid_device = r.device_id > 0,
            valid_stop = r.stop_id > 0,
            valid_route = r.route_id > 0 && r.route_id <= u32::MAX as i64,
            valid_driver = r.driver_id > 0,
            has_trace = !r.trace_id.trim().is_empty(),
            valid_bus_id = r.bus_info.trim().parse::<u64>().is_ok_and(|id| id > 0),
            valid_latitude = (-90.0..=90.0).contains(&r.latitude),
            valid_longitude = (-180.0..=180.0).contains(&r.longitude),
            valid_date = r.traffic_date.replace("::", ":").len() == 19 && r.date().is_ok(),
            "route traffic payload validation"
        );
    }
    let Some(request) = request.filter(|r| {
        r.valid()
            && match action {
                Action::Begin
                | Action::Notification(_)
                | Action::NextStop
                | Action::SkippedStop
                | Action::ManualSkippedStop => true,
                Action::Send(_) => {
                    r.traffic_type_id
                        .is_some_and(|id| id > 0 && id <= i32::MAX as i64)
                        && r.traffic_situation_id.is_none_or(|id| id > 0)
                }
                _ => r.traffic_situation_id.is_some_and(|id| id > 0),
            }
    }) else {
        tracing::warn!(
            ?action,
            reason = "invalid_parameters",
            "route traffic request rejected"
        );
        return response(
            3,
            "Invalid Parameters received, please checkout the api documentation",
        );
    };
    let action = match action {
        Action::NextStop => Action::Notification(5),
        Action::Send(_) => Action::Send(request.traffic_type_id.expect("validated traffic type")),
        Action::Notification(_) => Action::Notification(
            request
                .traffic_type_id
                .expect("validated notification type"),
        ),
        action => action,
    };
    process(pool, config, vision, request, action).await
}

async fn process(
    pool: MySqlPool,
    config: Arc<Config>,
    vision: Arc<VisionConfig>,
    request: BeginRoute,
    action: Action,
) -> Json<Value> {
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
            broadcast::dispatch(&pool, &config, request.device_id, saved.traffic_id),
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
        "sent" | "skipped_inactive_device" | "skipped_unsupported_traffic" | "disabled"
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
