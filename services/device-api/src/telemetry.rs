use axum::{
    extract::{MatchedPath, Request, State},
    middleware::Next,
    response::Response,
};
use prometheus::{HistogramOpts, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder};
use std::time::Instant;

#[derive(Clone)]
pub struct Metrics {
    registry: Registry,
    requests: IntCounterVec,
    duration: HistogramVec,
}

impl Metrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();
        let requests = IntCounterVec::new(
            Opts::new("http_requests_total", "Completed HTTP requests"),
            &["route", "method", "status"],
        )?;
        let duration = HistogramVec::new(
            HistogramOpts::new("http_request_duration_seconds", "HTTP request duration"),
            &["route", "method"],
        )?;
        registry.register(Box::new(requests.clone()))?;
        registry.register(Box::new(duration.clone()))?;
        Ok(Self {
            registry,
            requests,
            duration,
        })
    }

    pub fn render(&self) -> Result<String, prometheus::Error> {
        TextEncoder::new().encode_to_string(&self.registry.gather())
    }
}

pub async fn observe(State(metrics): State<Metrics>, request: Request, next: Next) -> Response {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_else(|| "unmatched".into());
    let method = match request.method().as_str() {
        "GET" => "GET",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "HEAD" => "HEAD",
        "OPTIONS" => "OPTIONS",
        _ => "OTHER",
    };
    let started = Instant::now();
    let response = next.run(request).await;
    metrics
        .requests
        .with_label_values(&[route.as_str(), method, response.status().as_str()])
        .inc();
    metrics
        .duration
        .with_label_values(&[route.as_str(), method])
        .observe(started.elapsed().as_secs_f64());
    response
}
