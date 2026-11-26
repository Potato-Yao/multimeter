use std::collections::BTreeMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::ptr;
use std::slice;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::System::Performance::{
    PDH_CSTATUS_NEW_DATA, PDH_CSTATUS_VALID_DATA, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
    PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery,
    PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
};

use super::{BackendSample, PowerBackend};

const ENERGY_METER_COUNTER_PATH: &str = r"\Energy Meter(*)\Power";
const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);

struct WindowsEnergyMeterBackend {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
    last_sample_at: Option<Instant>,
}

pub(super) fn probe() -> Result<Box<dyn PowerBackend>, String> {
    let mut backend = WindowsEnergyMeterBackend::open()?;
    let package_power = match backend.current_package_power() {
        Ok(package_power) => package_power,
        Err(first_error) => {
            thread::sleep(Duration::from_millis(100));
            backend
                .current_package_power()
                .map_err(|retry_error| format!("{first_error}; retry failed: {retry_error}"))?
        }
    };
    if package_power.is_empty() {
        return Err("no RAPL package power counters were found in the Energy Meter".to_owned());
    }
    backend.last_sample_at = Some(Instant::now());
    Ok(Box::new(backend))
}

impl WindowsEnergyMeterBackend {
    fn open() -> Result<Self, String> {
        let mut query = ptr::null_mut();
        check_pdh(
            unsafe { PdhOpenQueryW(ptr::null(), 0, &mut query) },
            "open PDH query",
        )?;

        let counter_path = wide_string(ENERGY_METER_COUNTER_PATH);
        let mut counter = ptr::null_mut();
        let status =
            unsafe { PdhAddEnglishCounterW(query, counter_path.as_ptr(), 0, &mut counter) };
        if status != 0 {
            unsafe {
                PdhCloseQuery(query);
            }
            return Err(pdh_error("add Energy Meter power counter", status));
        }

        Ok(Self {
            query,
            counter,
            last_sample_at: None,
        })
    }

    fn current_package_power(&mut self) -> Result<BTreeMap<u32, f64>, String> {
        check_pdh(
            unsafe { PdhCollectQueryData(self.query) },
            "collect Energy Meter data",
        )?;

        for _ in 0..3 {
            let mut buffer_size = 0_u32;
            let mut item_count = 0_u32;
            let status = unsafe {
                PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut buffer_size,
                    &mut item_count,
                    ptr::null_mut(),
                )
            };
            if status != PDH_MORE_DATA && status != 0 {
                return Err(pdh_error("size Energy Meter counter data", status));
            }
            if buffer_size == 0 || item_count == 0 {
                return Ok(BTreeMap::new());
            }

            let word_count = (buffer_size as usize).div_ceil(std::mem::size_of::<usize>());
            let mut buffer = vec![0_usize; word_count];
            let mut item_count = 0_u32;
            let status = unsafe {
                PdhGetFormattedCounterArrayW(
                    self.counter,
                    PDH_FMT_DOUBLE,
                    &mut buffer_size,
                    &mut item_count,
                    buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
                )
            };
            if status == PDH_MORE_DATA {
                continue;
            }
            check_pdh(status, "read Energy Meter counter data")?;

            let items = unsafe {
                slice::from_raw_parts(
                    buffer.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
                    item_count as usize,
                )
            };
            let mut package_power = BTreeMap::new();
            for item in items {
                if item.FmtValue.CStatus != PDH_CSTATUS_VALID_DATA
                    && item.FmtValue.CStatus != PDH_CSTATUS_NEW_DATA
                {
                    continue;
                }
                let Some(name) = wide_pointer_to_string(item.szName) else {
                    continue;
                };
                let Some(package_index) = package_index(&name) else {
                    continue;
                };
                let milliwatts = unsafe { item.FmtValue.Anonymous.doubleValue };
                if milliwatts.is_finite() && milliwatts >= 0.0 {
                    package_power.entry(package_index).or_insert(milliwatts);
                }
            }
            return Ok(package_power);
        }

        Err("Energy Meter counter data changed repeatedly while being read".to_owned())
    }
}

impl PowerBackend for WindowsEnergyMeterBackend {
    fn reset(&mut self) -> Result<(), String> {
        self.last_sample_at = None;
        Ok(())
    }

    fn sample(&mut self) -> Result<BackendSample, String> {
        if self
            .last_sample_at
            .is_some_and(|sampled_at| sampled_at.elapsed() < SAMPLE_INTERVAL)
        {
            return Ok(BackendSample::TooSoon);
        }

        let package_power = self.current_package_power()?;
        if package_power.is_empty() {
            return Err("no RAPL package power counters are currently available".to_owned());
        }
        self.last_sample_at = Some(Instant::now());
        let total_milliwatts: f64 = package_power.into_values().sum();
        // Windows Energy Meter RAPL power counters report milliwatts.
        Ok(BackendSample::Watts((total_milliwatts / 1_000.0) as f32))
    }
}

impl Drop for WindowsEnergyMeterBackend {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.query);
        }
    }
}

// PDH handles are used only through CpuPowerSource's mutex, which serializes access.
unsafe impl Send for WindowsEnergyMeterBackend {}

fn package_index(instance: &str) -> Option<u32> {
    let lower = instance.to_ascii_lowercase();
    let suffix = lower.strip_prefix("rapl_package")?;
    let digit_count = suffix.bytes().take_while(u8::is_ascii_digit).count();
    if digit_count == 0 || !suffix[digit_count..].starts_with("_pkg") {
        return None;
    }
    suffix[..digit_count].parse().ok()
}

fn check_pdh(status: u32, operation: &str) -> Result<(), String> {
    (status == 0)
        .then_some(())
        .ok_or_else(|| pdh_error(operation, status))
}

fn pdh_error(operation: &str, status: u32) -> String {
    format!("failed to {operation}: PDH error 0x{status:08x}")
}

fn wide_string(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_pointer_to_string(value: *const u16) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let mut length = 0;
    unsafe {
        while *value.add(length) != 0 {
            length += 1;
        }
        Some(
            OsString::from_wide(slice::from_raw_parts(value, length))
                .to_string_lossy()
                .into_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::package_index;

    #[test]
    fn selects_only_package_energy_meter_instances() {
        assert_eq!(package_index("RAPL_Package0_PKG"), Some(0));
        assert_eq!(package_index("rapl_package12_pkg#1"), Some(12));
        assert_eq!(package_index("RAPL_Package0_Core1_CORE"), None);
        assert_eq!(package_index("RAPL_Package0_DRAM"), None);
        assert_eq!(package_index("BATTERY_TOP"), None);
    }
}
