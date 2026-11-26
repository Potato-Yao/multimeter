mod battery;
mod cpu_power;
mod nvml;
mod sysinfo;

use std::sync::Arc;

use crate::api::{HardwareSnapshot, InfoSection, InfoSource, ProviderCapabilities, ProviderError};
use crate::model::{GpuInfo, InfoWarning};

use battery::BatteryInfoSource;
use cpu_power::CpuPowerSource;
pub use sysinfo::SysinfoInfoSource;

use nvml::NvmlInfoSource;

/// Probed system and hardware providers used by the executable.
pub struct MultimeterInfoSource {
    sysinfo: SysinfoInfoSource,
    cpu_power: CpuPowerSource,
    nvml: NvmlInfoSource,
    battery: BatteryInfoSource,
}

impl MultimeterInfoSource {
    pub fn probe() -> Self {
        Self {
            sysinfo: SysinfoInfoSource::probe(),
            cpu_power: CpuPowerSource::probe(),
            nvml: NvmlInfoSource::probe(),
            battery: BatteryInfoSource::new(),
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
            cpu_power: self.cpu_power.status(),
        }
    }

    fn snapshot(&self, sections: &[InfoSection]) -> Result<HardwareSnapshot, ProviderError> {
        let mut snapshot = self.sysinfo.snapshot(sections)?;
        let sample_cpu_power = sections.contains(&InfoSection::Cpu) && snapshot.cpu.is_some();
        let (cpu_power_watts, cpu_power_warning) = self.cpu_power.sample(sample_cpu_power);
        if let Some(cpu) = snapshot.cpu.as_mut() {
            cpu.power_watts = cpu_power_watts;
        }
        if let Some(warning) = cpu_power_warning {
            snapshot.warnings.push(warning);
        }
        if sections.contains(&InfoSection::Gpus) {
            let (gpus, warnings) = self.nvml.gpus();
            snapshot.gpus = gpus;
            snapshot.warnings.extend(warnings);
        }
        if sections.contains(&InfoSection::Batteries) {
            let (batteries, warnings) = self.battery.snapshot();
            snapshot.batteries = Some(batteries);
            snapshot.warnings.extend(warnings);
        }
        Ok(snapshot)
    }

    fn reset_transient_samples(&self) {
        self.cpu_power.reset_transient_samples();
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
            .snapshot(&[
                InfoSection::Platform,
                InfoSection::Cpu,
                InfoSection::Process,
                InfoSection::Memory,
            ])
            .expect("system snapshot should succeed");

        assert!(snapshot.platform.is_some());
        assert!(snapshot.cpu.is_some());
        assert_eq!(
            snapshot
                .process
                .expect("process info should be available")
                .pid,
            std::process::id()
        );
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
