use axum::{body::Body, http::{Request, StatusCode}, Extension, Router};
use device_api::{device_access::queue, device_status::Config, router, telemetry::Metrics};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::MySqlPool;
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;

fn config() -> Config {
    Config {jwt_secret: "test-signing-secret-at-least-32-bytes".into(),
        username:"test".into(),password:"fixture-password".into(),previous_password:String::new(),broadcast_enabled:false,broadcast_endpoint:String::new(),broadcast_timeout:Duration::from_secs(1)}
}
fn app() -> Router {router(Metrics::new().unwrap()).layer(Extension(Arc::new(config())))}
async fn post(app: Router, path: &str, body: &str, auth: bool) -> (StatusCode, Value) {
    let mut request = Request::post(path).header("content-type","application/json");
    if auth {request=request.header("authorization",format!("Bearer {}",device_api::auth::issue_token(&config()).unwrap()));}
    let response=app.oneshot(request.body(Body::from(body.to_owned())).unwrap()).await.unwrap();
    let status=response.status();
    (status,serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap())
}
fn sample() -> Value {
    json!({"user_id":1,"device_id":42,"traffic_type":1,"access_type":3,"traffic_info":"Boarding","trace_id":"bulk-integration","latitude":27.1975,"longitude":-80.2528,"bus_info":"2","accessDate":"1792584000999","stop_id":9,"stop_id_by_distance":10,"route_id":7,"access_situation_id":2})
}
fn batch(events: Vec<Value>) -> Value {json!({"ReaxiumParameters":{"BulkUserAccess":events}})}
#[tokio::test]
async fn aliases_keep_auth_validation_and_unavailable_publisher_contract() {
    for path in ["/api/DeviceAccess/bulkUserAccess","/device/DeviceAccess/bulkUserAccess"] {
        let (status,body)=post(app(),path,"{}",false).await;
        assert_eq!(status,200);assert_eq!(body["ReaxiumResponse"]["code"],3);
        for input in ["{", "{}", r#"{"ReaxiumParameters":{"BulkUserAccess":[]}}"#, r#"{"ReaxiumParameters":{"BulkUserAccess":[{"user_id":"bad"}]}}"#, r#"{"ReaxiumParameters":{"BulkUserAccess":[{}]}}"#] {
            let (status,body)=post(app(),path,input,true).await;
            assert_eq!(status,400);
            assert_eq!(body,json!({"ReaxiumResponse":{"code":500,"message":"Error reporting device access","object":[]}}));
        }
    }
}
async fn wait_count(pool: &MySqlPool, table: &str, expected: i64) {
    let query=match table {"traffic"=>"SELECT COUNT(*) FROM traffic","messages"=>"SELECT COUNT(*) FROM messages",_=>panic!()};
    tokio::time::timeout(Duration::from_secs(15),async {
        loop {
            let count:i64=sqlx::query_scalar(query).fetch_one(pool).await.unwrap();
            if count==expected {break}
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }).await.unwrap();
}
#[tokio::test]
#[ignore="requires disposable TEST_DATABASE_URL, TEST_RABBITMQ_URL, TEST_REDIS_URL; isolated services only"]
async fn bulk_queue_persists_skips_missing_credentials_rolls_back_duplicates_and_records_retries() {
    use sqlx::Row;
    let pool=MySqlPool::connect(&std::env::var("TEST_DATABASE_URL").unwrap()).await.unwrap();
    let name:String=sqlx::query_scalar("SELECT DATABASE()").fetch_one(&pool).await.unwrap();
    assert_eq!(name,"device_api_migration_test","requires disposable test database");
    for statement in include_str!("fixtures/device_access.sql").split(';').map(str::trim).filter(|s|!s.is_empty()) {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }
    let config=queue::Config {
        url:std::env::var("TEST_RABBITMQ_URL").unwrap(),
        redis_url:std::env::var("TEST_REDIS_URL").unwrap(),
        queue:format!("bulk-test-{}",std::process::id()),exchange:format!("bulk-test-{}",std::process::id()),routing_key:"bulk.test".into(),
    };
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint=format!("http://{}/dispatch",listener.local_addr().unwrap());
    let (sent,mut received)=tokio::sync::mpsc::unbounded_channel();
    let observed_pool=pool.clone();
    let gateway=axum::Router::new().route("/dispatch",axum::routing::post(move |headers:axum::http::HeaderMap,axum::Json(body):axum::Json<Value>| {
        let sent=sent.clone();let pool=observed_pool.clone();
        async move {
            assert!(headers.contains_key("authorization"));
            let visible:i64=sqlx::query_scalar("SELECT COUNT(*) FROM traffic").fetch_one(&pool).await.unwrap();
            assert_eq!(visible,1,"publication must happen after commit");
            sent.send(body).unwrap();
            // Even an HTTP delivery failure must never retry committed traffic.
            StatusCode::SERVICE_UNAVAILABLE
        }
    }));
    let gateway=tokio::spawn(async move {axum::serve(listener,gateway).await.unwrap();});
    std::env::set_var("BROADCAST_ENABLED","true");
    std::env::set_var("DEVICES_API_EXPORT_DEVICE_STATUS_URL",endpoint);
    std::env::set_var("REAXIUM_API_USER","test");
    std::env::set_var("REAXIUM_API_PASSWORD","fixture-password");
    let (publisher,worker)=queue::start(pool.clone(),config.clone()).await.unwrap();
    let app=app().layer(Extension(publisher));
    let mut missing=sample();missing["user_id"]=json!(999);
    let (status,body)=post(app.clone(),"/api/DeviceAccess/bulkUserAccess",&batch(vec![missing,sample()]).to_string(),true).await;
    assert_eq!(status,200);assert_eq!(body["ReaxiumResponse"]["message"],"Device Access queued");
    wait_count(&pool,"traffic",1).await;
    let row=sqlx::query("SELECT *,CAST(datetime AS CHAR) AS event_date FROM traffic").fetch_one(&pool).await.unwrap();
    assert_eq!(row.get::<i32,_>("access_id"),100);
    assert_eq!(row.get::<String,_>("assigned_bus"),"ROUTE-BUS");
    assert_eq!(row.get::<String,_>("bus_info"),"2");
    assert_eq!(row.get::<String,_>("stop_number"),"DISTANCE");
    assert_eq!(row.get::<String,_>("event_date"),"2026-10-21 08:00:00");
    let content=tokio::time::timeout(Duration::from_secs(5),received.recv()).await.unwrap().unwrap();
    let content=&content["ReaxiumParameters"]["WebSocketPusher"]["content"];
    assert_eq!(content["traffic_label"],"ACCESS_TRAFFIC");
    assert_eq!(content["stop_number"],"DISTANCE");
    assert_eq!(content["user_id"],1);
    use redis::AsyncCommands;
    let mut redis=redis::Client::open(config.redis_url.clone()).unwrap().get_multiplexed_async_connection().await.unwrap();
    let id:String=redis.get("rust:device:user_creds_id:1:3").await.unwrap();assert_eq!(id,"100");
    let ttl:i64=redis.ttl("rust:device:user_creds_id:1:3").await.unwrap();assert!((14000..=14400).contains(&ttl));
    let missing:String=redis.get("device:user_creds:999:3").await.unwrap();assert_eq!(missing,"NULL");
    // A duplicate after a new event must roll back the entire transaction.
    let mut fresh=sample();fresh["trace_id"]=json!("must-roll-back");
    post(app.clone(),"/device/DeviceAccess/bulkUserAccess",&batch(vec![fresh,sample()]).to_string(),true).await;
    let mut oversized=sample();oversized["traffic_info"]=json!("x".repeat(3000));
    post(app.clone(),"/api/DeviceAccess/bulkUserAccess",&batch(vec![oversized]).to_string(),true).await;
    // Queue is sequential; a following all-invalid batch eventually writes a
    // terminal error record, proving preceding duplicate handling completed.
    let mut invalid=sample();invalid["accessDate"]=json!("invalid");
    post(app,"/api/DeviceAccess/bulkUserAccess",&batch(vec![invalid]).to_string(),true).await;
    wait_count(&pool,"messages",1).await;
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM traffic").fetch_one(&pool).await.unwrap();assert_eq!(count,1);
    let error=sqlx::query("SELECT topic,status,device_id,data FROM messages").fetch_one(&pool).await.unwrap();
    assert_eq!(error.get::<String,_>("topic"),"DEVICE_ACCESS");
    assert_eq!(error.get::<String,_>("status"),"ERROR");
    assert_eq!(error.get::<String,_>("device_id"),"42");
    let failed:Value=serde_json::from_str(&error.get::<String,_>("data")).unwrap();
    assert_eq!(failed["ReaxiumParameters"]["BulkUserAccess"][0]["accessDate"],"invalid", "integrity failures must not enter terminal retry records");
    worker.shutdown().await;
    assert!(received.try_recv().is_err(),"rollback and dispatch failure must not produce extra broadcasts");
    gateway.abort();
    let connection=lapin::Connection::connect(&config.url,lapin::ConnectionProperties::default()).await.unwrap();
    let channel=connection.create_channel().await.unwrap();
    channel.queue_delete(&config.queue,lapin::options::QueueDeleteOptions::default()).await.unwrap();
    channel.exchange_delete(&config.exchange,lapin::options::ExchangeDeleteOptions::default()).await.unwrap();
    connection.close(200,"test complete").await.unwrap();
    pool.close().await;
}
