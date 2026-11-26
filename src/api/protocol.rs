use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cpu_test::CpuTestResult;
use crate::gpu_burn::{GpuTestAdapter, GpuTestResult};
use crate::model::InfoSection;

pub const PROTOCOL_VERSION: u8 = 1;
pub const DEFAULT_INTERVAL_MS: u64 = 1_000;
pub const MIN_INTERVAL_MS: u64 = 250;
pub const MAX_INTERVAL_MS: u64 = 60_000;

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CpuTestState {
    Idle,
    Running,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuTestState {
    Idle,
    Running,
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
    pub cpu_power: ProviderStatus,
    pub battery: ProviderStatus,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionReadyData {
    pub protocol_version: u8,
    pub state: SessionState,
    pub cpu_test: CpuTestStateData,
    pub gpu_test: GpuTestStateData,
    pub providers: ProviderCapabilities,
    pub seeking: SeekingLimits,
}

#[derive(Clone, Debug, Serialize)]
pub struct SeekingStateData {
    pub state: SessionState,
}

#[derive(Clone, Debug, Serialize)]
pub struct CpuTestStateData {
    pub state: CpuTestState,
}

#[derive(Clone, Debug, Serialize)]
pub struct GpuTestStateData {
    pub state: GpuTestState,
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
pub struct CpuTestStartResult {
    pub state: CpuTestState,
    pub worker_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct CpuTestMetrics {
    pub worker_count: usize,
    pub batch_size: usize,
    pub elapsed_ms: u64,
    pub multiply_count: u64,
    pub multiplies_per_second: f64,
}

impl From<CpuTestResult> for CpuTestMetrics {
    fn from(result: CpuTestResult) -> Self {
        Self {
            worker_count: result.worker_count,
            batch_size: result.batch_size,
            elapsed_ms: u64::try_from(result.elapsed.as_millis()).unwrap_or(u64::MAX),
            multiply_count: result.multiply_count,
            multiplies_per_second: result.multiplies_per_second,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CpuTestStopResult {
    pub state: CpuTestState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<CpuTestMetrics>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GpuTestAdapterInfo {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
    pub driver_info: String,
}

impl From<&GpuTestAdapter> for GpuTestAdapterInfo {
    fn from(adapter: &GpuTestAdapter) -> Self {
        Self {
            name: adapter.name.clone(),
            backend: adapter.backend.clone(),
            device_type: adapter.device_type.clone(),
            driver: adapter.driver.clone(),
            driver_info: adapter.driver_info.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GpuTestStartResult {
    pub state: GpuTestState,
    pub adapter: GpuTestAdapterInfo,
}

#[derive(Clone, Debug, Serialize)]
pub struct GpuTestMetrics {
    pub adapter: GpuTestAdapterInfo,
    pub elapsed_ms: u64,
    pub dispatch_count: u64,
    pub element_rounds: u64,
}

impl From<GpuTestResult> for GpuTestMetrics {
    fn from(result: GpuTestResult) -> Self {
        Self {
            adapter: GpuTestAdapterInfo::from(&result.adapter),
            elapsed_ms: u64::try_from(result.elapsed.as_millis()).unwrap_or(u64::MAX),
            dispatch_count: result.dispatch_count,
            element_rounds: result.element_rounds,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GpuTestStopResult {
    pub state: GpuTestState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<GpuTestMetrics>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GpuTestFailureData {
    pub message: String,
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
