use device_api::device_status::Config;
use std::time::Duration;

pub fn support_token(username: &str, _password: &str) -> String {
    let config = Config {
        jwt_secret: "test-signing-secret-at-least-32-bytes".into(),
        username: username.into(),
        password: String::new(),
        previous_password: String::new(),
        broadcast_enabled: false,
        broadcast_endpoint: String::new(),
        broadcast_timeout: Duration::from_secs(1),
    };
    format!("Bearer {}", device_api::auth::issue_token(&config).unwrap())
}
