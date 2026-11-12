use axum::{
    extract::{rejection::JsonRejection, FromRequest, Request},
    http::header::{HeaderValue, CONTENT_TYPE},
    Json,
};
use serde_json::Value;

/// Volley 1.0.19 adds its body content type even when the app supplies one.
/// Apache can combine those duplicate headers into a comma-separated value.
pub struct RouteJson(pub Value);

impl<S: Send + Sync> FromRequest<S> for RouteJson {
    type Rejection = JsonRejection;

    async fn from_request(mut request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let mut count = 0;
        let all_json = request.headers().get_all(CONTENT_TYPE).iter().all(|value| {
            let Ok(value) = value.to_str() else {
                return false;
            };
            value.split(',').all(|part| {
                count += 1;
                let media_type = part.split(';').next().unwrap_or_default().trim();
                media_type.eq_ignore_ascii_case("application/json")
            })
        });
        // Keep ordinary headers and non-JSON bodies subject to Axum's rules.
        if all_json && count > 1 {
            request
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            tracing::info!("normalized duplicate JSON content types from legacy client");
        }
        // Retain Axum's body size limit and strict JSON parser.
        let Json(value) = Json::<Value>::from_request(request, state).await?;
        Ok(Self(value))
    }
}
