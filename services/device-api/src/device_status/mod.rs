pub(crate) mod broadcast;
mod filter;
pub(crate) mod http_broadcast;
pub(crate) mod model;
pub mod queue;
mod repository;

use crate::json_body::LegacyJson;
use axum::{extract::rejection::JsonRejection, http::HeaderMap, Extension, Json};
pub use model::Update;
use serde_json::{json, Value};
use std::{env, sync::Arc, time::Duration};

/// JWT signing and outbound broadcast configuration.
#[derive(Clone)]
pub struct Config {
    pub jwt_secret: String,
    pub dispatcher_token: String,
    pub broadcast_enabled: bool,
    pub broadcast_endpoint: String,
    pub broadcast_timeout: Duration,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let timeout: u64 = env::var("DEVICE_STATUS_BROADCAST_TIMEOUT_SECONDS")
            .unwrap_or_else(|_| "5".into())
            .parse()?;
        if timeout == 0 {
            return Err("DEVICE_STATUS_BROADCAST_TIMEOUT_SECONDS must be positive".into());
        }
        let broadcast_enabled = env::var("DASHBOARD_BROADCAST_ENABLED")
            .unwrap_or_else(|_| "true".into())
            .parse::<bool>()?;
        tracing::info!(
            broadcast_enabled,
            "dashboard broadcast configuration loaded"
        );
        let jwt_secret = env::var("REAXIUM_API_JWT_SECRET").unwrap_or_default();
        if jwt_secret.len() < 32 {
            return Err("REAXIUM_API_JWT_SECRET must contain at least 32 bytes".into());
        }
        let dispatcher_token = env::var("DISPATCHER_BEARER_TOKEN").unwrap_or_default();
        let broadcast_endpoint =
            env::var("DEVICES_API_EXPORT_DEVICE_STATUS_URL").unwrap_or_default();
        if broadcast_enabled
            && (dispatcher_token.trim().is_empty() || broadcast_endpoint.is_empty())
        {
            return Err(
                "enabled dashboard broadcasts require URL and DISPATCHER_BEARER_TOKEN".into(),
            );
        }
        Ok(Self {
            jwt_secret,
            broadcast_enabled,
            dispatcher_token,
            broadcast_endpoint,
            broadcast_timeout: Duration::from_secs(timeout),
        })
    }

    pub(crate) fn authorized(&self, headers: &HeaderMap) -> bool {
        crate::auth::authorized(self, headers)
    }
}

pub(crate) fn response(code: u16, message: impl Into<String>) -> Json<Value> {
    Json(json!({"ReaxiumResponse": {"code": code, "message": message.into(), "object": []}}))
}

static NEXT_REQUEST_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[tracing::instrument(skip_all, fields(request_id = NEXT_REQUEST_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)))]
pub async fn update(
    publisher: Option<Extension<queue::Publisher>>,
    Extension(config): Extension<Arc<Config>>,
    headers: HeaderMap,
    body: Result<LegacyJson, JsonRejection>,
) -> (axum::http::StatusCode, Json<Value>) {
    tracing::info!("device status request received");
    // Legacy clients inspect the envelope code, including for failures (HTTP 200).
    if !config.authorized(&headers) {
        tracing::warn!("device status rejected: invalid authentication; no data written");
        return (
            axum::http::StatusCode::OK,
            response(3, "Invalid Authentication"),
        );
    }
    let body = match body {
        Ok(LegacyJson(body)) => body,
        Err(error) => {
            let rejection = match &error {
                JsonRejection::MissingJsonContentType(_) => "missing_or_invalid_content_type",
                JsonRejection::JsonSyntaxError(_) => "invalid_json_syntax",
                JsonRejection::JsonDataError(_) => "invalid_json_data",
                JsonRejection::BytesRejection(_) => "body_read_or_size_limit",
                _ => "unknown_json_rejection",
            };
            tracing::warn!(
                rejection,
                status = error.status().as_u16(),
                "device status rejected: invalid JSON; no data written"
            );
            return (
                axum::http::StatusCode::BAD_REQUEST,
                response(500, "Error reporting device status"),
            );
        }
    };
    let Some(parameters) = body.get("ReaxiumParameters") else {
        tracing::warn!("device status rejected: missing ReaxiumParameters; no data written");
        return (
            axum::http::StatusCode::BAD_REQUEST,
            response(500, "Error reporting device status"),
        );
    };
    let parsed = parameters
        .get("DeviceStatus")
        .cloned()
        .and_then(|v| serde_json::from_value::<Update>(v).ok());
    let Some(_request) = parsed.filter(|r| r.valid()) else {
        tracing::warn!("device status rejected: invalid parameters; no data written");
        return (
            axum::http::StatusCode::BAD_REQUEST,
            response(500, "Error reporting device status"),
        );
    };
    tracing::info!("device status payload validated");
    let Some(Extension(publisher)) = publisher else {
        tracing::warn!("device status queue publisher unavailable; no message queued");
        return (
            axum::http::StatusCode::BAD_REQUEST,
            response(500, "Error reporting device status"),
        );
    };
    match publisher.publish(&body).await {
        Ok(()) => {
            tracing::info!("device status message queued; database processing is asynchronous");
            (
                axum::http::StatusCode::OK,
                response(0, "Device Status queued"),
            )
        }
        Err(error) => {
            tracing::error!(%error, "Error queueing Device Status");
            (
                axum::http::StatusCode::BAD_REQUEST,
                response(500, "Error reporting device status"),
            )
        }
    }
}
