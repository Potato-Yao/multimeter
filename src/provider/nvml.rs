use crate::api::ProviderStatus;
use crate::model::{GpuInfo, InfoWarning};

use super::{gpu_warning, unavailable_gpus};

#[cfg(any(target_os = "linux", target_os = "windows"))]
use nvml_wrapper::Nvml;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use nvml_wrapper::enum_wrappers::device::TemperatureSensor;

pub(super) struct NvmlInfoSource {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    nvml: Option<Nvml>,
    status: ProviderStatus,
}

impl NvmlInfoSource {
    pub(super) fn probe() -> Self {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            match Nvml::init() {
                Ok(nvml) => match nvml.device_count() {
                    Ok(0) => Self {
                        nvml: None,
                        status: ProviderStatus::unavailable("no NVIDIA GPU found"),
                    },
                    Ok(_) => Self {
                        nvml: Some(nvml),
                        status: ProviderStatus::available(),
                    },
                    Err(error) => Self {
                        nvml: None,
                        status: ProviderStatus::unavailable(format!(
                            "failed to enumerate NVIDIA GPUs: {error}"
                        )),
                    },
                },
                Err(error) => Self {
                    nvml: None,
                    status: ProviderStatus::unavailable(init_error_message(error)),
                },
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Self {
                status: ProviderStatus::unavailable("NVML is only supported on Windows and Linux"),
            }
        }
    }

    pub(super) fn status(&self) -> ProviderStatus {
        self.status.clone()
    }

    pub(super) fn gpus(&self) -> (Option<Vec<GpuInfo>>, Vec<InfoWarning>) {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.collect_gpus()
        }

        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            self.unavailable_gpus()
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn collect_gpus(&self) -> (Option<Vec<GpuInfo>>, Vec<InfoWarning>) {
        let Some(nvml) = self.nvml.as_ref() else {
            return self.unavailable_gpus();
        };

        let count = match nvml.device_count() {
            Ok(count) => count,
            Err(error) => {
                return (
                    unavailable_gpus(),
                    vec![gpu_warning(
                        "gpu_query_failed",
                        format!("failed to enumerate NVIDIA GPUs: {error}"),
                    )],
                );
            }
        };
        let driver_version = nvml.sys_driver_version().ok();
        let mut gpus = Vec::with_capacity(count as usize);
        let mut warnings = Vec::new();

        for index in 0..count {
            let device = match nvml.device_by_index(index) {
                Ok(device) => device,
                Err(error) => {
                    warnings.push(gpu_warning(
                        "gpu_unavailable",
                        format!("failed to query NVIDIA GPU {index}: {error}"),
                    ));
                    continue;
                }
            };

            let name = device
                .name()
                .unwrap_or_else(|_| format!("NVIDIA GPU {index}"));
            let utilization_percent = device
                .utilization_rates()
                .ok()
                .map(|rates| rates.gpu as f32);
            let (memory_total_bytes, memory_used_bytes) = device
                .memory_info()
                .map(|memory| (Some(memory.total), Some(memory.used)))
                .unwrap_or((None, None));
            let temperature_celsius = device
                .temperature(TemperatureSensor::Gpu)
                .ok()
                .map(|temperature| temperature as f32);
            let power_watts = device
                .power_usage()
                .ok()
                .map(|milliwatts| milliwatts as f32 / 1_000.0);

            gpus.push(GpuInfo {
                index,
                name,
                driver_version: driver_version.clone(),
                utilization_percent,
                memory_total_bytes,
                memory_used_bytes,
                temperature_celsius,
                power_watts,
            });
        }

        (Some(gpus), warnings)
    }

    fn unavailable_gpus(&self) -> (Option<Vec<GpuInfo>>, Vec<InfoWarning>) {
        let message = self
            .status
            .reason
            .as_deref()
            .unwrap_or("NVML is unavailable");
        (
            unavailable_gpus(),
            vec![gpu_warning("provider_unavailable", message)],
        )
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn init_error_message(error: nvml_wrapper::error::NvmlError) -> String {
    use nvml_wrapper::error::NvmlError;

    match error {
        NvmlError::LibraryNotFound => "NVIDIA Management Library was not found".to_owned(),
        NvmlError::FunctionNotFound => {
            "NVIDIA Management Library is missing required functions".to_owned()
        }
        NvmlError::DriverNotLoaded => "NVIDIA driver is not loaded".to_owned(),
        NvmlError::NoPermission => "permission denied while opening NVML".to_owned(),
        other => format!("failed to initialize NVML: {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::NvmlInfoSource;

    #[test]
    fn probing_nvml_never_panics() {
        let source = NvmlInfoSource::probe();
        let _ = source.status();
        let (gpus, warnings) = source.gpus();
        if let Some(reason) = source.status().reason {
            assert!(gpus.is_none());
            assert!(warnings.iter().any(|warning| warning.message == reason));
        } else {
            assert!(gpus.is_some());
        }
    }
}
