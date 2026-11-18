use redis::AsyncCommands;
use sqlx::MySqlPool;
use super::Error;

pub(super) struct Cache { client: redis::Client }
impl Cache {
    pub fn new(url: &str) -> Result<Self, Error> { Ok(Self {client: redis::Client::open(url)?}) }
    async fn get(&self, key: &str) -> Option<String> {
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            let mut connection = self.client.get_multiplexed_async_connection().await.ok()?;
            connection.get(key).await.ok()
        }).await.ok().flatten()
    }
    async fn put(&self, key: &str, value: String) {
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            let mut connection = self.client.get_multiplexed_async_connection().await?;
            connection.set_ex::<_,_,()>(key, value, 14400).await
        }).await;
        if !matches!(result, Ok(Ok(()))) { tracing::warn!("device access Redis write failed; database result retained"); }
    }
    pub async fn credentials(&self, pool: &MySqlPool, user: Option<i32>, method: Option<i32>) -> Result<Option<i64>, sqlx::Error> {
        let (Some(user),Some(method)) = (user,method) else {return Ok(None)};
        let key = format!("device:user_creds:{user}:{method}");
        if let Some(value) = self.get(&key).await {
            if value == "NULL" {return Ok(None)}
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&value) {
                if let Some(id) = value["userAccessDataId"].as_i64() { return Ok(Some(id)); }
            }
        }
        let id_key = format!("rust:device:user_creds_id:{user}:{method}");
        if let Some(value) = self.get(&id_key).await {
            if let Ok(id) = value.parse() {return Ok(Some(id))}
        }
        let id: Option<i64> = sqlx::query_scalar("SELECT CAST(user_access_data_id AS SIGNED) FROM user_access_data WHERE user_id=? AND access_type_id=? LIMIT 1").bind(user).bind(method).fetch_optional(pool).await?;
        // Match Java's cached object field naming; it only needs the ID here.
        // Do not publish incomplete credential objects to Java's shared cache.
        // Negative entries are compatible with Java and have the same TTL.
        match id {
            Some(id) => self.put(&id_key, id.to_string()).await,
            None => self.put(&key, "NULL".into()).await,
        }
        Ok(id)
    }
    pub async fn bus(&self, pool: &MySqlPool, route: Option<i32>) -> Result<Option<String>, sqlx::Error> {
        let Some(route) = route else {return Ok(None)};
        let key = format!("device:route_bus:{route}");
        if let Some(value) = self.get(&key).await {return Ok((value!="NULL").then_some(value))}
        let bus: Option<String> = sqlx::query_scalar("SELECT CAST(b.bus_number AS CHAR) FROM bus_routes_relationship rel INNER JOIN bus b ON b.bus_id=rel.bus_id WHERE rel.id_route=? ORDER BY rel.last_update DESC LIMIT 1").bind(route).fetch_optional(pool).await?.flatten();
        self.put(&key, bus.clone().unwrap_or_else(|| "NULL".into())).await;
        Ok(bus)
    }
}
