use sqlx::{mysql::MySqlConnectOptions, mysql::MySqlPoolOptions, MySqlPool};
use std::{env, error::Error, io, time::Duration};

/// Connect before accepting requests so invalid database settings fail at startup.
pub async fn connect() -> Result<MySqlPool, Box<dyn Error + Send + Sync>> {
    let port: u16 = required("DB_PORT")?.parse()?;
    let max_connections: u32 = required("DB_MAX_CONNECTIONS")?.parse()?;
    let timeout: u64 = required("DB_CONNECT_TIMEOUT_SECONDS")?.parse()?;
    if port == 0 || max_connections == 0 || timeout == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "DB_PORT, DB_MAX_CONNECTIONS, and DB_CONNECT_TIMEOUT_SECONDS must be positive",
        )
        .into());
    }
    let options = MySqlConnectOptions::new()
        .host(&required("DB_HOST")?)
        .port(port)
        .database(&required("DB_NAME")?)
        .username(&required("DB_USER")?)
        .password(&required("DB_PASSWORD")?);

    tracing::info!(
        db_host = options.get_host(),
        db_port = options.get_port(),
        db_name = options.get_database(),
        "connecting to configured MySQL database"
    );
    Ok(MySqlPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(timeout))
        .connect_with(options)
        .await?)
}

fn required(name: &str) -> Result<String, io::Error> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name} must be set in .env or the process environment"),
            )
        })
}
