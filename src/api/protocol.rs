use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u8 = 1;
pub const DEFAULT_INTERVAL_MS: u64 = 1_000;
pub const MIN_INTERVAL_MS: u64 = 250;
pub const MAX_INTERVAL_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InfoSection {
    Platform,
    Cpu,
    Memory,
    Swap,
    Disks,
    Networks,
    Gpus,
    Components,
}

impl InfoSection {
    pub const ALL: [Self; 8] = [
        Self::Platform,
        Self::Cpu,
        Self::Memory,
        Self::Swap,
        Self::Disks,
        Self::Networks,
        Self::Gpus,
        Self::Components,
    ];
}

pub fn normalize_sections(sections: Vec<InfoSection>) -> Vec<InfoSection> {
    let mut normalized = Vec::with_capacity(sections.len());
    for section in sections {
        if !normalized.contains(&section) {
            normalized.push(section);
        }
    }
    normalized
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClientMessage {
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct InfoQueryParams {
    #[serde(default)]
    pub sections: Option<Vec<InfoSection>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SeekingStartParams {
    #[serde(default)]
    pub interval_ms: Option<u64>,
    #[serde(default)]
    pub sections: Option<Vec<InfoSection>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Idle,
    Seeking,
    Paused,
    Closing,
}

#[derive(Clone, Debug)]
pub struct SeekingConfig {
    pub interval_ms: u64,
    pub sections: Vec<InfoSection>,
}

impl Default for SeekingConfig {
    fn default() -> Self {
        Self {
            interval_ms: DEFAULT_INTERVAL_MS,
            sections: InfoSection::ALL.to_vec(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SeekingLimits {
    pub default_interval_ms: u64,
    pub minimum_interval_ms: u64,
    pub maximum_interval_ms: u64,
}

impl Default for SeekingLimits {
    fn default() -> Self {
        Self {
            default_interval_ms: DEFAULT_INTERVAL_MS,
            minimum_interval_ms: MIN_INTERVAL_MS,
            maximum_interval_ms: MAX_INTERVAL_MS,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ProviderStatus {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ProviderStatus {
    pub fn available() -> Self {
        Self {
            available: true,
            reason: None,
        }
    }

    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ProviderCapabilities {
    pub sysinfo: ProviderStatus,
    pub nvml: ProviderStatus,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionReadyData {
    pub protocol_version: u8,
    pub state: SessionState,
    pub providers: ProviderCapabilities,
    pub seeking: SeekingLimits,
}

#[derive(Clone, Debug, Serialize)]
pub struct SeekingStateData {
    pub state: SessionState,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionClosedData {
    pub reason: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct SeekingResult {
    pub state: SessionState,
    pub interval_ms: u64,
    pub sections: Vec<InfoSection>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApiErrorPayload {
    pub code: String,
    pub message: String,
}

impl ApiErrorPayload {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Event {
        event: &'static str,
        data: Value,
    },
    Response {
        id: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<ApiErrorPayload>,
    },
}

impl ServerMessage {
    pub fn event(event: &'static str, data: impl Serialize) -> Result<Self, serde_json::Error> {
        Ok(Self::Event {
            event,
            data: serde_json::to_value(data)?,
        })
    }

    pub fn success(id: String, result: impl Serialize) -> Result<Self, serde_json::Error> {
        Ok(Self::Response {
            id,
            ok: true,
            result: Some(serde_json::to_value(result)?),
            error: None,
        })
    }

    pub fn failure(id: String, error: ApiErrorPayload) -> Self {
        Self::Response {
            id,
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}
