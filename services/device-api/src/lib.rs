pub mod database;
pub mod device_status;
pub mod device_traffic;
pub mod telemetry;

use axum::{middleware, routing::get, Router};
use telemetry::Metrics;
use tower_http::trace::TraceLayer;

pub fn router(metrics: Metrics) -> Router {
    Router::new()
        .route("/", get(|| async { "Hello, world!" }))
        .route(
            "/device/DeviceStatus/update",
            axum::routing::post(device_status::update),
        )
        .route(
            "/api/DeviceStatus/updateStatus",
            axum::routing::post(device_status::update),
        )
        .route(
            "/DeviceTraffic/beginARoute",
            axum::routing::post(device_traffic::begin_route),
        )
        .route(
            "/school_bus_cloud/DeviceTraffic/beginARoute",
            axum::routing::post(device_traffic::begin_route),
        )
        .route(
            "/DeviceTraffic/endARoute",
            axum::routing::post(device_traffic::end_route),
        )
        .route(
            "/school_bus_cloud/DeviceTraffic/endARoute",
            axum::routing::post(device_traffic::end_route),
        )
        .layer(middleware::from_fn_with_state(metrics, telemetry::observe))
        .layer(TraceLayer::new_for_http())
}

pub fn admin_router(metrics: Metrics) -> Router {
    Router::new()
        .route("/health/live", get(|| async { "ok" }))
        .route(
            "/metrics",
            get(move || {
                let metrics = metrics.clone();
                async move {
                    metrics
                        .render()
                        .map(|body| {
                            (
                                [(
                                    axum::http::header::CONTENT_TYPE,
                                    "text/plain; version=0.0.4",
                                )],
                                body,
                            )
                        })
                        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                }
            }),
        )
}
