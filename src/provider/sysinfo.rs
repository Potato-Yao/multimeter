use std::sync::Mutex;

use crate::api::{
    ComponentInfo, CpuInfo, DiskInfo, HardwareSnapshot, InfoSection, InfoSource, InfoWarning,
    LoadAverage, MemoryInfo, NetworkInfo, ProviderCapabilities, ProviderError, ProviderStatus,
    SwapInfo, SystemInfo,
};

use ::sysinfo::{
    Components, CpuRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System,
};

pub struct SysinfoInfoSource {
    collectors: Mutex<CollectorState>,
    status: ProviderStatus,
    section_failures: Vec<SectionFailure>,
}

enum CollectorState {
    Ready(Box<Collectors>),
    Unavailable(String),
}

struct Collectors {
    system: System,
    physical_cores: Option<u32>,
    disks: Option<Disks>,
    networks: Option<Networks>,
    components: Option<Components>,
}

#[derive(Clone)]
struct SectionFailure {
    section: InfoSection,
    message: String,
}

impl SysinfoInfoSource {
    pub fn probe() -> Self {
        match Collectors::probe() {
            Ok((collectors, section_failures)) => Self {
                collectors: Mutex::new(CollectorState::Ready(Box::new(collectors))),
                status: ProviderStatus::available(),
                section_failures,
            },
            Err(error) => Self {
                collectors: Mutex::new(CollectorState::Unavailable(error.clone())),
                status: ProviderStatus::unavailable(error),
                section_failures: Vec::new(),
            },
        }
    }

    pub fn status(&self) -> ProviderStatus {
        self.status.clone()
    }

    fn unavailable_warnings(&self, sections: &[InfoSection], message: &str) -> Vec<InfoWarning> {
        sections
            .iter()
            .filter(|section| **section != InfoSection::Gpus)
            .map(|section| InfoWarning {
                code: "provider_unavailable".to_owned(),
                message: message.to_owned(),
                provider: Some("sysinfo".to_owned()),
                section: Some(*section),
            })
            .collect()
    }

    fn section_warnings(&self, sections: &[InfoSection]) -> Vec<InfoWarning> {
        self.section_failures
            .iter()
            .filter(|failure| sections.contains(&failure.section))
            .map(|failure| InfoWarning {
                code: "section_unavailable".to_owned(),
                message: failure.message.clone(),
                provider: Some("sysinfo".to_owned()),
                section: Some(failure.section),
            })
            .collect()
    }
}

impl InfoSource for SysinfoInfoSource {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            sysinfo: self.status(),
            nvml: ProviderStatus::unavailable("NVML is provided separately"),
        }
    }

    fn snapshot(&self, sections: &[InfoSection]) -> Result<HardwareSnapshot, ProviderError> {
        let mut snapshot = HardwareSnapshot {
            warnings: self.section_warnings(sections),
            ..HardwareSnapshot::default()
        };
        let mut collectors = self.collectors.lock().map_err(|_| {
            ProviderError::new("collector_poisoned", "sysinfo collector lock failed")
        })?;

        let collectors = match &mut *collectors {
            CollectorState::Ready(collectors) => collectors.as_mut(),
            CollectorState::Unavailable(message) => {
                snapshot
                    .warnings
                    .extend(self.unavailable_warnings(sections, message));
                return Ok(snapshot);
            }
        };

        if sections.contains(&InfoSection::Platform) {
            snapshot.platform = Some(collect_platform());
        }
        if sections.contains(&InfoSection::Cpu) {
            snapshot.cpu = Some(collectors.cpu());
        }
        if sections.contains(&InfoSection::Memory) || sections.contains(&InfoSection::Swap) {
            let mut kinds = MemoryRefreshKind::nothing();
            if sections.contains(&InfoSection::Memory) {
                kinds = kinds.with_ram();
            }
            if sections.contains(&InfoSection::Swap) {
                kinds = kinds.with_swap();
            }
            collectors.system.refresh_memory_specifics(kinds);
            if sections.contains(&InfoSection::Memory) {
                snapshot.memory = Some(MemoryInfo {
                    total_bytes: collectors.system.total_memory(),
                    used_bytes: collectors.system.used_memory(),
                    available_bytes: collectors.system.available_memory(),
                    free_bytes: Some(collectors.system.free_memory()),
                });
            }
            if sections.contains(&InfoSection::Swap) {
                snapshot.swap = Some(SwapInfo {
                    total_bytes: collectors.system.total_swap(),
                    used_bytes: collectors.system.used_swap(),
                    free_bytes: Some(collectors.system.free_swap()),
                });
            }
        }
        if sections.contains(&InfoSection::Disks) {
            snapshot.disks = Some(collectors.disks());
        }
        if sections.contains(&InfoSection::Networks) {
            snapshot.networks = Some(collectors.networks());
        }
        if sections.contains(&InfoSection::Components) {
            snapshot.components = Some(collectors.components());
        }

        Ok(snapshot)
    }
}

impl Collectors {
    fn probe() -> Result<(Self, Vec<SectionFailure>), String> {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        )
        .map_err(|error| format!("failed to initialize sysinfo system data: {error}"))?;
        let physical_cores = System::physical_core_count()
            .ok()
            .and_then(|count| u32::try_from(count).ok());
        let mut section_failures = Vec::new();

        let disks = match Disks::new_with_refreshed_list() {
            Ok(disks) => Some(disks),
            Err(error) => {
                section_failures.push(SectionFailure {
                    section: InfoSection::Disks,
                    message: format!("failed to initialize disk data: {error}"),
                });
                None
            }
        };
        let networks = match Networks::new_with_refreshed_list() {
            Ok(networks) => Some(networks),
            Err(error) => {
                section_failures.push(SectionFailure {
                    section: InfoSection::Networks,
                    message: format!("failed to initialize network data: {error}"),
                });
                None
            }
        };
        let components = match Components::new_with_refreshed_list() {
            Ok(components) => Some(components),
            Err(error) => {
                section_failures.push(SectionFailure {
                    section: InfoSection::Components,
                    message: format!("failed to initialize component data: {error}"),
                });
                None
            }
        };

        Ok((
            Self {
                system,
                physical_cores,
                disks,
                networks,
                components,
            },
            section_failures,
        ))
    }

    fn cpu(&mut self) -> CpuInfo {
        self.system.refresh_cpu_all();
        let cpus = self.system.cpus();
        let logical_cores = u32::try_from(cpus.len()).unwrap_or(u32::MAX);
        let first = cpus.first();
        let frequency_sum = cpus
            .iter()
            .map(::sysinfo::Cpu::frequency)
            .fold(0_u64, u64::saturating_add);
        let frequency_mhz = if logical_cores == 0 {
            0
        } else {
            frequency_sum / u64::from(logical_cores)
        };

        CpuInfo {
            name: first
                .map(|cpu| cpu.name())
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
            vendor_id: first
                .map(|cpu| cpu.vendor_id())
                .filter(|vendor| !vendor.is_empty())
                .map(str::to_owned),
            brand: first
                .map(|cpu| cpu.brand())
                .filter(|brand| !brand.is_empty())
                .map(str::to_owned),
            physical_cores: self.physical_cores,
            logical_cores,
            usage_percent: self.system.global_cpu_usage(),
            frequency_mhz,
            per_core_usage_percent: cpus.iter().map(::sysinfo::Cpu::usage).collect(),
        }
    }

    fn disks(&mut self) -> Vec<DiskInfo> {
        let Some(disks) = self.disks.as_mut() else {
            return Vec::new();
        };
        disks.refresh(true);
        let mut output: Vec<_> = disks
            .iter()
            .map(|disk| {
                let usage = disk.usage();
                DiskInfo {
                    id: disk.id(),
                    name: disk.name().to_string_lossy().into_owned(),
                    mount_point: disk.mount_point().to_string_lossy().into_owned(),
                    filesystem: non_empty_os_string(disk.file_system()),
                    kind: Some(disk.kind().to_string().to_ascii_lowercase()),
                    total_bytes: disk.total_space(),
                    available_bytes: disk.available_space(),
                    removable: disk.is_removable(),
                    read_only: disk.is_read_only(),
                    read_bytes: usage.read_bytes,
                    total_read_bytes: usage.total_read_bytes,
                    written_bytes: usage.written_bytes,
                    total_written_bytes: usage.total_written_bytes,
                }
            })
            .collect();
        output.sort_by(|left, right| left.mount_point.cmp(&right.mount_point));
        output
    }

    fn networks(&mut self) -> Vec<NetworkInfo> {
        let Some(networks) = self.networks.as_mut() else {
            return Vec::new();
        };
        networks.refresh(true);
        let mut output: Vec<_> = networks
            .iter()
            .map(|(name, network)| NetworkInfo {
                name: name.to_string_lossy().into_owned(),
                received_bytes: network.received(),
                transmitted_bytes: network.transmitted(),
                total_received_bytes: network.total_received(),
                total_transmitted_bytes: network.total_transmitted(),
                packets_received: network.packets_received(),
                total_packets_received: network.total_packets_received(),
                packets_transmitted: network.packets_transmitted(),
                total_packets_transmitted: network.total_packets_transmitted(),
                receive_errors: network.errors_on_received(),
                transmit_errors: network.errors_on_transmitted(),
                mac_address: (!network.mac_address().is_unspecified())
                    .then(|| network.mac_address().to_string()),
                mtu: Some(network.mtu()),
                receive_link_speed_bps: network.receive_link_speed(),
                transmit_link_speed_bps: network.transmit_link_speed(),
                operational_state: Some(network.operational_state().to_string()),
            })
            .collect();
        output.sort_by(|left, right| left.name.cmp(&right.name));
        output
    }

    fn components(&mut self) -> Vec<ComponentInfo> {
        let Some(components) = self.components.as_mut() else {
            return Vec::new();
        };
        components.refresh(true);
        let mut output: Vec<_> = components
            .iter()
            .map(|component| ComponentInfo {
                id: component.id().map(str::to_owned),
                label: component.label().to_owned(),
                temperature_celsius: finite(component.temperature()),
                max_celsius: finite(component.max()),
                critical_celsius: finite(component.critical()),
            })
            .collect();
        output.sort_by(|left, right| left.label.cmp(&right.label));
        output
    }
}

fn collect_platform() -> SystemInfo {
    SystemInfo {
        host_name: System::host_name().ok(),
        os_name: System::name().ok(),
        os_family: std::env::consts::OS.to_owned(),
        os_version: System::os_version().ok(),
        long_os_version: System::long_os_version().ok(),
        kernel_version: System::kernel_version().ok(),
        kernel_long_version: System::kernel_long_version().ok(),
        distribution_id: non_empty(System::distribution_id()),
        distribution_id_like: System::distribution_id_like(),
        architecture: System::cpu_arch(),
        uptime_seconds: System::uptime().ok(),
        boot_time_unix_seconds: System::boot_time().ok(),
        load_average: System::load_average().ok().map(|load| LoadAverage {
            one_minute: load.one,
            five_minutes: load.five,
            fifteen_minutes: load.fifteen,
        }),
    }
}

fn finite(value: Option<f32>) -> Option<f32> {
    value.filter(|value| value.is_finite())
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn non_empty_os_string(value: &std::ffi::OsStr) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string_lossy().into_owned())
}
