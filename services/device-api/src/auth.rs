//! Cloud database login and strict HS256 bearer-token validation.
use crate::device_status::Config;
use axum::{
    extract::rejection::JsonRejection,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

const ISSUER: &str = "driver_app_api";
const AUDIENCE: &str = "driver_app_api";
pub const TOKEN_LIFETIME_SECONDS: u64 = 3600;

#[derive(Clone, Serialize, Deserialize)]
struct Claims {
    sub: String,
    iss: String,
    aud: String,
    iat: u64,
    exp: u64,
}

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
}

/// Issue a token for a database user after successful password verification.
pub fn issue_token(config: &Config, subject: &str) -> Result<String, jsonwebtoken::errors::Error> {
    let now = jsonwebtoken::get_current_timestamp();
    encode(
        &Header::new(Algorithm::HS256),
        &Claims {
            sub: subject.into(),
            iss: ISSUER.into(),
            aud: AUDIENCE.into(),
            iat: now,
            exp: now + TOKEN_LIFETIME_SECONDS,
        },
        &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
    )
}

pub(crate) fn authorized(config: &Config, headers: &HeaderMap) -> bool {
    if config.jwt_secret.len() < 32 {
        return false;
    }
    let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some((scheme, token)) = value.split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("bearer") {
        return false;
    }
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[AUDIENCE]);
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        &validation,
    )
    .map(|data| {
        !data.claims.sub.is_empty()
            && data.claims.exp > jsonwebtoken::get_current_timestamp()
            && data.claims.iat <= jsonwebtoken::get_current_timestamp()
    })
    .unwrap_or(false)
}

/// Verify PHP PASSWORD_BCRYPT hashes, including the $2y$ prefix.
fn password_matches(password: &str, hash: &str) -> bool {
    !password.is_empty() && bcrypt::verify(password, hash.trim()).unwrap_or(false)
}

async fn authenticate(
    pool: &sqlx::MySqlPool,
    credentials: Credentials,
) -> Result<Option<String>, sqlx::Error> {
    if credentials.username.is_empty() || credentials.password.is_empty() {
        return Ok(None);
    }
    let rows: Vec<(i64, Option<String>)> = sqlx::query_as(
        "SELECT user_id, user_hash_password FROM user_access_data WHERE user_login_name = ? AND access_type_id = 1 AND status_id = 1"
    ).bind(&credentials.username).fetch_all(pool).await?;
    // bcrypt is CPU intensive; keep it off the async executor.
    tokio::task::spawn_blocking(move || {
        rows.into_iter().find_map(|(id, hash)| {
            hash.filter(|hash| password_matches(&credentials.password, hash))
                .map(|_| id.to_string())
        })
    })
    .await
    .map_err(|_| sqlx::Error::Protocol("password verification worker failed".into()))
}

pub async fn token(
    Extension(config): Extension<Arc<Config>>,
    pool: Option<Extension<sqlx::MySqlPool>>,
    body: Result<Json<Credentials>, JsonRejection>,
) -> Response {
    let result = match body {
        Err(_) => (StatusCode::BAD_REQUEST, json!({"error": "invalid_request"})),
        Ok(_) if config.jwt_secret.len() < 32 => (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"error": "authentication_unavailable"}),
        ),
        Ok(Json(credentials)) => {
            let authenticated = match pool {
                Some(Extension(pool)) => authenticate(&pool, credentials).await,
                None => Err(sqlx::Error::PoolClosed),
            };
            match authenticated {
                Ok(Some(subject)) => match issue_token(&config, &subject) {
                    Ok(token) => (
                        StatusCode::OK,
                        json!({"access_token": token, "token_type": "Bearer", "expires_in": TOKEN_LIFETIME_SECONDS}),
                    ),
                    Err(_) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        json!({"error": "token_creation_failed"}),
                    ),
                },
                Ok(None) => (
                    StatusCode::UNAUTHORIZED,
                    json!({"error": "invalid_credentials"}),
                ),
                Err(_) => {
                    tracing::error!("authentication database lookup or verification unavailable");
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        json!({"error": "authentication_unavailable"}),
                    )
                }
            }
        }
    };
    (
        result.0,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::PRAGMA, "no-cache"),
        ],
        Json(result.1),
    )
        .into_response()
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};

    use tower::ServiceExt;

    fn config() -> Config {
        Config {
            jwt_secret: "test-signing-secret-at-least-32-bytes".into(),
            dispatcher_token: "test-dispatcher-token".into(),
            broadcast_enabled: false,
            broadcast_endpoint: String::new(),
            broadcast_timeout: std::time::Duration::from_secs(1),
        }
    }
    fn accepts(config: &Config, value: &str) -> bool {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, value.parse().unwrap());
        authorized(config, &headers)
    }
    #[tokio::test]
    async fn login_never_falls_back_to_environment_credentials() {
        let app = crate::router(crate::telemetry::Metrics::new().unwrap())
            .layer(Extension(Arc::new(config())));
        for (body, status) in [
            (
                r#"{"username":"api","password":"current:password"}"#,
                StatusCode::SERVICE_UNAVAILABLE,
            ),
            ("{}", StatusCode::BAD_REQUEST),
            ("{", StatusCode::BAD_REQUEST),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::post("/api/auth/token")
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
    }
    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL pointing to a disposable MySQL database"]
    async fn cloud_database_login() {
        let pool = sqlx::mysql::MySqlPoolOptions::new()
            .max_connections(1)
            .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
        sqlx::query("CREATE TEMPORARY TABLE user_access_data (user_id BIGINT, user_login_name VARCHAR(255), user_hash_password VARCHAR(255), access_type_id INT, status_id INT)")
            .execute(&pool).await.unwrap();
        let hash = "$2y$10$EmXR4xQ7A7rIelsn3DN5vu1VHJIITtFmffyL7Ilz6zWhfxcl.QFdq";
        for (name, hash, kind, status) in [
            ("active", hash, 1, 1),
            ("inactive", hash, 1, 2),
            ("card", hash, 2, 1),
            ("broken", "bad hash", 1, 1),
        ] {
            sqlx::query("INSERT INTO user_access_data VALUES (42, ?, ?, ?, ?)")
                .bind(name)
                .bind(hash)
                .bind(kind)
                .bind(status)
                .execute(&pool)
                .await
                .unwrap();
        }
        let app = crate::router(crate::telemetry::Metrics::new().unwrap())
            .layer(Extension(Arc::new(config())))
            .layer(Extension(pool));
        for (user, password, expected) in [
            ("active", "password", StatusCode::OK),
            ("active", "wrong", StatusCode::UNAUTHORIZED),
            ("inactive", "password", StatusCode::UNAUTHORIZED),
            ("card", "password", StatusCode::UNAUTHORIZED),
            ("broken", "password", StatusCode::UNAUTHORIZED),
            ("missing", "password", StatusCode::UNAUTHORIZED),
            ("active' OR 1=1 --", "password", StatusCode::UNAUTHORIZED),
            ("api", "current:password", StatusCode::UNAUTHORIZED),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::post("/api/auth/token")
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({"username":user,"password":password}).to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{user}");
        }
    }
    #[test]
    fn php_bcrypt_compatibility() {
        // PHP password_hash('password', PASSWORD_BCRYPT) compatible fixture.
        let hash = "$2y$10$EmXR4xQ7A7rIelsn3DN5vu1VHJIITtFmffyL7Ilz6zWhfxcl.QFdq";
        assert!(password_matches("password", hash));
        assert!(password_matches("password", &hash.replace("$2y$", "$2a$")));
        assert!(!password_matches("PASSWORD", hash));
        assert!(!password_matches("wrong", hash));
        assert!(!password_matches("", hash));
        assert!(!password_matches("password", "invalid"));
    }
    #[test]
    fn rejects_basic_tampered_expired_and_wrong_claims() {
        let config = config();
        assert!(!accepts(&config, "Basic YXBpOmN1cnJlbnQ6cGFzc3dvcmQ="));
        assert!(!accepts(&config, "Bearer invalid"));
        let token = issue_token(&config, "42").unwrap();
        assert!(accepts(&config, &format!("bearer {token}")));
        assert!(!accepts(&config, &format!("Bearer {token}x")));
        let mut other = config.clone();
        other.jwt_secret.push('x');
        assert!(!accepts(&other, &format!("Bearer {token}")));
        let now = jsonwebtoken::get_current_timestamp();
        let good = Claims {
            sub: "42".into(),
            iss: ISSUER.into(),
            aud: AUDIENCE.into(),
            iat: now,
            exp: now + 3600,
        };
        for variant in 0..6 {
            let mut claims = good.clone();
            match variant {
                0 => claims.exp = now - 1,
                1 => claims.exp = now,
                2 => claims.iss = "other".into(),
                3 => claims.aud = "other".into(),
                4 => claims.sub = String::new(),
                _ => claims.iat = now + 3600,
            }
            let token = encode(
                &Header::new(Algorithm::HS256),
                &claims,
                &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
            )
            .unwrap();
            assert!(!accepts(&config, &format!("Bearer {token}")));
        }
        let token = encode(
            &Header::new(Algorithm::HS384),
            &good,
            &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
        )
        .unwrap();
        assert!(!accepts(&config, &format!("Bearer {token}")));
    }
}
