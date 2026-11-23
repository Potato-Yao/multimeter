use std::error::Error;
use std::fmt;

use serde::Serialize;

use super::protocol::{InfoSection, ProviderCapabilities, ProviderStatus};

#[derive(Clone, Debug, Default, Serialize)]
pub struct PlatformInfo {
    pub os_family: String,
    pub os_version: Option<String>,
    pub kernel_version: Option<String>,
    pub architecture: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct CpuInfo {
    pub brand: Option<String>,
    pub physical_cores: Option<u32>,
    pub logical_cores: u32,
    pub usage_percent: f32,
    pub frequency_mhz: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SwapInfo {
    pub total_bytes: u64,
    pub used_bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub filesystem: Option<String>,
    pub kind: Option<String>,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub removable: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct NetworkInfo {
    pub name: String,
    pub received_bytes: u64,
    pub transmitted_bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct GpuInfo {
    pub index: u32,
    pub name: String,
    pub driver_version: Option<String>,
    pub utilization_percent: Option<f32>,
    pub memory_total_bytes: Option<u64>,
    pub memory_used_bytes: Option<u64>,
    pub temperature_celsius: Option<f32>,
    pub power_watts: Option<f32>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ComponentInfo {
    pub label: String,
    pub temperature_celsius: Option<f32>,
    pub max_celsius: Option<f32>,
    pub critical_celsius: Option<f32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InfoWarning {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<InfoSection>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct HardwareSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<PlatformInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<CpuInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub swap: Option<SwapInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disks: Option<Vec<DiskInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub networks: Option<Vec<NetworkInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpus: Option<Vec<GpuInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<ComponentInfo>>,
    pub warnings: Vec<InfoWarning>,
}

#[derive(Clone, Debug)]
pub struct ProviderError {
    pub code: String,
    pub message: String,
}

impl ProviderError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl Error for ProviderError {}

pub trait InfoSource: Send + Sync {
    fn capabilities(&self) -> ProviderCapabilities;

    fn snapshot(&self, sections: &[InfoSection]) -> Result<HardwareSnapshot, ProviderError>;
}

/// Temporary source used until the sysinfo and NVML provider integration lands.
pub struct UnavailableInfoSource;

impl InfoSource for UnavailableInfoSource {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            sysinfo: ProviderStatus::unavailable("provider integration pending"),
            nvml: ProviderStatus::unavailable("provider integration pending"),
        }
    }

    fn snapshot(&self, sections: &[InfoSection]) -> Result<HardwareSnapshot, ProviderError> {
        let warnings = sections
            .iter()
            .map(|section| {
                let provider = if matches!(section, InfoSection::Gpus) {
                    "nvml"
                } else {
                    "sysinfo"
                };
                InfoWarning {
                    code: "provider_unavailable".to_owned(),
                    message: format!(
                        "{provider} is not integrated yet; no data was collected for {section:?}"
                    ),
                    provider: Some(provider.to_owned()),
                    section: Some(*section),
                }
            })
            .collect();

        Ok(HardwareSnapshot {
            warnings,
            ..HardwareSnapshot::default()
        })
    }
}
