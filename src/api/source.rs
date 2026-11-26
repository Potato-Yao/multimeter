use std::error::Error;
use std::fmt;

use crate::model::{HardwareSnapshot, InfoSection, InfoWarning};

use super::protocol::{ProviderCapabilities, ProviderStatus};

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

    fn reset_transient_samples(&self) {}
}

/// Temporary source used until the sysinfo and NVML provider integration lands.
pub struct UnavailableInfoSource;

impl InfoSource for UnavailableInfoSource {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            sysinfo: ProviderStatus::unavailable("provider integration pending"),
            nvml: ProviderStatus::unavailable("provider integration pending"),
            cpu_power: ProviderStatus::unavailable("provider integration pending"),
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
