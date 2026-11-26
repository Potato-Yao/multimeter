use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InfoSection {
    Platform,
    Cpu,
    #[serde(rename = "self")]
    Process,
    Memory,
    Swap,
    Disks,
    Networks,
    Gpus,
    Components,
    Batteries,
}

impl InfoSection {
    pub const ALL: [Self; 10] = [
        Self::Platform,
        Self::Cpu,
        Self::Process,
        Self::Memory,
        Self::Swap,
        Self::Disks,
        Self::Networks,
        Self::Gpus,
        Self::Components,
        Self::Batteries,
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

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SystemInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_name: Option<String>,
    pub os_family: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_os_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_long_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distribution_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub distribution_id_like: Vec<String>,
    pub architecture: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uptime_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_time_unix_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_average: Option<LoadAverage>,
}

pub type PlatformInfo = SystemInfo;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LoadAverage {
    pub one_minute: f64,
    pub five_minutes: f64,
    pub fifteen_minutes: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct CpuInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brand: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub physical_cores: Option<u32>,
    pub logical_cores: u32,
    pub usage_percent: f32,
    pub frequency_mhz: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_watts: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub per_core_usage_percent: Vec<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub cpu_usage_percent: f32,
    pub memory_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_bytes: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SwapInfo {
    pub total_bytes: u64,
    pub used_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_bytes: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DiskInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub name: String,
    pub mount_point: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub removable: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub read_only: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub read_bytes: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub total_read_bytes: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub written_bytes: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub total_written_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct NetworkInfo {
    pub name: String,
    pub received_bytes: u64,
    pub transmitted_bytes: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub total_received_bytes: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub total_transmitted_bytes: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub packets_received: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub total_packets_received: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub packets_transmitted: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub total_packets_transmitted: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub receive_errors: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub transmit_errors: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mac_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtu: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receive_link_speed_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transmit_link_speed_bps: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operational_state: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct GpuInfo {
    pub index: u32,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utilization_percent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_total_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_used_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_celsius: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_watts: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ComponentInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_celsius: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_celsius: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical_celsius: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct BatteryInfo {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub technology: String,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_of_charge_percent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_of_health_percent: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_wh: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_full_wh: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_full_design_wh: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub energy_rate_watts: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voltage_volts: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature_celsius: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_full_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_empty_seconds: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InfoWarning {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<InfoSection>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct HardwareSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<SystemInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<CpuInfo>,
    #[serde(rename = "self", skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessInfo>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batteries: Option<Vec<BatteryInfo>>,
    pub warnings: Vec<InfoWarning>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        BatteryInfo, CpuInfo, DiskInfo, HardwareSnapshot, InfoSection, InfoWarning, ProcessInfo,
        SystemInfo, normalize_sections,
    };

    #[test]
    fn normalizes_section_order_and_duplicates() {
        assert_eq!(
            normalize_sections(vec![
                InfoSection::Cpu,
                InfoSection::Process,
                InfoSection::Memory,
                InfoSection::Process,
                InfoSection::Cpu,
            ]),
            vec![InfoSection::Cpu, InfoSection::Process, InfoSection::Memory]
        );
    }

    #[test]
    fn serializes_system_and_hardware_units() {
        let snapshot = HardwareSnapshot {
            platform: Some(SystemInfo {
                host_name: Some("guest-pc".to_owned()),
                os_family: "windows".to_owned(),
                architecture: "x86_64".to_owned(),
                uptime_seconds: Some(3_600),
                ..SystemInfo::default()
            }),
            cpu: Some(CpuInfo {
                logical_cores: 8,
                usage_percent: 25.0,
                frequency_mhz: 3_200,
                power_watts: Some(40.0),
                per_core_usage_percent: vec![20.0, 30.0],
                ..CpuInfo::default()
            }),
            process: Some(ProcessInfo {
                pid: 1_234,
                cpu_usage_percent: 12.5,
                memory_bytes: 64 * 1024 * 1024,
            }),
            disks: Some(vec![DiskInfo {
                name: "disk0".to_owned(),
                mount_point: "/".to_owned(),
                total_bytes: 1_000,
                available_bytes: 250,
                ..DiskInfo::default()
            }]),
            batteries: Some(vec![BatteryInfo {
                index: 0,
                vendor: Some("Example Vendor".to_owned()),
                model: Some("Example Battery".to_owned()),
                technology: "lithium-ion".to_owned(),
                state: "discharging".to_owned(),
                state_of_charge_percent: Some(73.4),
                state_of_health_percent: Some(91.2),
                energy_wh: Some(41.2),
                energy_full_wh: Some(56.3),
                energy_full_design_wh: Some(61.8),
                energy_rate_watts: Some(12.7),
                voltage_volts: Some(12.1),
                temperature_celsius: Some(31.4),
                cycle_count: Some(142),
                time_to_full_seconds: None,
                time_to_empty_seconds: Some(11_640),
            }]),
            warnings: vec![InfoWarning {
                code: "partial_data".to_owned(),
                message: "temperature unavailable".to_owned(),
                provider: None,
                section: Some(InfoSection::Components),
            }],
            ..HardwareSnapshot::default()
        };

        let value = serde_json::to_value(snapshot).expect("snapshot should serialize");
        assert_eq!(value["platform"]["uptime_seconds"], 3_600);
        assert_eq!(value["cpu"]["frequency_mhz"], 3_200);
        assert_eq!(value["cpu"]["power_watts"], 40.0);
        assert_eq!(value["cpu"]["per_core_usage_percent"], json!([20.0, 30.0]));
        assert_eq!(value["self"]["pid"], 1_234);
        assert_eq!(value["self"]["cpu_usage_percent"], 12.5);
        assert_eq!(value["self"]["memory_bytes"], 67_108_864);
        assert_eq!(value["disks"][0]["available_bytes"], 250);
        assert_eq!(value["batteries"][0]["state"], "discharging");
        assert!(
            (value["batteries"][0]["state_of_charge_percent"]
                .as_f64()
                .expect("charge should be numeric")
                - 73.4)
                .abs()
                < 0.001
        );
        assert_eq!(value["batteries"][0]["time_to_empty_seconds"], 11_640);
        assert!(value["batteries"][0].get("time_to_full_seconds").is_none());
        assert_eq!(value["warnings"][0]["section"], "components");
    }
}
