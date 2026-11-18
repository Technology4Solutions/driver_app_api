//! Java-compatible bulk access worker with persistent retry messages and manual acknowledgements.
use super::{broadcast, cache::Cache, model, repository};
use crate::device_status::http_broadcast::Broadcaster;
use futures_util::StreamExt;
use lapin::{
    options::*, publisher_confirm::Confirmation, types::{FieldTable, AMQPValue}, BasicProperties, Channel,
    Connection, ConnectionProperties, Consumer, ExchangeKind,
};
use serde_json::Value;
use sqlx::MySqlPool;
use std::{env, sync::Arc, time::Duration};
use tokio::sync::{watch, RwLock};

type Error = Box<dyn std::error::Error + Send + Sync>;

#[derive(Clone)]
pub struct Config {
    pub url: String,
    pub queue: String,
    pub exchange: String,
    pub routing_key: String,
    pub redis_url: String,
}
impl Config {
    pub fn from_env() -> Result<Self, Error> {
        Ok(Self {
            url: env::var("RABBITMQ_URL")
                .unwrap_or_else(|_| "amqp://guest:guest@127.0.0.1:5672/%2f".into()),
            queue: env::var("DEVICE_ACCESS_QUEUE")
                .unwrap_or_else(|_| "devicesAccessAPIQueue".into()),
            exchange: env::var("DEVICE_ACCESS_EXCHANGE")
                .unwrap_or_else(|_| "devicesAccessAPIExchange".into()),
            redis_url: env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/".into()),
            routing_key: env::var("DEVICE_ACCESS_ROUTING_KEY")
                .unwrap_or_else(|_| "reaxium.devices.access.api.#".into()),
        })
    }
}

#[derive(Clone)]
pub struct Publisher {
    channel: Arc<RwLock<Option<Channel>>>,
    config: Arc<Config>,
}
impl Publisher {
    pub async fn publish(&self, body: &Value) -> Result<(), Error> {
        let payload = serde_json::to_vec(body)?;
        self.send(&self.config.exchange, &self.config.routing_key, &payload, BasicProperties::default()).await
    }
    async fn send(&self, exchange: &str, key: &str, payload: &[u8], properties: BasicProperties) -> Result<(), Error> {
        let channel = self
            .channel
            .read()
            .await
            .clone()
            .ok_or("RabbitMQ unavailable")?;
        tokio::time::timeout(Duration::from_secs(5), async {
            let confirmation = channel
                .basic_publish(
                    exchange,
                    key,
                    BasicPublishOptions {
                        mandatory: true,
                        ..Default::default()
                    },
                    payload,
                    properties
                        .with_content_type("application/json".into())
                        .with_delivery_mode(2),
                )
                .await?
                .await?;
            match confirmation {
                Confirmation::Ack(None) => Ok(()),
                _ => Err("RabbitMQ did not confirm a routed message".into()),
            }
        })
        .await?
    }
}

pub struct Worker {
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}
impl Worker {
    pub async fn shutdown(mut self) {
        let _ = self.stop.send(true);
        if tokio::time::timeout(Duration::from_secs(15), &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
            let _ = self.task.await;
        }
    }
}

struct Session {
    connection: Connection,
    channel: Channel,
    consumer: Consumer,
}
async fn connect(config: &Config) -> Result<Session, Error> {
    let connection = Connection::connect(&config.url, ConnectionProperties::default()).await?;
    let channel = connection.create_channel().await?;
    channel
        .exchange_declare(
            &config.exchange,
            ExchangeKind::Topic,
            ExchangeDeclareOptions {
                durable: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await?;
    channel
        .queue_declare(
            &config.queue,
            QueueDeclareOptions {
                durable: true,
                ..Default::default()
            },
            FieldTable::default(),
        )
        .await?;
    channel
        .queue_bind(
            &config.queue,
            &config.exchange,
            &config.routing_key,
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await?;
    let publisher = connection.create_channel().await?;
    publisher
        .confirm_select(ConfirmSelectOptions::default())
        .await?;
    channel.basic_qos(5, BasicQosOptions::default()).await?;
    let consumer = channel
        .basic_consume(
            &config.queue,
            "",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await?;
    Ok(Session {
        connection,
        channel: publisher,
        consumer,
    })
}

pub async fn start(pool: MySqlPool, config: Config) -> Result<(Publisher, Worker), Error> {
    let cache = Cache::new(&config.redis_url)?;
    let broadcaster = Broadcaster::from_env()?;
    let session = tokio::time::timeout(Duration::from_secs(10), connect(&config)).await??;
    let config = Arc::new(config);
    let publisher = Publisher {
        channel: Arc::new(RwLock::new(Some(session.channel.clone()))),
        config,
    };
    let (stop, receiver) = watch::channel(false);
    let task = tokio::spawn(run(pool, publisher.clone(), session, cache, broadcaster, receiver));
    Ok((publisher, Worker { stop, task }))
}

fn retry_count(properties: &BasicProperties) -> u32 {
    properties.headers().as_ref().and_then(|h| h.inner().get("x-retry-count")).and_then(|v| match v {
        AMQPValue::LongInt(n) if *n >= 0 => Some(*n as u32),
        AMQPValue::LongLongInt(n) if *n >= 0 => Some((*n).min(u32::MAX as i64) as u32),
        _ => None,
    }).unwrap_or(0)
}
fn integrity_error(error: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    matches!(error.downcast_ref::<sqlx::Error>(), Some(sqlx::Error::Database(db)) if db.code().is_some_and(|code| (code.starts_with("23") || code.starts_with("22")) || matches!(code.as_ref(), "1062" | "1452" | "1048" | "1406")))
}
async fn process(pool: &MySqlPool, cache: &Cache, data: &[u8]) -> Result<Vec<u64>, Error> {
    let body: Value = serde_json::from_slice(data)?;
    let events = if body["abulk"].as_bool().unwrap_or(false) {
        model::parse(&body).map_err(|e| -> Error {e.into()})?
    } else {
        // Java's single-access publisher shares this queue.
        vec![serde_json::from_value(body["ReaxiumParameters"]["UserAccess"].clone())?]
    };
    repository::store(pool, cache, events).await
}

async fn run(
    pool: MySqlPool,
    publisher: Publisher,
    mut session: Session,
    cache: Cache,
    broadcaster: Broadcaster,
    mut stop: watch::Receiver<bool>,
) {
    let mut broadcasts = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            _ = broadcasts.join_next(), if !broadcasts.is_empty() => {},
            delivery = session.consumer.next() => {
                match delivery {
                    Some(Ok(delivery)) => {
                        match process(&pool, &cache, &delivery.data).await {
                            Ok(saved) => {
                                // Commit happened before enrichment/HTTP. Dispatch errors
                                // must never retry a successfully committed batch.
                                if broadcaster.enabled {
                                    for id in saved {
                                        match broadcast::payload(&pool, id).await {
                                            Ok(Some(payload)) => {
                                                while broadcasts.len() >= 32 { broadcasts.join_next().await; }
                                                let broadcaster = broadcaster.clone();
                                                broadcasts.spawn(async move {broadcaster.send(payload).await;});
                                            }
                                            Ok(None) => {},
                                            Err(error) => tracing::error!(%error,traffic_id=id,"bulk access enrichment failed after commit"),
                                        }
                                    }
                                }
                                let _ = delivery.ack(BasicAckOptions::default()).await;
                            }
                            Err(error) => {
                                if integrity_error(error.as_ref()) {
                                    tracing::warn!(%error,"bulk access integrity/duplicate failure; rejecting without retry");
                                    let _ = delivery.nack(BasicNackOptions {multiple:false,requeue:false}).await;
                                    continue;
                                }
                                let retry = retry_count(&delivery.properties);
                                if retry < 3 {
                                    let mut headers = delivery.properties.headers().clone().unwrap_or_default();
                                    headers.insert("x-retry-count".into(), AMQPValue::LongInt((retry+1) as i32));
                                    let properties = delivery.properties.clone().with_headers(headers);
                                    match publisher.send("", &publisher.config.queue, &delivery.data, properties).await {
                                        Ok(()) => {
                                            tracing::warn!(retry=retry+1,%error,"bulk access retry published");
                                            let _ = delivery.ack(BasicAckOptions::default()).await;
                                        }
                                        Err(publish_error) => {
                                            tracing::error!(%publish_error,"bulk access retry publish failed; preserving original message");
                                            let _ = delivery.nack(BasicNackOptions {multiple:false,requeue:true}).await;
                                            tokio::time::sleep(Duration::from_millis(250)).await;
                                        }
                                    }
                                } else {
                                    tracing::error!(%error,retries=retry,"bulk access retries exhausted; saving error record");
                                    let body = String::from_utf8_lossy(&delivery.data);
                                    match repository::record_failure(&pool, &body, &error.to_string()).await {
                                        Ok(()) => {let _ = delivery.ack(BasicAckOptions::default()).await;}
                                        Err(log_error) => {
                                            tracing::error!(%log_error,"bulk error-record write failed; preserving original message");
                                            let _ = delivery.nack(BasicNackOptions {multiple:false,requeue:true}).await;
                                            tokio::time::sleep(Duration::from_secs(1)).await;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {
                        *publisher.channel.write().await = None;
                        let _ = session.connection.close(200, "reconnect").await;
                        loop {
                            tokio::select! {
                                _ = stop.changed() => {
                                    while broadcasts.join_next().await.is_some() {}
                                    return;
                                },
                                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                            }
                            match tokio::time::timeout(Duration::from_secs(10), connect(&publisher.config)).await {
                                Ok(Ok(next)) => {
                                    *publisher.channel.write().await = Some(next.channel.clone());
                                    session = next;
                                    tracing::info!("RabbitMQ device access worker reconnected");
                                    break;
                                }
                                _ => tracing::warn!("RabbitMQ reconnect failed; retrying"),
                            }
                        }
                    }
                }
            }
        }
    }
    *publisher.channel.write().await = None;
    let _ = session.connection.close(200, "shutdown").await;
    while broadcasts.join_next().await.is_some() {}
}
