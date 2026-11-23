use std::error::Error;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use futures_util::SinkExt;
use futures_util::StreamExt;
use futures_util::stream::SplitSink;
use rand::RngCore;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tokio::time::sleep_until;
use tokio::time::timeout;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::accept_hdr_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::ErrorResponse;
use tokio_tungstenite::tungstenite::handshake::server::Request;
use tokio_tungstenite::tungstenite::handshake::server::Response;

use super::protocol::{
    ApiErrorPayload, ClientMessage, InfoQueryParams, InfoSection, MAX_INTERVAL_MS, MIN_INTERVAL_MS,
    PROTOCOL_VERSION, SeekingConfig, SeekingLimits, SeekingResult, SeekingStartParams,
    SeekingStateData, ServerMessage, SessionClosedData, SessionReadyData, SessionState,
    normalize_sections,
};
use super::source::{HardwareSnapshot, InfoSource};

const DEFAULT_AUTHENTICATION_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_INBOUND_MESSAGE_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug)]
pub struct ApiOptions {
    pub allowed_origin: Option<String>,
    pub authentication_timeout: Duration,
}

impl Default for ApiOptions {
    fn default() -> Self {
        Self {
            allowed_origin: None,
            authentication_timeout: DEFAULT_AUTHENTICATION_TIMEOUT,
        }
    }
}

#[derive(Clone, Serialize)]
struct BootstrapPayload<'a> {
    v: u8,
    ws: &'a str,
    token: &'a str,
}

#[derive(Clone)]
pub struct ApiBootstrap {
    websocket_url: String,
    token: String,
}

impl fmt::Debug for ApiBootstrap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiBootstrap")
            .field("websocket_url", &self.websocket_url)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl ApiBootstrap {
    pub fn websocket_url(&self) -> &str {
        &self.websocket_url
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn fragment(&self) -> Result<String, ApiError> {
        let payload = BootstrapPayload {
            v: PROTOCOL_VERSION,
            ws: &self.websocket_url,
            token: &self.token,
        };
        let json = serde_json::to_vec(&payload)?;
        Ok(URL_SAFE_NO_PAD.encode(json))
    }

    pub fn launch_url(&self, frontend_url: &str) -> Result<String, ApiError> {
        let base_url = frontend_url
            .split_once('#')
            .map_or(frontend_url, |(base, _)| base);
        Ok(format!("{base_url}#multimeter={}", self.fragment()?))
    }
}

#[derive(Debug)]
pub enum ApiError {
    Io(std::io::Error),
    Json(serde_json::Error),
    WebSocket(Box<tokio_tungstenite::tungstenite::Error>),
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Json(error) => write!(formatter, "JSON error: {error}"),
            Self::WebSocket(error) => write!(formatter, "WebSocket error: {error}"),
        }
    }
}

impl Error for ApiError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::WebSocket(error) => Some(error.as_ref()),
        }
    }
}

impl From<std::io::Error> for ApiError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for ApiError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::WebSocket(Box::new(error))
    }
}

#[derive(Clone, Debug)]
struct Session {
    state: SessionState,
    config: SeekingConfig,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            state: SessionState::Idle,
            config: SeekingConfig::default(),
        }
    }
}

pub struct ApiServer {
    listener: TcpListener,
    source: Arc<dyn InfoSource>,
    options: ApiOptions,
    token: String,
}

impl ApiServer {
    pub async fn bind(
        source: Arc<dyn InfoSource>,
        options: ApiOptions,
    ) -> Result<(Self, ApiBootstrap), ApiError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;

        let mut token_bytes = [0_u8; 32];
        rand::rng().fill_bytes(&mut token_bytes);
        let token = URL_SAFE_NO_PAD.encode(token_bytes);
        let websocket_url = format!("ws://{address}/v1/ws");

        let bootstrap = ApiBootstrap {
            websocket_url,
            token: token.clone(),
        };
        Ok((
            Self {
                listener,
                source,
                options,
                token,
            },
            bootstrap,
        ))
    }

    pub async fn run(self) -> Result<(), ApiError> {
        let source = self.source;
        let options = Arc::new(self.options);
        let token = Arc::new(self.token);
        let session = Arc::new(Mutex::new(Session::default()));
        let connection_slot = Arc::new(Semaphore::new(1));
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let mut tasks = JoinSet::new();

        loop {
            tokio::select! {
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted?;
                    tasks.spawn(serve_connection(
                        stream,
                        Arc::clone(&source),
                        Arc::clone(&options),
                        Arc::clone(&token),
                        Arc::clone(&session),
                        Arc::clone(&connection_slot),
                        shutdown_tx.clone(),
                    ));
                }
                joined = tasks.join_next(), if !tasks.is_empty() => {
                    if let Some(Err(error)) = joined {
                        eprintln!("multimeter API connection task failed: {error}");
                    }
                }
                changed = shutdown_rx.changed() => {
                    if changed.is_err() || *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
        }

        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                eprintln!("multimeter API connection task failed: {error}");
            }
        }
        Ok(())
    }
}

type ClientSink = SplitSink<WebSocketStream<TcpStream>, Message>;

#[allow(clippy::result_large_err)]
async fn serve_connection(
    stream: TcpStream,
    source: Arc<dyn InfoSource>,
    options: Arc<ApiOptions>,
    token: Arc<String>,
    shared_session: Arc<Mutex<Session>>,
    connection_slot: Arc<Semaphore>,
    shutdown_tx: watch::Sender<bool>,
) -> Result<(), ApiError> {
    let expected_origin = options.allowed_origin.clone();
    let callback = move |request: &Request, response: Response| {
        if request.uri().path() != "/v1/ws" {
            return Err(ErrorResponse::new(Some(
                "websocket path must be /v1/ws".to_owned(),
            )));
        }
        if let Some(expected_origin) = expected_origin.as_deref() {
            let actual_origin = request
                .headers()
                .get("origin")
                .and_then(|value| value.to_str().ok());
            if actual_origin != Some(expected_origin) {
                return Err(ErrorResponse::new(Some(
                    "websocket origin is not allowed".to_owned(),
                )));
            }
        }
        Ok(response)
    };

    let websocket = accept_hdr_async(stream, callback).await?;
    let permit = match Arc::clone(&connection_slot).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            let (mut sink, _socket) = websocket.split();
            let _ = sink.send(Message::Close(None)).await;
            return Ok(());
        }
    };
    let (mut sink, mut socket) = websocket.split();

    let authentication = timeout(options.authentication_timeout, socket.next()).await;
    let authenticate = match authentication {
        Ok(Some(Ok(Message::Text(text)))) if text.len() <= MAX_INBOUND_MESSAGE_BYTES => {
            parse_message(text.as_str())
        }
        _ => None,
    };
    let authenticated = authenticate.is_some_and(|message| {
        message.message_type == "authenticate"
            && message
                .token
                .as_deref()
                .is_some_and(|candidate| constant_time_equal(candidate, token.as_str()))
    });
    if !authenticated {
        let _ = sink.send(Message::Close(None)).await;
        return Ok(());
    }

    let (state, config) = {
        let session = shared_session.lock().await;
        if matches!(session.state, SessionState::Closing) {
            let _ = sink.send(Message::Close(None)).await;
            return Ok(());
        }
        (session.state.clone(), session.config.clone())
    };

    send_message(
        &mut sink,
        ServerMessage::event(
            "session.ready",
            SessionReadyData {
                protocol_version: PROTOCOL_VERSION,
                state,
                providers: source.capabilities(),
                seeking: SeekingLimits::default(),
            },
        )?,
    )
    .await?;

    let result = run_session(
        &mut sink,
        &mut socket,
        source,
        shared_session.clone(),
        shutdown_tx,
        config,
    )
    .await;

    {
        let mut session = shared_session.lock().await;
        if matches!(session.state, SessionState::Seeking) {
            session.state = SessionState::Paused;
        }
    }
    drop(permit);
    result
}

async fn run_session(
    sink: &mut ClientSink,
    socket: &mut futures_util::stream::SplitStream<WebSocketStream<TcpStream>>,
    source: Arc<dyn InfoSource>,
    shared_session: Arc<Mutex<Session>>,
    shutdown_tx: watch::Sender<bool>,
    mut config: SeekingConfig,
) -> Result<(), ApiError> {
    let mut state = shared_session.lock().await.state.clone();
    let mut sequence = 0_u64;
    let mut next_tick: Option<Instant> = None;

    loop {
        let tick_deadline =
            next_tick.unwrap_or_else(|| Instant::now() + Duration::from_secs(86_400));
        tokio::select! {
            incoming = socket.next() => {
                let Some(incoming) = incoming else {
                    break;
                };
                match incoming? {
                    Message::Text(text) => {
                        if text.len() > MAX_INBOUND_MESSAGE_BYTES {
                            send_protocol_error(
                                sink,
                                "request_too_large",
                                "messages must not exceed 16384 bytes",
                            )
                            .await?;
                            continue;
                        }
                        let Some(message) = parse_message(text.as_str()) else {
                            send_message(
                                sink,
                                ServerMessage::event(
                                    "protocol.error",
                                    ApiErrorPayload::new("invalid_json", "message must be valid JSON"),
                                )?,
                            )
                            .await?;
                            continue;
                        };
                        if message.message_type == "authenticate" {
                            if let Some(id) = message.id {
                                send_message(
                                    sink,
                                    ServerMessage::failure(
                                        id,
                                        ApiErrorPayload::new(
                                            "invalid_request",
                                            "the session is already authenticated",
                                        ),
                                    ),
                                )
                                .await?;
                            } else {
                                send_message(
                                    sink,
                                    ServerMessage::event(
                                        "protocol.error",
                                        ApiErrorPayload::new(
                                            "invalid_request",
                                            "the session is already authenticated",
                                        ),
                                    )?,
                                )
                                .await?;
                            }
                            continue;
                        }
                        if message.message_type != "command" {
                            send_protocol_error(
                                sink,
                                "invalid_request",
                                "message type must be command",
                            )
                            .await?;
                            continue;
                        }
                        let Some(id) = message.id.filter(|id| !id.is_empty()) else {
                            send_protocol_error(
                                sink,
                                "invalid_request",
                                "command messages require a non-empty id",
                            )
                            .await?;
                            continue;
                        };
                        let Some(command) = message.command else {
                            send_message(
                                sink,
                                ServerMessage::failure(
                                    id,
                                    ApiErrorPayload::new(
                                        "invalid_request",
                                        "command messages require a command",
                                    ),
                                ),
                            )
                            .await?;
                            continue;
                        };

                        match command.as_str() {
                            "info.query" => {
                                let params: InfoQueryParams =
                                    match parse_params(message.params) {
                                        Ok(params) => params,
                                        Err(error) => {
                                            send_message(
                                                sink,
                                                ServerMessage::failure(id, error),
                                            )
                                            .await?;
                                            continue;
                                        }
                                    };
                                let sections = params
                                    .sections
                                    .map(normalize_sections)
                                    .unwrap_or_else(|| InfoSection::ALL.to_vec());
                                match snapshot_payload(&*source, &sections, &mut sequence) {
                                    Ok(snapshot) => {
                                        send_message(
                                            sink,
                                            ServerMessage::success(
                                                id,
                                                json!({ "snapshot": snapshot }),
                                            )?,
                                        )
                                        .await?;
                                    }
                                    Err(error) => {
                                        send_message(
                                            sink,
                                            ServerMessage::failure(
                                                id,
                                                ApiErrorPayload::new(error.code, error.message),
                                            ),
                                        )
                                        .await?;
                                    }
                                }
                            }
                            "seeking.start" => {
                                let params: SeekingStartParams =
                                    match parse_params(message.params) {
                                        Ok(params) => params,
                                        Err(error) => {
                                            send_message(
                                                sink,
                                                ServerMessage::failure(id, error),
                                            )
                                            .await?;
                                            continue;
                                        }
                                    };
                                let interval_ms = params.interval_ms.unwrap_or(config.interval_ms);
                                if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&interval_ms) {
                                    send_message(
                                        sink,
                                        ServerMessage::failure(
                                            id,
                                            ApiErrorPayload::new(
                                                "invalid_request",
                                                format!(
                                                    "interval_ms must be between {MIN_INTERVAL_MS} and {MAX_INTERVAL_MS}"
                                                ),
                                            ),
                                        ),
                                    )
                                    .await?;
                                    continue;
                                }
                                config.interval_ms = interval_ms;
                                if let Some(sections) = params.sections {
                                    config.sections = normalize_sections(sections);
                                }
                                state = SessionState::Seeking;

                                {
                                    let mut shared = shared_session.lock().await;
                                    shared.state = state.clone();
                                    shared.config = config.clone();
                                }

                                send_message(
                                    sink,
                                    ServerMessage::success(
                                        id,
                                        SeekingResult {
                                            state: state.clone(),
                                            interval_ms: config.interval_ms,
                                            sections: config.sections.clone(),
                                        },
                                    )?,
                                )
                                .await?;
                                send_message(
                                    sink,
                                    ServerMessage::event(
                                        "seeking.state",
                                        SeekingStateData { state: state.clone() },
                                    )?,
                                )
                                .await?;
                                send_snapshot_event(
                                    sink,
                                    &*source,
                                    &config.sections,
                                    &mut sequence,
                                )
                                .await?;
                                next_tick =
                                    Some(Instant::now() + Duration::from_millis(interval_ms));
                            }
                            "seeking.pause" => {
                                if !message.params.is_null() && !message.params.is_object() {
                                    send_message(
                                        sink,
                                        ServerMessage::failure(
                                            id,
                                            ApiErrorPayload::new(
                                                "invalid_request",
                                                "params must be an object or null",
                                            ),
                                        ),
                                    )
                                    .await?;
                                    continue;
                                }
                                if matches!(state, SessionState::Seeking) {
                                    state = SessionState::Paused;
                                }
                                next_tick = None;
                                {
                                    let mut shared = shared_session.lock().await;
                                    shared.state = state.clone();
                                    shared.config = config.clone();
                                }
                                send_message(
                                    sink,
                                    ServerMessage::success(
                                        id,
                                        SeekingResult {
                                            state: state.clone(),
                                            interval_ms: config.interval_ms,
                                            sections: config.sections.clone(),
                                        },
                                    )?,
                                )
                                .await?;
                                send_message(
                                    sink,
                                    ServerMessage::event(
                                        "seeking.state",
                                        SeekingStateData { state: state.clone() },
                                    )?,
                                )
                                .await?;
                            }
                            "app.shutdown" => {
                                if !message.params.is_null() && !message.params.is_object() {
                                    send_message(
                                        sink,
                                        ServerMessage::failure(
                                            id,
                                            ApiErrorPayload::new(
                                                "invalid_request",
                                                "params must be an object or null",
                                            ),
                                        ),
                                    )
                                    .await?;
                                    continue;
                                }
                                state = SessionState::Closing;
                                {
                                    let mut shared = shared_session.lock().await;
                                    shared.state = state.clone();
                                }
                                send_message(
                                    sink,
                                    ServerMessage::success(
                                        id,
                                        SeekingStateData { state: state.clone() },
                                    )?,
                                )
                                .await?;
                                send_message(
                                    sink,
                                    ServerMessage::event(
                                        "seeking.state",
                                        SeekingStateData { state },
                                    )?,
                                )
                                .await?;
                                send_message(
                                    sink,
                                    ServerMessage::event(
                                        "session.closed",
                                        SessionClosedData { reason: "shutdown" },
                                    )?,
                                )
                                .await?;
                                let _ = sink.send(Message::Close(None)).await;
                                let _ = shutdown_tx.send(true);
                                return Ok(());
                            }
                            _ => {
                                send_message(
                                    sink,
                                    ServerMessage::failure(
                                        id,
                                        ApiErrorPayload::new(
                                            "unknown_command",
                                            format!("unknown command: {command}"),
                                        ),
                                    ),
                                )
                                .await?;
                            }
                        }
                    }
                    Message::Ping(payload) => {
                        sink.send(Message::Pong(payload)).await?;
                    }
                    Message::Close(_) => break,
                    Message::Binary(_) => {
                        send_protocol_error(
                            sink,
                            "invalid_request",
                            "binary messages are not supported",
                        )
                        .await?;
                    }
                    Message::Pong(_) | Message::Frame(_) => {}
                }
            }
            _ = sleep_until(tick_deadline), if next_tick.is_some() => {
                next_tick = Some(Instant::now() + Duration::from_millis(config.interval_ms));
                send_snapshot_event(sink, &*source, &config.sections, &mut sequence).await?;
            }
        }
    }
    Ok(())
}

fn snapshot_payload(
    source: &dyn InfoSource,
    sections: &[InfoSection],
    sequence: &mut u64,
) -> Result<SnapshotPayloadOwned, super::source::ProviderError> {
    let hardware = source.snapshot(sections)?;
    *sequence += 1;
    Ok(SnapshotPayloadOwned {
        sequence: *sequence,
        captured_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default(),
        hardware,
    })
}

#[derive(Serialize)]
struct SnapshotPayloadOwned {
    sequence: u64,
    captured_at: String,
    #[serde(flatten)]
    hardware: HardwareSnapshot,
}

async fn send_snapshot_event(
    sink: &mut ClientSink,
    source: &dyn InfoSource,
    sections: &[InfoSection],
    sequence: &mut u64,
) -> Result<(), ApiError> {
    match snapshot_payload(source, sections, sequence) {
        Ok(snapshot) => {
            send_message(sink, ServerMessage::event("info.snapshot", snapshot)?).await?;
        }
        Err(error) => {
            send_message(
                sink,
                ServerMessage::event(
                    "info.error",
                    ApiErrorPayload::new(error.code, error.message),
                )?,
            )
            .await?;
        }
    }
    Ok(())
}

fn parse_message(text: &str) -> Option<ClientMessage> {
    serde_json::from_str(text).ok()
}

fn parse_params<T: DeserializeOwned>(params: Value) -> Result<T, ApiErrorPayload> {
    let params = if params.is_null() { json!({}) } else { params };
    if !params.is_object() {
        return Err(ApiErrorPayload::new(
            "invalid_request",
            "params must be an object or null",
        ));
    }
    serde_json::from_value(params).map_err(|error| {
        ApiErrorPayload::new(
            "invalid_request",
            format!("invalid command parameters: {error}"),
        )
    })
}

async fn send_protocol_error(
    sink: &mut ClientSink,
    code: &str,
    message: &str,
) -> Result<(), ApiError> {
    send_message(
        sink,
        ServerMessage::event("protocol.error", ApiErrorPayload::new(code, message))?,
    )
    .await
}

async fn send_message(sink: &mut ClientSink, message: ServerMessage) -> Result<(), ApiError> {
    sink.send(Message::Text(serde_json::to_string(&message)?.into()))
        .await?;
    Ok(())
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut difference = left.len() ^ right.len();

    for index in 0..right.len() {
        let left_byte = left.get(index).copied().unwrap_or_default();
        let right_byte = right.get(index).copied().unwrap_or_default();
        difference |= usize::from(left_byte ^ right_byte);
    }
    difference == 0
}
