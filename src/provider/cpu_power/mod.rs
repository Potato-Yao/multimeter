#[cfg(target_os = "linux")]
mod linux;

use std::sync::Mutex;

use crate::api::ProviderStatus;
use crate::model::{InfoSection, InfoWarning};

#[cfg(target_os = "linux")]
const PROVIDER_NAME: &str = "rapl";
#[cfg(target_os = "windows")]
const PROVIDER_NAME: &str = "windows_energy_meter";
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
const PROVIDER_NAME: &str = "cpu_power";

pub(super) struct CpuPowerSource {
    status: ProviderStatus,
    state: Mutex<CpuPowerState>,
}

struct CpuPowerState {
    backend: Option<Box<dyn PowerBackend>>,
    requested_last_snapshot: bool,
    last_power_watts: Option<f32>,
}

trait PowerBackend: Send {
    fn reset(&mut self) -> Result<(), String>;
    fn sample(&mut self) -> Result<BackendSample, String>;
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(target_os = "windows", allow(dead_code))]
enum BackendSample {
    Watts(f32),
    TooSoon,
}

impl CpuPowerSource {
    pub(super) fn probe() -> Self {
        let (backend, status) = match probe_backend() {
            Ok(backend) => (Some(backend), ProviderStatus::available()),
            Err(error) => (None, ProviderStatus::unavailable(error)),
        };

        Self {
            status,
            state: Mutex::new(CpuPowerState {
                backend,
                requested_last_snapshot: false,
                last_power_watts: None,
            }),
        }
    }

    pub(super) fn status(&self) -> ProviderStatus {
        self.status.clone()
    }

    pub(super) fn sample(&self, requested: bool) -> (Option<f32>, Option<InfoWarning>) {
        let Ok(mut state) = self.state.lock() else {
            return (
                None,
                Some(power_warning(
                    "cpu_power_query_failed",
                    "CPU power collector lock failed",
                )),
            );
        };

        if !requested {
            state.requested_last_snapshot = false;
            state.last_power_watts = None;
            return (None, None);
        }

        if state.backend.is_none() {
            return (
                None,
                Some(power_warning(
                    "cpu_power_unavailable",
                    self.status
                        .reason
                        .as_deref()
                        .unwrap_or("CPU package power is unavailable"),
                )),
            );
        }

        if !state.requested_last_snapshot {
            state.requested_last_snapshot = true;
            state.last_power_watts = None;
            if let Err(error) = state
                .backend
                .as_mut()
                .expect("backend presence checked above")
                .reset()
            {
                state.requested_last_snapshot = false;
                return (None, Some(power_warning("cpu_power_query_failed", error)));
            }
            return (None, None);
        }

        let backend = state
            .backend
            .as_mut()
            .expect("backend presence checked above");
        match backend.sample() {
            Ok(BackendSample::Watts(watts)) if watts.is_finite() && watts >= 0.0 => {
                state.last_power_watts = Some(watts);
                (Some(watts), None)
            }
            Ok(BackendSample::Watts(watts)) => {
                state.requested_last_snapshot = false;
                (
                    None,
                    Some(power_warning(
                        "cpu_power_query_failed",
                        format!("CPU power provider returned an invalid value: {watts}"),
                    )),
                )
            }
            Ok(BackendSample::TooSoon) => (state.last_power_watts, None),
            Err(error) => {
                state.requested_last_snapshot = false;
                (None, Some(power_warning("cpu_power_query_failed", error)))
            }
        }
    }

    pub(super) fn reset_transient_samples(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.requested_last_snapshot = false;
            state.last_power_watts = None;
        }
    }
}

fn power_warning(code: &str, message: impl Into<String>) -> InfoWarning {
    InfoWarning {
        code: code.to_owned(),
        message: message.into(),
        provider: Some(PROVIDER_NAME.to_owned()),
        section: Some(InfoSection::Cpu),
    }
}

#[cfg(target_os = "linux")]
fn probe_backend() -> Result<Box<dyn PowerBackend>, String> {
    linux::probe()
}

#[cfg(target_os = "windows")]
fn probe_backend() -> Result<Box<dyn PowerBackend>, String> {
    Err("Windows Energy Meter support is not enabled".to_owned())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn probe_backend() -> Result<Box<dyn PowerBackend>, String> {
    Err("CPU package power is only supported on Windows and Linux".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{BackendSample, CpuPowerSource, CpuPowerState, PowerBackend};
    use crate::api::ProviderStatus;
    use std::sync::Mutex;

    struct FakeBackend {
        resets: usize,
        samples: usize,
    }

    impl PowerBackend for FakeBackend {
        fn reset(&mut self) -> Result<(), String> {
            self.resets += 1;
            Ok(())
        }

        fn sample(&mut self) -> Result<BackendSample, String> {
            self.samples += 1;
            Ok(BackendSample::Watts(42.5))
        }
    }

    #[test]
    fn primes_after_entering_or_resuming_cpu_collection() {
        let source = CpuPowerSource {
            status: ProviderStatus::available(),
            state: Mutex::new(CpuPowerState {
                backend: Some(Box::new(FakeBackend {
                    resets: 0,
                    samples: 0,
                })),
                requested_last_snapshot: false,
                last_power_watts: None,
            }),
        };

        assert_eq!(source.sample(true), (None, None));
        assert_eq!(source.sample(true), (Some(42.5), None));
        assert_eq!(source.sample(false), (None, None));
        assert_eq!(source.sample(true), (None, None));
    }

    #[test]
    fn unavailable_provider_reports_a_cpu_section_warning() {
        let source = CpuPowerSource {
            status: ProviderStatus::unavailable("no package counters"),
            state: Mutex::new(CpuPowerState {
                backend: None,
                requested_last_snapshot: false,
                last_power_watts: None,
            }),
        };

        let (watts, warning) = source.sample(true);
        assert!(watts.is_none());
        assert_eq!(
            warning.as_ref().map(|warning| warning.code.as_str()),
            Some("cpu_power_unavailable")
        );
    }

    #[test]
    fn probing_the_platform_backend_never_panics() {
        let source = CpuPowerSource::probe();
        let _ = source.status();
        let _ = source.sample(true);
    }
}
