// HTTP gateway fixture exposes per-channel messages for the existing payload assertions.
use serde_json::{json, Value};
pub struct Receiver {
    receiver: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    server: Option<tokio::task::JoinHandle<()>>,
}
impl Receiver {
    pub fn new() -> Self {
        Self {
            receiver: None,
            server: None,
        }
    }
    pub async fn bind(&mut self, _: &str) -> Result<String, std::io::Error> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/dispatch", listener.local_addr()?);
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let app = axum::Router::new().route(
            "/dispatch",
            axum::routing::post(
                move |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<Value>| {
                    let sender = sender.clone();
                    async move {
                        assert!(headers
                            .get("authorization")
                            .unwrap()
                            .to_str()
                            .unwrap()
                            .starts_with("Basic "));
                        let content = &body["ReaxiumParameters"]["WebSocketPusher"]["content"];
                        for channel in content["channels"].as_array().unwrap() {
                            sender
                                .send(json!({"channel": channel, "content": content}).to_string())
                                .unwrap();
                        }
                        axum::Json(json!({"ReaxiumResponse":{"code":0}}))
                    }
                },
            ),
        );
        self.receiver = Some(receiver);
        self.server = Some(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        Ok(url)
    }
    pub async fn recv(&mut self) -> Result<String, &'static str> {
        self.receiver
            .as_mut()
            .unwrap()
            .recv()
            .await
            .ok_or("HTTP dispatcher closed")
    }
}
impl Drop for Receiver {
    fn drop(&mut self) {
        if let Some(server) = &self.server {
            server.abort();
        }
    }
}
