use battery::units::{
    electric_potential::volt, energy::watt_hour, power::watt, ratio::percent,
    thermodynamic_temperature::degree_celsius, time::second,
};

use crate::api::{BatteryInfo, InfoSection, InfoWarning, ProviderStatus};

pub(super) struct BatteryInfoSource {
    status: ProviderStatus,
}

impl BatteryInfoSource {
    pub fn probe() -> Self {
        Self {
            status: probe_status(),
        }
    }

    pub fn status(&self) -> ProviderStatus {
        self.status.clone()
    }

    pub fn snapshot(&self) -> (Vec<BatteryInfo>, Vec<InfoWarning>) {
        let manager = match battery::Manager::new() {
            Ok(manager) => manager,
            Err(error) => {
                return (
                    Vec::new(),
                    vec![warning(
                        "battery_unavailable",
                        format!("failed to initialize battery provider: {error}"),
                    )],
                );
            }
        };
        let batteries = match manager.batteries() {
            Ok(batteries) => batteries,
            Err(error) => {
                return (
                    Vec::new(),
                    vec![warning(
                        "battery_unavailable",
                        format!("failed to enumerate batteries: {error}"),
                    )],
                );
            }
        };

        let mut output = Vec::new();
        let mut warnings = Vec::new();
        for (index, result) in batteries.enumerate() {
            match result {
                Ok(battery) => output.push(battery_info(index, &battery)),
                Err(error) => warnings.push(warning(
                    "battery_read_failed",
                    format!("failed to read one battery: {error}"),
                )),
            }
        }
        if output.is_empty() && warnings.is_empty() {
            warnings.push(warning(
                "no_battery_detected",
                "no battery was detected on this system",
            ));
        }

        (output, warnings)
    }
}

fn probe_status() -> ProviderStatus {
    let manager = match battery::Manager::new() {
        Ok(manager) => manager,
        Err(error) => {
            return ProviderStatus::unavailable(format!(
                "failed to initialize battery provider: {error}"
            ));
        }
    };
    let batteries = match manager.batteries() {
        Ok(batteries) => batteries,
        Err(error) => {
            return ProviderStatus::unavailable(format!("failed to enumerate batteries: {error}"));
        }
    };

    let mut first_error = None;
    for result in batteries {
        match result {
            Ok(_) => return ProviderStatus::available(),
            Err(error) => first_error = Some(error),
        }
    }

    first_error.map_or_else(
        || ProviderStatus::unavailable("no_battery_detected"),
        |error| ProviderStatus::unavailable(format!("failed to read battery information: {error}")),
    )
}

fn battery_info(index: usize, battery: &battery::Battery) -> BatteryInfo {
    BatteryInfo {
        index: u32::try_from(index).unwrap_or(u32::MAX),
        vendor: non_empty(battery.vendor()),
        model: non_empty(battery.model()),
        technology: battery.technology().to_string(),
        state: battery.state().to_string(),
        state_of_charge_percent: percentage(battery.state_of_charge().get::<percent>()),
        state_of_health_percent: percentage(battery.state_of_health().get::<percent>()),
        energy_wh: non_negative(battery.energy().get::<watt_hour>()),
        energy_full_wh: non_negative(battery.energy_full().get::<watt_hour>()),
        energy_full_design_wh: non_negative(battery.energy_full_design().get::<watt_hour>()),
        energy_rate_watts: non_negative(battery.energy_rate().get::<watt>()),
        voltage_volts: non_negative(battery.voltage().get::<volt>()),
        temperature_celsius: battery
            .temperature()
            .map(|temperature| temperature.get::<degree_celsius>())
            .and_then(finite),
        cycle_count: battery.cycle_count(),
        time_to_full_seconds: battery
            .time_to_full()
            .and_then(|time| duration_seconds(time.get::<second>())),
        time_to_empty_seconds: battery
            .time_to_empty()
            .and_then(|time| duration_seconds(time.get::<second>())),
    }
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn percentage(value: f32) -> Option<f32> {
    non_negative(value).map(|value| value.min(100.0))
}

fn duration_seconds(value: f32) -> Option<u64> {
    non_negative(value).map(|value| value.round() as u64)
}

fn non_negative(value: f32) -> Option<f32> {
    finite(value).map(|value| value.max(0.0))
}

fn finite(value: f32) -> Option<f32> {
    value.is_finite().then_some(value)
}

fn warning(code: &str, message: impl Into<String>) -> InfoWarning {
    InfoWarning {
        code: code.to_owned(),
        message: message.into(),
        provider: Some("battery".to_owned()),
        section: Some(InfoSection::Batteries),
    }
}

#[cfg(test)]
mod tests {
    use super::{BatteryInfoSource, duration_seconds, percentage};

    #[test]
    fn normalizes_battery_measurements() {
        assert_eq!(percentage(73.4), Some(73.4));
        assert_eq!(percentage(-1.0), Some(0.0));
        assert_eq!(percentage(120.0), Some(100.0));
        assert_eq!(percentage(f32::NAN), None);
        assert_eq!(duration_seconds(11_639.6), Some(11_640));
        assert_eq!(duration_seconds(-1.0), Some(0));
        assert_eq!(duration_seconds(f32::INFINITY), None);
    }

    #[test]
    fn snapshot_is_non_fatal_with_or_without_a_battery() {
        let (batteries, warnings) = BatteryInfoSource::probe().snapshot();

        assert!(batteries.iter().all(|battery| !battery.state.is_empty()));
        if batteries.is_empty() {
            assert!(
                warnings
                    .iter()
                    .any(|warning| warning.provider.as_deref() == Some("battery"))
            );
        }
    }
}
