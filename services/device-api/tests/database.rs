#[tokio::test]
#[ignore = "requires a running MySQL instance configured through .env"]
async fn connects_and_executes_query() {
    dotenvy::dotenv().ok();
    let pool = device_api::database::connect().await.unwrap();
    let result: (i32,) = sqlx::query_as("SELECT 1").fetch_one(&pool).await.unwrap();
    assert_eq!(result.0, 1);
    pool.close().await;
}
