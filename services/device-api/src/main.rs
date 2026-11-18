use device_api::{admin_router, router, telemetry::Metrics};
use std::{env, net::SocketAddr};
use tracing_subscriber::{filter::filter_fn, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let log_level = env::var("LOG_LEVEL").unwrap_or_else(|_| "INFO".into());
    let debug = match log_level.trim().to_ascii_uppercase().as_str() {
        "INFO" => false,
        "DEBUG" => true,
        _ => return Err("LOG_LEVEL must be INFO or DEBUG".into()),
    };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .json()
                .with_filter(filter_fn(move |metadata| {
                    log_enabled(metadata.level(), debug)
                }))
                .with_filter(EnvFilter::new(if debug { "debug" } else { "info" })),
        )
        .init();
    let bind: SocketAddr = env::var("API_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()?;
    let admin_bind: SocketAddr = env::var("ADMIN_BIND")
        .unwrap_or_else(|_| "127.0.0.1:9090".into())
        .parse()?;
    let metrics = Metrics::new()?;
    let endpoint_config = std::sync::Arc::new(device_api::device_status::Config::from_env()?);
    let vision_config = std::sync::Arc::new(device_api::device_traffic::VisionConfig::from_env()?);
    let database = device_api::database::connect().await?;
    tracing::info!("database connected");
    let (status_publisher, status_worker) = device_api::device_status::queue::start(
        database.clone(),
        device_api::device_status::queue::Config::from_env()?,
    )
    .await?;
    let (attendance_publisher, attendance_worker) = device_api::device_attendance::queue::start(
        database.clone(),
        device_api::device_attendance::queue::Config::from_env()?,
    )
    .await?;
    let (traffic_attendance_publisher, traffic_attendance_worker) =
        device_api::traffic_attendance::queue::start(
            database.clone(),
            device_api::traffic_attendance::queue::Config::from_env()?,
        )
        .await?;
    let (access_publisher, access_worker) = device_api::device_access::queue::start(
        database.clone(), device_api::device_access::queue::Config::from_env()?,
    ).await?;
    let api = tokio::net::TcpListener::bind(bind).await?;
    let admin = tokio::net::TcpListener::bind(admin_bind).await?;
    tracing::info!(%bind, %admin_bind, "service started");
    let result = tokio::try_join!(
        async {
            axum::serve(
                api,
                router(metrics.clone())
                    .layer(axum::Extension(database.clone()))
                    .layer(axum::Extension(status_publisher))
                    .layer(axum::Extension(access_publisher))
                    .layer(axum::Extension(attendance_publisher))
                    .layer(axum::Extension(traffic_attendance_publisher))
                    .layer(axum::Extension(endpoint_config))
                    .layer(axum::Extension(vision_config)),
            )
            .with_graceful_shutdown(shutdown())
            .await
        },
        async {
            axum::serve(admin, admin_router(metrics.clone()))
                .with_graceful_shutdown(shutdown())
                .await
        }
    );
    tokio::join!(
        status_worker.shutdown(),
        attendance_worker.shutdown(),
        traffic_attendance_worker.shutdown(),
        access_worker.shutdown()
    );
    result?;
    database.close().await;
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c()
        .await
        .expect("install shutdown handler");
}

fn log_enabled(level: &tracing::Level, debug: bool) -> bool {
    matches!(*level, tracing::Level::INFO | tracing::Level::ERROR)
        || (debug && *level == tracing::Level::DEBUG)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_levels_filter_events() {
        for debug in [false, true] {
            assert!(log_enabled(&tracing::Level::INFO, debug));
            assert!(log_enabled(&tracing::Level::ERROR, debug));
            assert_eq!(log_enabled(&tracing::Level::DEBUG, debug), debug);
            assert!(!log_enabled(&tracing::Level::WARN, debug));
            assert!(!log_enabled(&tracing::Level::TRACE, debug));
        }
    }
}
