pub(crate) mod broadcast;
mod filter;
mod http_broadcast;
pub(crate) mod model;
pub mod queue;
mod repository;

use axum::{extract::rejection::JsonRejection, http::HeaderMap, Extension, Json};
use base64::{engine::general_purpose::STANDARD, Engine};
pub use model::Update;
use serde_json::{json, Value};
use std::{env, sync::Arc, time::Duration};
use subtle::ConstantTimeEq;

/// Shared API credentials, using the same names and rotation aliases as ReaxiumCloud.
#[derive(Clone)]
pub struct Config {
    pub username: String,
    pub password: String,
    pub previous_password: String,
    pub broadcast_enabled: bool,
    pub broadcast_endpoint: String,
    pub broadcast_timeout: Duration,
}

impl Config {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        fn first(names: &[&str]) -> String {
            names
                .iter()
                .filter_map(|n| env::var(n).ok())
                .find(|s| !s.is_empty())
                .unwrap_or_default()
        }
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
        Ok(Self {
            broadcast_enabled,
            username: first(&["REAXIUM_API_USER", "REAXIUM_API_BASIC_AUTH_USERNAME"]),
            password: first(&["REAXIUM_API_PASSWORD", "REAXIUM_API_BASIC_AUTH_PASSWORD"]),
            previous_password: first(&["REAXIUM_API_PASSWORD_PREVIOUS"]),
            broadcast_endpoint: env::var("DEVICE_STATUS_ZMQ_ENDPOINT")
                .unwrap_or_else(|_| "tcp://127.0.0.1:5555".into()),
            broadcast_timeout: Duration::from_secs(timeout),
        })
    }

    pub(crate) fn authorized(&self, headers: &HeaderMap) -> bool {
        let Some(header) = headers.get("authorization").and_then(|h| h.to_str().ok()) else {
            return false;
        };
        let Some((scheme, encoded)) = header.split_once(' ') else {
            return false;
        };
        if !scheme.eq_ignore_ascii_case("basic") {
            return false;
        }
        let Ok(bytes) = STANDARD.decode(encoded.trim()) else {
            return false;
        };
        let Some(colon) = bytes.iter().position(|b| *b == b':') else {
            return false;
        };
        let user = &bytes[..colon];
        let password = &bytes[colon + 1..];
        let user_matches =
            !self.username.is_empty() & bool::from(user.ct_eq(self.username.as_bytes()));
        let current =
            !self.password.is_empty() & bool::from(password.ct_eq(self.password.as_bytes()));
        let previous = !self.previous_password.is_empty()
            & bool::from(password.ct_eq(self.previous_password.as_bytes()));
        user_matches & (current | previous)
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
    body: Result<Json<Value>, JsonRejection>,
) -> (axum::http::StatusCode, Json<Value>) {
    // Legacy clients inspect the envelope code, including for failures (HTTP 200).
    if !config.authorized(&headers) {
        tracing::warn!("device status rejected: invalid authentication; no data written");
        return (
            axum::http::StatusCode::OK,
            response(3, "Invalid Authentication"),
        );
    }
    let Ok(Json(body)) = body else {
        tracing::warn!("device status rejected: invalid JSON; no data written");
        return (
            axum::http::StatusCode::BAD_REQUEST,
            response(500, "Error reporting device status"),
        );
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
    let Some(Extension(publisher)) = publisher else {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            response(500, "Error reporting device status"),
        );
    };
    match publisher.publish(&body).await {
        Ok(()) => (
            axum::http::StatusCode::OK,
            response(0, "Device Status queued"),
        ),
        Err(error) => {
            tracing::error!(%error, "Error queueing Device Status");
            (
                axum::http::StatusCode::BAD_REQUEST,
                response(500, "Error reporting device status"),
            )
        }
    }
}
