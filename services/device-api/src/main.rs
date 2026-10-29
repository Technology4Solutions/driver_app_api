use device_api::{admin_router, router, telemetry::Metrics};
use std::{env, net::SocketAddr};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=debug".into()),
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
    status_worker.shutdown().await;
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
