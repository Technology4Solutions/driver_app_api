use device_api::device_status::Config;
use std::time::Duration;

pub fn support_token(_username: &str, _password: &str) -> String {
    let config = Config {
        jwt_secret: "test-signing-secret-at-least-32-bytes".into(),
        dispatcher_token: "test-dispatcher-token".into(),
        broadcast_enabled: false,
        broadcast_endpoint: String::new(),
        broadcast_timeout: Duration::from_secs(1),
    };
    format!("Bearer {}", device_api::auth::issue_token(&config, "42").unwrap())
}
