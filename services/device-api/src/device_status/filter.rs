use super::Update;
use chrono::{Duration, NaiveDateTime};

pub(super) struct Filter {
    client: redis::Client,
    connection: Option<redis::aio::MultiplexedConnection>,
    interval: i64,
}

impl Filter {
    pub fn new(url: &str, interval: i64) -> Result<Self, redis::RedisError> {
        Ok(Self {
            client: redis::Client::open(url)?,
            connection: None,
            interval,
        })
    }

    pub async fn allows(&mut self, request: &Update) -> bool {
        match tokio::time::timeout(std::time::Duration::from_secs(2), self.check(request)).await {
            Ok(Ok(allowed)) => allowed,
            error => {
                // Java also fails open when Redis is unavailable.
                tracing::warn!(?error, "device status filter unavailable; allowing update");
                self.connection = None;
                true
            }
        }
    }

    async fn check(&mut self, request: &Update) -> Result<bool, redis::RedisError> {
        if self.connection.is_none() {
            self.connection = Some(self.client.get_multiplexed_async_connection().await?);
        }
        let date = NaiveDateTime::parse_from_str(&request.status_date, "%Y-%m-%d %H:%M:%S")
            .expect("validated request date");
        let cutoff = (date - Duration::seconds(self.interval))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        // Same key/value and 24-hour TTL as Java. Atomic across Rust replicas.
        // Update before persistence, as Java's receiver does.
        redis::Script::new(
            r#"
            local previous = redis.call('GET', KEYS[1])
            if previous and (previous >= ARGV[1] or previous > ARGV[2]) then return 0 end
            redis.call('SET', KEYS[1], ARGV[1], 'EX', 86400)
            return 1
        "#,
        )
        .key(format!("device:last_status_date:{}", request.device_imei))
        .arg(&request.status_date)
        .arg(cutoff)
        .invoke_async(self.connection.as_mut().unwrap())
        .await
    }
}
