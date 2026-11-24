use std::hint::black_box;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const DEFAULT_BATCH_SIZE: usize = 1_024;
const MULTIPLIER: f64 = 1.000_000_1;
const RESET_INTERVAL_BATCHES: u64 = 65_536;

/// Aggregate result from running a CPU test.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CpuTestResult {
    pub worker_count: usize,
    pub batch_size: usize,
    pub elapsed: Duration,
    pub multiply_count: u64,
    pub multiplies_per_second: f64,
}

/// Controls how long a CPU test runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CpuTestMode {
    Finite(Duration),
    Infinite,
}

/// A running CPU test that can be stopped externally.
pub struct CpuTestRun {
    started: Instant,
    worker_count: usize,
    batch_size: usize,
    cancelled: Arc<AtomicBool>,
    workers: Option<Vec<JoinHandle<u64>>>,
}

impl CpuTestRun {
    pub fn worker_count(&self) -> usize {
        self.worker_count
    }

    /// Signals every worker to stop and returns the operations completed so far.
    pub fn stop(mut self) -> io::Result<CpuTestResult> {
        self.cancelled.store(true, Ordering::Release);
        self.finish()
    }

    fn wait(mut self) -> io::Result<CpuTestResult> {
        self.finish()
    }

    fn finish(&mut self) -> io::Result<CpuTestResult> {
        let workers = self.workers.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "CPU test is already stopped")
        })?;
        let mut multiply_count = 0_u64;
        let mut worker_panicked = false;
        for worker in workers {
            match worker.join() {
                Ok(worker_multiply_count) => {
                    multiply_count = multiply_count.saturating_add(worker_multiply_count);
                }
                Err(_) => {
                    self.cancelled.store(true, Ordering::Release);
                    worker_panicked = true;
                }
            }
        }
        if worker_panicked {
            return Err(io::Error::other("CPU test worker panicked"));
        }

        let elapsed = self.started.elapsed();
        let elapsed_seconds = elapsed.as_secs_f64();
        let multiplies_per_second = if elapsed_seconds > 0.0 {
            multiply_count as f64 / elapsed_seconds
        } else {
            0.0
        };

        Ok(CpuTestResult {
            worker_count: self.worker_count,
            batch_size: self.batch_size,
            elapsed,
            multiply_count,
            multiplies_per_second,
        })
    }
}

impl Drop for CpuTestRun {
    fn drop(&mut self) {
        let Some(workers) = self.workers.take() else {
            return;
        };

        self.cancelled.store(true, Ordering::Release);
        for worker in workers {
            let _ = worker.join();
        }
    }
}

/// Starts a CPU test without blocking and returns a handle that can stop it.
///
/// One worker is started for each logical CPU available to the process. Use
/// [`CpuTestMode::Infinite`] for an open-ended burn and call
/// [`CpuTestRun::stop`] when it should end.
pub fn start_cpu_test(mode: CpuTestMode) -> io::Result<CpuTestRun> {
    let worker_count = thread::available_parallelism().map_or(1, |count| count.get());
    start_cpu_test_with_workers(mode, worker_count, DEFAULT_BATCH_SIZE)
}

/// Saturates every logical CPU with batched floating-point multiplication.
///
/// One worker is started for each logical CPU available to the process. The
/// call blocks until `duration` has elapsed, then waits for all workers to stop.
pub fn run_cpu_test(duration: Duration) -> io::Result<CpuTestResult> {
    start_cpu_test(CpuTestMode::Finite(duration))?.wait()
}

fn start_cpu_test_with_workers(
    mode: CpuTestMode,
    worker_count: usize,
    batch_size: usize,
) -> io::Result<CpuTestRun> {
    if worker_count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CPU test requires at least one worker",
        ));
    }
    if batch_size == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "CPU test batch size must be greater than zero",
        ));
    }

    let started = Instant::now();
    let deadline = match mode {
        CpuTestMode::Finite(duration) => Some(started.checked_add(duration).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "CPU test duration exceeds the supported range",
            )
        })?),
        CpuTestMode::Infinite => None,
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut workers = Vec::with_capacity(worker_count);

    for worker_index in 0..worker_count {
        let worker_cancelled = Arc::clone(&cancelled);
        let worker = thread::Builder::new()
            .name(format!("multimeter-cpu-{worker_index}"))
            .spawn(move || run_worker(deadline, batch_size, &worker_cancelled));

        match worker {
            Ok(worker) => workers.push(worker),
            Err(error) => {
                cancelled.store(true, Ordering::Release);
                for worker in workers {
                    let _ = worker.join();
                }
                return Err(error);
            }
        }
    }

    Ok(CpuTestRun {
        started,
        worker_count,
        batch_size,
        cancelled,
        workers: Some(workers),
    })
}

fn run_worker(deadline: Option<Instant>, batch_size: usize, cancelled: &AtomicBool) -> u64 {
    let mut values = vec![1.0_f64; batch_size];
    let mut multiply_count = 0_u64;
    let mut batches_since_reset = 0_u64;

    while !cancelled.load(Ordering::Acquire)
        && deadline.is_none_or(|deadline| Instant::now() < deadline)
    {
        for value in &mut values {
            *value *= MULTIPLIER;
        }
        black_box(&mut values);

        multiply_count = multiply_count.saturating_add(batch_size as u64);
        batches_since_reset += 1;
        if batches_since_reset == RESET_INTERVAL_BATCHES {
            values.fill(1.0);
            batches_since_reset = 0;
        }
    }

    black_box(values);
    multiply_count
}

#[cfg(test)]
mod tests {
    use super::{CpuTestMode, start_cpu_test_with_workers};
    use std::time::Duration;

    #[test]
    fn runs_batched_multiplies_on_each_worker() {
        let result =
            start_cpu_test_with_workers(CpuTestMode::Finite(Duration::from_millis(20)), 2, 64)
                .expect("CPU test should start")
                .wait()
                .expect("CPU test should run");

        assert_eq!(result.worker_count, 2);
        assert_eq!(result.batch_size, 64);
        assert_eq!(result.multiply_count % 64, 0);
        assert!(result.multiply_count > 0);
        assert!(result.multiplies_per_second > 0.0);
        assert!(result.elapsed >= Duration::from_millis(20));
    }

    #[test]
    fn zero_duration_completes_without_work() {
        let result = start_cpu_test_with_workers(CpuTestMode::Finite(Duration::ZERO), 1, 64)
            .expect("zero-duration CPU test should start")
            .wait()
            .expect("zero-duration CPU test should complete");

        assert_eq!(result.multiply_count, 0);
        assert_eq!(result.multiplies_per_second, 0.0);
    }

    #[test]
    fn infinite_test_stops_when_requested() {
        let run = start_cpu_test_with_workers(CpuTestMode::Infinite, 1, 64)
            .expect("infinite CPU test should start");
        assert_eq!(run.worker_count(), 1);
        std::thread::sleep(Duration::from_millis(10));

        let result = run.stop().expect("infinite CPU test should stop");

        assert_eq!(result.worker_count, 1);
        assert!(result.multiply_count > 0);
        assert!(result.elapsed >= Duration::from_millis(10));
    }
}
