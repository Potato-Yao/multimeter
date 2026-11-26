use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{BackendSample, PowerBackend};

const POWER_CAP_ROOT: &str = "/sys/class/powercap";
const MIN_SAMPLE_INTERVAL: Duration = Duration::from_millis(50);

struct RaplBackend {
    domains: Vec<EnergyDomain>,
}

struct EnergyDomain {
    name: String,
    energy_path: PathBuf,
    max_energy_range_uj: Option<u64>,
    previous: Option<(Instant, u64)>,
}

struct Candidate {
    name: String,
    energy_path: PathBuf,
    max_energy_range_path: PathBuf,
    priority: u8,
}

pub(super) fn probe() -> Result<Box<dyn PowerBackend>, String> {
    probe_from(Path::new(POWER_CAP_ROOT))
}

fn probe_from(root: &Path) -> Result<Box<dyn PowerBackend>, String> {
    let candidates = discover_candidates(root)?;
    if candidates.is_empty() {
        return Err("no RAPL package counters were found".to_owned());
    }

    let mut domains = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        let max_energy_range_uj = fs::read_to_string(&candidate.max_energy_range_path)
            .ok()
            .and_then(|value| value.trim().parse().ok());
        let current = read_u64(&candidate.energy_path).map_err(|error| {
            format!(
                "failed to read RAPL energy counter {}: {error}",
                candidate.energy_path.display()
            )
        })?;
        domains.push(EnergyDomain {
            name: candidate.name,
            energy_path: candidate.energy_path,
            max_energy_range_uj,
            previous: Some((Instant::now(), current)),
        });
    }

    Ok(Box::new(RaplBackend { domains }))
}

impl PowerBackend for RaplBackend {
    fn reset(&mut self) -> Result<(), String> {
        let mut refreshed = Vec::with_capacity(self.domains.len());
        for domain in &self.domains {
            let current = read_u64(&domain.energy_path).map_err(|error| {
                format!(
                    "failed to read RAPL energy counter {}: {error}",
                    domain.energy_path.display()
                )
            })?;
            refreshed.push((Instant::now(), current));
        }

        for (domain, previous) in self.domains.iter_mut().zip(refreshed) {
            domain.previous = Some(previous);
        }
        Ok(())
    }

    fn sample(&mut self) -> Result<BackendSample, String> {
        if self.domains.iter().any(|domain| {
            domain
                .previous
                .as_ref()
                .is_some_and(|(sampled_at, _)| sampled_at.elapsed() < MIN_SAMPLE_INTERVAL)
        }) {
            return Ok(BackendSample::TooSoon);
        }

        let mut total_watts = 0.0;
        let mut refreshed = Vec::with_capacity(self.domains.len());

        for domain in &self.domains {
            let Some((previous_at, previous_energy_uj)) = domain.previous else {
                return Ok(BackendSample::TooSoon);
            };
            let current_energy_uj = read_u64(&domain.energy_path).map_err(|error| {
                format!(
                    "failed to read RAPL energy counter {}: {error}",
                    domain.energy_path.display()
                )
            })?;
            let sampled_at = Instant::now();
            let elapsed = sampled_at.saturating_duration_since(previous_at);
            if elapsed < MIN_SAMPLE_INTERVAL {
                return Ok(BackendSample::TooSoon);
            }

            let delta_uj = energy_delta(
                previous_energy_uj,
                current_energy_uj,
                domain.max_energy_range_uj,
            )
            .map_err(|error| format!("RAPL domain {}: {error}", domain.name))?;
            total_watts += delta_uj as f64 / elapsed.as_secs_f64() / 1_000_000.0;
            refreshed.push((sampled_at, current_energy_uj));
        }

        for (domain, previous) in self.domains.iter_mut().zip(refreshed) {
            domain.previous = Some(previous);
        }

        Ok(BackendSample::Watts(total_watts as f32))
    }
}

fn discover_candidates(root: &Path) -> Result<Vec<Candidate>, String> {
    let entries = fs::read_dir(root)
        .map_err(|error| format!("failed to scan {}: {error}", root.display()))?;
    let mut by_package = BTreeMap::<u32, Candidate>::new();

    for entry in entries {
        let path = entry
            .map_err(|error| format!("failed to inspect {}: {error}", root.display()))?
            .path();
        let energy_path = path.join("energy_uj");
        if !energy_path.is_file() {
            continue;
        }

        let name = match fs::read_to_string(path.join("name")) {
            Ok(name) => name.trim().to_owned(),
            Err(_) => continue,
        };
        let Some(package_index) = package_index(&name) else {
            continue;
        };
        let priority = control_priority(&path);
        let candidate = Candidate {
            name,
            energy_path,
            max_energy_range_path: path.join("max_energy_range_uj"),
            priority,
        };

        select_candidate(&mut by_package, package_index, candidate);
    }

    Ok(by_package.into_values().collect())
}

fn select_candidate(
    by_package: &mut BTreeMap<u32, Candidate>,
    package_index: u32,
    candidate: Candidate,
) {
    match by_package.get(&package_index) {
        Some(existing) if existing.priority <= candidate.priority => {}
        _ => {
            by_package.insert(package_index, candidate);
        }
    }
}

fn control_priority(path: &Path) -> u8 {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    if name.starts_with("intel-rapl:") {
        0
    } else if name.starts_with("intel-rapl-mmio:") {
        1
    } else {
        2
    }
}

fn package_index(name: &str) -> Option<u32> {
    name.strip_prefix("package-")?.parse().ok()
}

fn energy_delta(previous: u64, current: u64, max_range: Option<u64>) -> Result<u128, String> {
    if current >= previous {
        return Ok(u128::from(current - previous));
    }

    let max_range = max_range
        .filter(|max_range| *max_range > previous)
        .ok_or_else(|| {
            "energy counter moved backwards and max_energy_range_uj is unavailable".to_owned()
        })?;
    Ok(u128::from(current) + u128::from(max_range) - u128::from(previous))
}

fn read_u64(path: &Path) -> Result<u64, String> {
    fs::read_to_string(path)
        .map_err(|error| error.to_string())?
        .trim()
        .parse()
        .map_err(|error| format!("invalid integer in {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use super::{
        BackendSample, Candidate, energy_delta, package_index, probe_from, select_candidate,
    };

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "multimeter-rapl-test-{}-{sequence}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("test directory should be created");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn calculates_regular_and_wrapped_energy_deltas() {
        assert_eq!(energy_delta(100, 250, Some(1_000)), Ok(150));
        assert_eq!(energy_delta(900, 50, Some(1_000)), Ok(150));
        assert!(energy_delta(900, 50, None).is_err());
    }

    #[test]
    fn selects_only_numeric_package_domains() {
        assert_eq!(package_index("package-0"), Some(0));
        assert_eq!(package_index("package-12"), Some(12));
        assert_eq!(package_index("psys"), None);
        assert_eq!(package_index("core"), None);
        assert_eq!(package_index("package-"), None);
    }

    #[test]
    fn prefers_msr_package_counter_over_mmio_duplicate() {
        let mut candidates = BTreeMap::new();
        select_candidate(
            &mut candidates,
            0,
            Candidate {
                name: "package-0".to_owned(),
                energy_path: PathBuf::from("mmio/energy_uj"),
                max_energy_range_path: PathBuf::from("mmio/max_energy_range_uj"),
                priority: 1,
            },
        );
        select_candidate(
            &mut candidates,
            0,
            Candidate {
                name: "package-0".to_owned(),
                energy_path: PathBuf::from("msr/energy_uj"),
                max_energy_range_path: PathBuf::from("msr/max_energy_range_uj"),
                priority: 0,
            },
        );

        assert_eq!(candidates[&0].energy_path, PathBuf::from("msr/energy_uj"));
    }

    #[test]
    fn samples_a_discovered_package_counter() {
        let directory = TestDirectory::new();
        let package = directory.0.join("intel-rapl:0");
        fs::create_dir_all(&package).expect("package directory should be created");
        fs::write(package.join("name"), "package-0\n").expect("name should be written");
        fs::write(package.join("energy_uj"), "1000000\n").expect("energy should be written");
        fs::write(package.join("max_energy_range_uj"), "100000000\n")
            .expect("maximum range should be written");

        let mut backend = probe_from(&directory.0).expect("package counter should probe");
        std::thread::sleep(Duration::from_millis(100));
        fs::write(package.join("energy_uj"), "1500000\n").expect("energy should be updated");

        match backend.sample().expect("sample should succeed") {
            BackendSample::Watts(watts) => {
                assert!((1.0..=10.0).contains(&watts), "unexpected power: {watts}");
            }
            BackendSample::TooSoon => panic!("sample interval should have elapsed"),
        }
    }
}
