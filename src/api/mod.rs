mod protocol;
mod server;
mod source;

pub use crate::model::{
    ComponentInfo, CpuInfo, DiskInfo, GpuInfo, HardwareSnapshot, InfoSection, InfoWarning,
    LoadAverage, MemoryInfo, NetworkInfo, PlatformInfo, SwapInfo, SystemInfo, normalize_sections,
};
pub use protocol::{
    ApiErrorPayload, ClientMessage, InfoQueryParams, MAX_INTERVAL_MS, MIN_INTERVAL_MS,
    PROTOCOL_VERSION, ProviderCapabilities, ProviderStatus, SeekingConfig, SeekingLimits,
    SeekingStartParams, SessionState,
};
pub use server::{ApiBootstrap, ApiError, ApiOptions, ApiServer};
pub use source::{InfoSource, ProviderError, UnavailableInfoSource};
