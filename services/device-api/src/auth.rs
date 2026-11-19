//! Shared API login and strict HS256 bearer-token validation.
use crate::device_status::Config;
use axum::{extract::rejection::JsonRejection, http::{header, HeaderMap, StatusCode}, response::{IntoResponse, Response}, Extension, Json};
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

/// Issue a token after the caller has authenticated the API credentials.
pub fn issue_token(config: &Config) -> Result<String, jsonwebtoken::errors::Error> {
    let now = jsonwebtoken::get_current_timestamp();
    encode(&Header::new(Algorithm::HS256), &Claims {
        sub: config.username.clone(), iss: ISSUER.into(), aud: AUDIENCE.into(),
        iat: now, exp: now + TOKEN_LIFETIME_SECONDS,
    }, &EncodingKey::from_secret(config.jwt_secret.as_bytes()))
}

pub(crate) fn authorized(config: &Config, headers: &HeaderMap) -> bool {
    if config.username.is_empty() || config.jwt_secret.len() < 32 {
        return false;
    }
    let Some(value) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) else { return false; };
    let Some((scheme, token)) = value.split_once(' ') else { return false; };
    if !scheme.eq_ignore_ascii_case("bearer") { return false; }
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 0;
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[AUDIENCE]);
    validation.sub = Some(config.username.clone());
    decode::<Claims>(token, &DecodingKey::from_secret(config.jwt_secret.as_bytes()), &validation)
        .map(|data| data.claims.exp > jsonwebtoken::get_current_timestamp() && data.claims.iat <= jsonwebtoken::get_current_timestamp())
        .unwrap_or(false)
}

pub async fn token(
    Extension(config): Extension<Arc<Config>>,
    body: Result<Json<Credentials>, JsonRejection>,
) -> Response {
    let result = match body {
        Err(_) => (StatusCode::BAD_REQUEST, json!({"error": "invalid_request"})),
        Ok(Json(credentials)) if !config.credentials_match(credentials.username.as_bytes(), credentials.password.as_bytes()) =>
            (StatusCode::UNAUTHORIZED, json!({"error": "invalid_credentials"})),
        Ok(_) if config.jwt_secret.len() < 32 =>
            (StatusCode::SERVICE_UNAVAILABLE, json!({"error": "authentication_unavailable"})),
        Ok(_) => match issue_token(&config) {
            Ok(token) => (StatusCode::OK, json!({"access_token": token, "token_type": "Bearer", "expires_in": TOKEN_LIFETIME_SECONDS})),
            Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "token_creation_failed"})),
        },
    };
    (result.0, [(header::CACHE_CONTROL, "no-store"), (header::PRAGMA, "no-cache")], Json(result.1)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn config() -> Config {
        Config {
            jwt_secret: "test-signing-secret-at-least-32-bytes".into(),
            username: "api".into(), password: "current:password".into(), previous_password: "previous".into(),
            broadcast_enabled: false, broadcast_endpoint: String::new(), broadcast_timeout: std::time::Duration::from_secs(1),
        }
    }
    fn accepts(config: &Config, value: &str) -> bool {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, value.parse().unwrap());
        authorized(config, &headers)
    }
    #[tokio::test]
    async fn login_and_bearer_round_trip() {
        let config = config();
        let app = crate::router(crate::telemetry::Metrics::new().unwrap()).layer(Extension(Arc::new(config.clone())));
        for password in ["current:password", "previous", "wrong", ""] {
            let response = app.clone().oneshot(Request::post("/api/auth/token")
                .header("content-type", "application/json")
                .body(Body::from(json!({"username":"api", "password":password}).to_string())).unwrap()).await.unwrap();
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let valid = password == "current:password" || password == "previous";
            assert_eq!(response.status(), if valid { StatusCode::OK } else { StatusCode::UNAUTHORIZED });
            let body: serde_json::Value = serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
            if valid {
                assert_eq!(body["expires_in"], 3600);
                assert!(accepts(&config, &format!("Bearer {}", body["access_token"].as_str().unwrap())));
            } else { assert!(body.get("access_token").is_none()); }
        }
        for body in ["{}", "{", "null"] {
            let response = app.clone().oneshot(Request::post("/api/auth/token").header("content-type", "application/json").body(Body::from(body)).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
    }
    #[test]
    fn rejects_basic_tampered_expired_and_wrong_claims() {
        let config = config();
        assert!(!accepts(&config, "Basic YXBpOmN1cnJlbnQ6cGFzc3dvcmQ="));
        assert!(!accepts(&config, "Bearer invalid"));
        let token = issue_token(&config).unwrap();
        assert!(accepts(&config, &format!("bearer {token}")));
        assert!(!accepts(&config, &format!("Bearer {token}x")));
        let mut other = config.clone();
        other.jwt_secret.push('x');
        assert!(!accepts(&other, &format!("Bearer {token}")));
        let now = jsonwebtoken::get_current_timestamp();
        let good = Claims { sub: config.username.clone(), iss: ISSUER.into(), aud: AUDIENCE.into(), iat: now, exp: now + 3600 };
        for variant in 0..6 {
            let mut claims = good.clone();
            match variant {
                0 => claims.exp = now - 1,
                1 => claims.exp = now,
                2 => claims.iss = "other".into(),
                3 => claims.aud = "other".into(),
                4 => claims.sub = "other".into(),
                _ => claims.iat = now + 3600,
            }
            let token = encode(&Header::new(Algorithm::HS256), &claims, &EncodingKey::from_secret(config.jwt_secret.as_bytes())).unwrap();
            assert!(!accepts(&config, &format!("Bearer {token}")));
        }
        let token = encode(&Header::new(Algorithm::HS384), &good, &EncodingKey::from_secret(config.jwt_secret.as_bytes())).unwrap();
        assert!(!accepts(&config, &format!("Bearer {token}")));
    }
}
