//! Durable attendance event queue, using the Java topology.
use super::{parse, store};
use futures_util::StreamExt;
use lapin::{
    options::*, publisher_confirm::Confirmation, types::FieldTable, BasicProperties, Channel,
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
}
impl Config {
    pub fn from_env() -> Result<Self, Error> {
        Ok(Self {
            url: env::var("RABBITMQ_URL")
                .unwrap_or_else(|_| "amqp://guest:guest@127.0.0.1:5672/%2f".into()),
            queue: env::var("DEVICE_ATTENDANCE_QUEUE")
                .unwrap_or_else(|_| "attendanceAPIQueue".into()),
            exchange: env::var("DEVICE_ATTENDANCE_EXCHANGE")
                .unwrap_or_else(|_| "attendanceAPIExchange".into()),
            routing_key: env::var("DEVICE_ATTENDANCE_ROUTING_KEY")
                .unwrap_or_else(|_| "reaxium.devices.attendance.api.#".into()),
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
        let channel = self
            .channel
            .read()
            .await
            .clone()
            .ok_or("RabbitMQ unavailable")?;
        let payload = serde_json::to_vec(body)?;
        tokio::time::timeout(Duration::from_secs(5), async {
            let confirmation = channel
                .basic_publish(
                    &self.config.exchange,
                    &self.config.routing_key,
                    BasicPublishOptions {
                        mandatory: true,
                        ..Default::default()
                    },
                    &payload,
                    BasicProperties::default()
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
    channel.basic_qos(1, BasicQosOptions::default()).await?;
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
    let session = tokio::time::timeout(Duration::from_secs(10), connect(&config)).await??;
    let config = Arc::new(config);
    let publisher = Publisher {
        channel: Arc::new(RwLock::new(Some(session.channel.clone()))),
        config,
    };
    let (stop, receiver) = watch::channel(false);
    let task = tokio::spawn(run(pool, publisher.clone(), session, receiver));
    Ok((publisher, Worker { stop, task }))
}

async fn process(pool: &MySqlPool, data: &[u8]) -> Result<(), Error> {
    let body: Value = serde_json::from_slice(data)?;
    let event = parse(&body)?;
    store(pool, &event).await?;
    Ok(())
}

async fn run(
    pool: MySqlPool,
    publisher: Publisher,
    mut session: Session,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            delivery = session.consumer.next() => {
                match delivery {
                    Some(Ok(delivery)) => {
                        match process(&pool, &delivery.data).await {
                            Ok(()) => {
                                let _ = delivery.ack(BasicAckOptions::default()).await;
                            }
                            Err(error) => {
                                tracing::error!(%error, "Error saving device attendance record; rejecting without requeue");
                                let _ = delivery.nack(BasicNackOptions { multiple: false, requeue: false }).await;
                            }
                        }
                    }
                    _ => {
                        *publisher.channel.write().await = None;
                        let _ = session.connection.close(200, "reconnect").await;
                        loop {
                            tokio::select! {
                                _ = stop.changed() => {
                                    return;
                                },
                                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                            }
                            match tokio::time::timeout(Duration::from_secs(10), connect(&publisher.config)).await {
                                Ok(Ok(next)) => {
                                    *publisher.channel.write().await = Some(next.channel.clone());
                                    session = next;
                                    tracing::info!("RabbitMQ attendance worker reconnected");
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
}
