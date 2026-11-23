use std::sync::Arc;
use std::time::Duration;

use futures_util::SinkExt;
use futures_util::StreamExt;
use multimeter::api::{
    ApiBootstrap, ApiOptions, ApiServer, CpuInfo, HardwareSnapshot, InfoSection, InfoSource,
    MemoryInfo, ProviderCapabilities, ProviderError, ProviderStatus,
};
use serde_json::Value;
use serde_json::json;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

struct FakeInfoSource;

impl InfoSource for FakeInfoSource {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            sysinfo: ProviderStatus::available(),
            nvml: ProviderStatus::unavailable("no_supported_gpu"),
        }
    }

    fn snapshot(&self, sections: &[InfoSection]) -> Result<HardwareSnapshot, ProviderError> {
        Ok(HardwareSnapshot {
            cpu: sections.contains(&InfoSection::Cpu).then(|| CpuInfo {
                brand: Some("Example CPU".to_owned()),
                physical_cores: Some(4),
                logical_cores: 8,
                usage_percent: 25.0,
                frequency_mhz: 3_200,
            }),
            memory: sections
                .contains(&InfoSection::Memory)
                .then_some(MemoryInfo {
                    total_bytes: 16 * 1024 * 1024 * 1024,
                    used_bytes: 4 * 1024 * 1024 * 1024,
                    available_bytes: 12 * 1024 * 1024 * 1024,
                }),
            ..HardwareSnapshot::default()
        })
    }
}

type Client = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn spawn_server() -> (
    ApiBootstrap,
    JoinHandle<Result<(), multimeter::api::ApiError>>,
) {
    let (server, bootstrap) = ApiServer::bind(Arc::new(FakeInfoSource), ApiOptions::default())
        .await
        .expect("server should bind");
    let task = tokio::spawn(server.run());
    (bootstrap, task)
}

async fn connect(bootstrap: &ApiBootstrap) -> Client {
    let (mut websocket, _) = connect_async(bootstrap.websocket_url())
        .await
        .expect("websocket should connect");
    send(
        &mut websocket,
        json!({
            "type": "authenticate",
            "token": bootstrap.token()
        }),
    )
    .await;
    websocket
}

async fn send(socket: &mut Client, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .expect("message should send");
}

async fn next_json(socket: &mut Client) -> Value {
    let message = timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("message should arrive before timeout")
        .expect("socket should remain open")
        .expect("message should be valid");
    match message {
        Message::Text(text) => serde_json::from_str(text.as_str()).expect("JSON message"),
        other => panic!("expected text message, got {other:?}"),
    }
}

#[tokio::test]
async fn supports_query_seeking_pause_and_shutdown() {
    let (bootstrap, server_task) = spawn_server().await;
    let launch_url = bootstrap
        .configured_launch_url()
        .expect("configured launch URL should serialize");
    assert!(launch_url.starts_with(multimeter::config::FRONTEND_BASE_URL));
    assert!(launch_url.contains("#multimeter="));

    let mut socket = connect(&bootstrap).await;

    let ready = next_json(&mut socket).await;
    assert_eq!(ready["type"], "event");
    assert_eq!(ready["event"], "session.ready");
    assert_eq!(ready["data"]["state"], "idle");
    assert_eq!(ready["data"]["providers"]["sysinfo"]["available"], true);
    assert_eq!(ready["data"]["providers"]["nvml"]["available"], false);

    send(
        &mut socket,
        json!({
            "type": "command",
            "id": "query-1",
            "command": "info.query",
            "params": {
                "sections": ["cpu", "memory"]
            }
        }),
    )
    .await;
    let query_response = next_json(&mut socket).await;
    assert_eq!(query_response["id"], "query-1");
    assert_eq!(query_response["ok"], true);
    assert_eq!(
        query_response["result"]["snapshot"]["cpu"]["logical_cores"],
        8
    );
    assert!(query_response["result"]["snapshot"]["gpus"].is_null());

    send(
        &mut socket,
        json!({
            "type": "command",
            "id": "start-1",
            "command": "seeking.start",
            "params": {
                "interval_ms": 250,
                "sections": ["cpu"]
            }
        }),
    )
    .await;
    let start_response = next_json(&mut socket).await;
    assert_eq!(start_response["id"], "start-1");
    assert_eq!(start_response["result"]["state"], "seeking");
    assert_eq!(start_response["result"]["interval_ms"], 250);
    assert_eq!(next_json(&mut socket).await["data"]["state"], "seeking");
    assert_eq!(next_json(&mut socket).await["event"], "info.snapshot");

    send(
        &mut socket,
        json!({
            "type": "command",
            "id": "pause-1",
            "command": "seeking.pause",
            "params": {}
        }),
    )
    .await;
    let pause_response = next_json(&mut socket).await;
    assert_eq!(pause_response["id"], "pause-1");
    assert_eq!(pause_response["result"]["state"], "paused");
    assert_eq!(next_json(&mut socket).await["data"]["state"], "paused");
    assert!(
        timeout(Duration::from_millis(400), socket.next())
            .await
            .is_err(),
        "no snapshots should arrive while paused"
    );

    send(
        &mut socket,
        json!({
            "type": "command",
            "id": "query-2",
            "command": "info.query",
            "params": {}
        }),
    )
    .await;
    assert_eq!(next_json(&mut socket).await["id"], "query-2");

    send(
        &mut socket,
        json!({
            "type": "command",
            "id": "shutdown-1",
            "command": "app.shutdown",
            "params": {}
        }),
    )
    .await;
    let shutdown_response = next_json(&mut socket).await;
    assert_eq!(shutdown_response["id"], "shutdown-1");
    assert_eq!(shutdown_response["result"]["state"], "closing");
    assert_eq!(next_json(&mut socket).await["data"]["state"], "closing");
    let closed = next_json(&mut socket).await;
    assert_eq!(closed["event"], "session.closed");
    assert_eq!(closed["data"]["reason"], "shutdown");

    timeout(Duration::from_secs(1), server_task)
        .await
        .expect("server should stop")
        .expect("server task should not panic")
        .expect("server should stop cleanly");
}

#[tokio::test]
async fn rejects_invalid_interval() {
    let (bootstrap, server_task) = spawn_server().await;
    let mut socket = connect(&bootstrap).await;
    assert_eq!(next_json(&mut socket).await["event"], "session.ready");

    send(
        &mut socket,
        json!({
            "type": "command",
            "id": "start-invalid",
            "command": "seeking.start",
            "params": {
                "interval_ms": 10
            }
        }),
    )
    .await;
    let invalid = next_json(&mut socket).await;
    assert_eq!(invalid["ok"], false);
    assert_eq!(invalid["error"]["code"], "invalid_request");

    server_task.abort();
}

#[tokio::test]
async fn rejects_invalid_authentication() {
    let (bootstrap, server_task) = spawn_server().await;
    let (mut invalid_socket, _) = connect_async(bootstrap.websocket_url())
        .await
        .expect("websocket should connect");
    send(
        &mut invalid_socket,
        json!({
            "type": "authenticate",
            "token": "wrong-token"
        }),
    )
    .await;
    let rejection = timeout(Duration::from_secs(1), invalid_socket.next())
        .await
        .expect("authentication should fail promptly")
        .expect("socket should close")
        .expect("close should be a websocket message");
    assert!(matches!(rejection, Message::Close(_)));

    server_task.abort();
}
