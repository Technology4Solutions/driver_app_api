mod broadcast;
mod cache;
mod model;
mod repository;
pub mod queue;

use crate::{device_status::{response, Config}, json_body::LegacyJson};
use axum::{extract::rejection::JsonRejection, http::{HeaderMap, StatusCode}, Extension, Json};
use serde_json::Value;
use std::sync::Arc;
type Error = Box<dyn std::error::Error + Send + Sync>;

pub async fn bulk_user_access(
    publisher: Option<Extension<queue::Publisher>>,
    Extension(config): Extension<Arc<Config>>,
    headers: HeaderMap,
    body: Result<LegacyJson, JsonRejection>,
) -> (StatusCode, Json<Value>) {
    if !config.authorized(&headers) { return (StatusCode::OK, response(3, "Invalid Authentication")); }
    let failure = || (StatusCode::BAD_REQUEST, response(500, "Error reporting device access"));
    let Ok(LegacyJson(body)) = body else { return failure(); };
    let Ok(events) = model::parse(&body) else { return failure(); };
    let Some(Extension(publisher)) = publisher else { return failure(); };
    match publisher.publish(&model::message(&events)).await {
        Ok(()) => {
            tracing::info!(batch_size=events.len(), "Device Access queued");
            (StatusCode::OK, response(0, "Device Access queued"))
        }
        Err(error) => { tracing::error!(%error,"Error queueing Device Access"); failure() }
    }
}
