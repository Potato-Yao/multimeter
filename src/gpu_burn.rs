use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;
use wgpu::util::DeviceExt;

const BUFFER_BYTES: u64 = 16 * 1024 * 1024;
const ELEMENT_COUNT: u32 = (BUFFER_BYTES / size_of::<u32>() as u64) as u32;
const ROUNDS_PER_DISPATCH: u32 = 256;
const WORKGROUP_SIZE: u32 = 256;
const POLL_TIMEOUT: Duration = Duration::from_millis(10);
const TARGET_SUBMISSION_TIME: Duration = Duration::from_millis(30);
const MAX_DISPATCHES_PER_SUBMISSION: u32 = 64;

/// Hardware adapter selected for a GPU test.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuTestAdapter {
    pub name: String,
    pub backend: String,
    pub device_type: String,
    pub driver: String,
    pub driver_info: String,
}

/// Aggregate result from running a GPU test.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuTestResult {
    pub adapter: GpuTestAdapter,
    pub elapsed: Duration,
    pub dispatch_count: u64,
    pub element_rounds: u64,
}

/// An asynchronous failure reported by a running GPU test.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GpuTestFailure {
    pub message: String,
}

/// Controls how long a GPU test runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GpuTestMode {
    Finite(Duration),
    Infinite,
}

/// A running GPU test that can be stopped externally.
pub struct GpuTestRun {
    started: Instant,
    adapter: GpuTestAdapter,
    cancelled: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<GpuWorkerResult>>>,
    failure: Option<oneshot::Receiver<GpuTestFailure>>,
}

impl GpuTestRun {
    pub fn adapter(&self) -> &GpuTestAdapter {
        &self.adapter
    }

    /// Takes the failure receiver used to detect device loss while the test runs.
    pub fn take_failure_receiver(&mut self) -> Option<oneshot::Receiver<GpuTestFailure>> {
        self.failure.take()
    }

    /// Signals the worker to stop and returns the work completed so far.
    pub fn stop(mut self) -> io::Result<GpuTestResult> {
        self.cancelled.store(true, Ordering::Release);
        self.finish()
    }

    fn finish(&mut self) -> io::Result<GpuTestResult> {
        let worker = self.worker.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "GPU test is already stopped")
        })?;
        let worker_result = worker
            .join()
            .map_err(|_| io::Error::other("GPU test worker panicked"))??;

        Ok(GpuTestResult {
            adapter: self.adapter.clone(),
            elapsed: self.started.elapsed(),
            dispatch_count: worker_result.dispatch_count,
            element_rounds: worker_result.element_rounds,
        })
    }
}

impl Drop for GpuTestRun {
    fn drop(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };

        self.cancelled.store(true, Ordering::Release);
        let _ = worker.join();
    }
}

/// Starts a headless wgpu compute test and returns once initialization completes.
///
/// The call waits until a suitable hardware adapter and compute pipeline are ready.
/// Use [`GpuTestMode::Infinite`] for an open-ended burn and call
/// [`GpuTestRun::stop`] when it should end.
pub fn start_gpu_test(mode: GpuTestMode) -> io::Result<GpuTestRun> {
    let cancelled = Arc::new(AtomicBool::new(false));
    let (initialized_tx, initialized_rx) = mpsc::sync_channel(1);
    let (failure_tx, failure_rx) = oneshot::channel();
    let worker_cancelled = Arc::clone(&cancelled);

    let worker = thread::Builder::new()
        .name("multimeter-gpu-test".to_owned())
        .spawn(move || {
            let runtime = match GpuRuntime::create() {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = initialized_tx.send(Err(error));
                    return Ok(GpuWorkerResult::default());
                }
            };
            let adapter = runtime.adapter.clone();
            let started = Instant::now();
            if initialized_tx.send(Ok((adapter, started))).is_err() {
                return Ok(GpuWorkerResult::default());
            }

            let result = run_worker(&runtime, mode, started, &worker_cancelled);
            if let Err(error) = &result {
                let _ = failure_tx.send(GpuTestFailure {
                    message: error.to_string(),
                });
            }
            result
        })?;

    let (adapter, started) = match initialized_rx.recv() {
        Ok(Ok(initialized)) => initialized,
        Ok(Err(error)) => {
            let _ = worker.join();
            return Err(error);
        }
        Err(error) => {
            let _ = worker.join();
            return Err(io::Error::other(format!(
                "GPU test worker stopped during initialization: {error}"
            )));
        }
    };

    Ok(GpuTestRun {
        started,
        adapter,
        cancelled,
        worker: Some(worker),
        failure: Some(failure_rx),
    })
}

#[derive(Default)]
struct GpuWorkerResult {
    dispatch_count: u64,
    element_rounds: u64,
}

struct GpuRuntime {
    adapter: GpuTestAdapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    element_count: u32,
    rounds_per_dispatch: u32,
}

impl GpuRuntime {
    fn create() -> io::Result<Self> {
        Self::create_with_options(false, ELEMENT_COUNT, ROUNDS_PER_DISPATCH)
    }

    fn create_with_options(
        allow_software: bool,
        element_count: u32,
        rounds_per_dispatch: u32,
    ) -> io::Result<Self> {
        if element_count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "GPU test requires at least one state element",
            ));
        }
        if rounds_per_dispatch == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "GPU test rounds per dispatch must be greater than zero",
            ));
        }

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|error| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                format!("no compatible GPU adapter: {error}"),
            )
        })?;
        let info = adapter.get_info();
        if !allow_software && matches!(info.device_type, wgpu::DeviceType::Cpu) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("selected GPU adapter is software-only: {}", info.name),
            ));
        }
        if !adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "GPU adapter does not support compute shaders: {}",
                    info.name
                ),
            ));
        }

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("multimeter GPU test device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            trace: wgpu::Trace::Off,
        }))
        .map_err(|error| io::Error::other(format!("failed to create GPU device: {error}")))?;

        let shader = device.create_shader_module(wgpu::include_wgsl!("gpu_burn.wgsl"));
        let params = [rounds_per_dispatch, element_count, 0_u32, 0_u32];
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("multimeter GPU test parameters"),
            contents: bytemuck::cast_slice(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let state_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("multimeter GPU test state"),
            size: u64::from(element_count) * size_of::<u32>() as u64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("multimeter GPU test bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("multimeter GPU test bind group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: state_buffer.as_entire_binding(),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("multimeter GPU test pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let error_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("multimeter GPU test pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("burn"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        if let Some(error) = pollster::block_on(error_scope.pop()) {
            return Err(io::Error::other(format!(
                "failed to create GPU compute pipeline: {error}"
            )));
        }

        Ok(Self {
            adapter: GpuTestAdapter {
                name: info.name,
                backend: backend_name(info.backend),
                device_type: device_type_name(info.device_type),
                driver: info.driver,
                driver_info: info.driver_info,
            },
            device,
            queue,
            pipeline,
            bind_group,
            element_count,
            rounds_per_dispatch,
        })
    }
}

fn run_worker(
    runtime: &GpuRuntime,
    mode: GpuTestMode,
    started: Instant,
    cancelled: &AtomicBool,
) -> io::Result<GpuWorkerResult> {
    let deadline = match mode {
        GpuTestMode::Finite(duration) => Some(started.checked_add(duration).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "GPU test duration exceeds the supported range",
            )
        })?),
        GpuTestMode::Infinite => None,
    };
    let mut result = GpuWorkerResult::default();

    if !should_continue(cancelled, deadline) {
        return Ok(result);
    }

    submit_compute_batch(runtime, 1)?;
    record_dispatches(runtime, &mut result, 1);
    if !should_continue(cancelled, deadline) {
        return Ok(result);
    }

    let calibration_started = Instant::now();
    submit_compute_batch(runtime, 1)?;
    record_dispatches(runtime, &mut result, 1);
    let calibration_nanos = calibration_started.elapsed().as_nanos().max(1);
    let dispatches_per_submission = u32::try_from(
        TARGET_SUBMISSION_TIME
            .as_nanos()
            .div_ceil(calibration_nanos)
            .min(u128::from(MAX_DISPATCHES_PER_SUBMISSION)),
    )
    .unwrap_or(MAX_DISPATCHES_PER_SUBMISSION)
    .max(1);

    while should_continue(cancelled, deadline) {
        submit_compute_batch(runtime, dispatches_per_submission)?;
        record_dispatches(runtime, &mut result, dispatches_per_submission);
    }

    Ok(result)
}

fn should_continue(cancelled: &AtomicBool, deadline: Option<Instant>) -> bool {
    !cancelled.load(Ordering::Acquire) && deadline.is_none_or(|deadline| Instant::now() < deadline)
}

fn submit_compute_batch(runtime: &GpuRuntime, dispatch_count: u32) -> io::Result<()> {
    let mut encoder = runtime
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("multimeter GPU test encoder"),
        });
    for _ in 0..dispatch_count {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("multimeter GPU test pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&runtime.pipeline);
        pass.set_bind_group(0, &runtime.bind_group, &[]);
        pass.dispatch_workgroups(runtime.element_count.div_ceil(WORKGROUP_SIZE), 1, 1);
    }

    let submission = runtime.queue.submit([encoder.finish()]);
    wait_for_submission(&runtime.device, submission)
}

fn record_dispatches(runtime: &GpuRuntime, result: &mut GpuWorkerResult, dispatch_count: u32) {
    result.dispatch_count = result
        .dispatch_count
        .saturating_add(u64::from(dispatch_count));
    result.element_rounds = result.element_rounds.saturating_add(
        u64::from(dispatch_count)
            * u64::from(runtime.element_count)
            * u64::from(runtime.rounds_per_dispatch),
    );
}

fn wait_for_submission(
    device: &wgpu::Device,
    submission_index: wgpu::SubmissionIndex,
) -> io::Result<()> {
    loop {
        match device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission_index.clone()),
            timeout: Some(POLL_TIMEOUT),
        }) {
            Ok(_) => return Ok(()),
            Err(wgpu::PollError::Timeout) => {}
            Err(error) => {
                return Err(io::Error::other(format!(
                    "GPU test submission failed: {error}"
                )));
            }
        }
    }
}

fn backend_name(backend: wgpu::Backend) -> String {
    match backend {
        wgpu::Backend::Dx12 => "dx12".to_owned(),
        wgpu::Backend::Gl => "gles".to_owned(),
        wgpu::Backend::Vulkan => "vulkan".to_owned(),
        other => format!("{other:?}").to_lowercase(),
    }
}

fn device_type_name(device_type: wgpu::DeviceType) -> String {
    match device_type {
        wgpu::DeviceType::DiscreteGpu => "discrete_gpu".to_owned(),
        wgpu::DeviceType::IntegratedGpu => "integrated_gpu".to_owned(),
        wgpu::DeviceType::VirtualGpu => "virtual_gpu".to_owned(),
        wgpu::DeviceType::Cpu => "cpu".to_owned(),
        wgpu::DeviceType::Other => "other".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{GpuRuntime, GpuTestMode, run_worker, start_gpu_test};
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    #[test]
    fn burn_shader_is_valid_wgsl() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("gpu_burn.wgsl"))
            .expect("GPU burn shader should parse");
        let mut validator = wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        );
        validator
            .validate(&module)
            .expect("GPU burn shader should validate");
    }

    #[test]
    fn executes_pipeline_on_an_available_adapter() {
        let runtime = match GpuRuntime::create_with_options(true, 256, 4) {
            Ok(runtime) => runtime,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Unsupported | std::io::ErrorKind::NotFound
                ) =>
            {
                return;
            }
            Err(error) => panic!("GPU test pipeline creation failed unexpectedly: {error}"),
        };
        let result = run_worker(
            &runtime,
            GpuTestMode::Finite(Duration::from_millis(1)),
            Instant::now(),
            &AtomicBool::new(false),
        )
        .expect("GPU test pipeline should execute");

        assert!(result.dispatch_count > 0);
        assert!(result.element_rounds > 0);
    }

    #[test]
    fn finite_test_stops_or_reports_missing_hardware() {
        let run = match start_gpu_test(GpuTestMode::Finite(Duration::from_millis(20))) {
            Ok(run) => run,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Unsupported | std::io::ErrorKind::NotFound
                ) =>
            {
                return;
            }
            Err(error) => panic!("GPU test failed unexpectedly: {error}"),
        };
        let adapter = run.adapter().clone();
        let result = run.stop().expect("GPU test should stop");

        assert_eq!(result.adapter, adapter);
        assert!(result.elapsed >= Duration::from_millis(20));
        assert!(result.dispatch_count > 0);
        assert!(result.element_rounds > 0);
    }
}
