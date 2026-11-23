mod nvml;
mod sysinfo;

use std::sync::Arc;

use crate::api::{HardwareSnapshot, InfoSection, InfoSource, ProviderCapabilities, ProviderError};
use crate::model::{GpuInfo, InfoWarning};

pub use sysinfo::SysinfoInfoSource;

use nvml::NvmlInfoSource;

/// Probed system and hardware providers used by the executable.
pub struct MultimeterInfoSource {
    sysinfo: SysinfoInfoSource,
    nvml: NvmlInfoSource,
}

impl MultimeterInfoSource {
    pub fn probe() -> Self {
        Self {
            sysinfo: SysinfoInfoSource::probe(),
            nvml: NvmlInfoSource::probe(),
        }
    }

    pub fn into_shared(self) -> Arc<Self> {
        Arc::new(self)
    }
}

impl InfoSource for MultimeterInfoSource {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            sysinfo: self.sysinfo.status(),
            nvml: self.nvml.status(),
        }
    }

    fn snapshot(&self, sections: &[InfoSection]) -> Result<HardwareSnapshot, ProviderError> {
        let mut snapshot = self.sysinfo.snapshot(sections)?;
        if sections.contains(&InfoSection::Gpus) {
            let (gpus, warnings) = self.nvml.gpus();
            snapshot.gpus = gpus;
            snapshot.warnings.extend(warnings);
        }
        Ok(snapshot)
    }
}

fn gpu_warning(code: &str, message: impl Into<String>) -> InfoWarning {
    InfoWarning {
        code: code.to_owned(),
        message: message.into(),
        provider: Some("nvml".to_owned()),
        section: Some(InfoSection::Gpus),
    }
}

fn unavailable_gpus() -> Option<Vec<GpuInfo>> {
    None
}

#[cfg(test)]
mod tests {
    use crate::api::{HardwareSnapshot, InfoSection, InfoSource};

    use super::MultimeterInfoSource;

    #[test]
    fn probes_and_collects_requested_system_data() {
        let source = MultimeterInfoSource::probe().into_shared();
        let capabilities = source.capabilities();
        assert!(capabilities.sysinfo.available);
        let query_service = crate::api::QueryService::new(source);

        let snapshot = query_service
            .snapshot(&[InfoSection::Platform, InfoSection::Cpu, InfoSection::Memory])
            .expect("system snapshot should succeed");

        assert!(snapshot.platform.is_some());
        assert!(snapshot.cpu.is_some());
        assert!(snapshot.memory.is_some());
        assert!(snapshot.disks.is_none());
    }

    #[test]
    fn empty_section_list_returns_no_measurements() {
        let source = MultimeterInfoSource::probe();
        assert_eq!(
            source.snapshot(&[]).expect("empty snapshot should succeed"),
            HardwareSnapshot::default()
        );
    }
}
