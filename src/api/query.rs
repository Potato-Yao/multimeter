use std::sync::Arc;

use crate::model::{HardwareSnapshot, InfoSection};

use super::protocol::ProviderCapabilities;
use super::source::{InfoSource, ProviderError};

/// Provides fresh hardware snapshots on demand.
///
/// The service intentionally keeps no cached values. Every call delegates to the
/// configured provider, whose implementation owns any mutable collector state.
pub struct QueryService {
    source: Arc<dyn InfoSource>,
}

impl QueryService {
    pub fn new(source: Arc<dyn InfoSource>) -> Self {
        Self { source }
    }

    pub fn capabilities(&self) -> ProviderCapabilities {
        self.source.capabilities()
    }

    pub fn snapshot(&self, sections: &[InfoSection]) -> Result<HardwareSnapshot, ProviderError> {
        self.source.snapshot(sections)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::model::{HardwareSnapshot, InfoSection};

    use super::super::protocol::{ProviderCapabilities, ProviderStatus};
    use super::super::source::{InfoSource, ProviderError};
    use super::QueryService;

    struct CountingSource {
        calls: AtomicUsize,
    }

    impl InfoSource for CountingSource {
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                sysinfo: ProviderStatus::available(),
                nvml: ProviderStatus::unavailable("not present"),
            }
        }

        fn snapshot(&self, _sections: &[InfoSection]) -> Result<HardwareSnapshot, ProviderError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(HardwareSnapshot::default())
        }
    }

    #[test]
    fn queries_the_provider_on_every_request() {
        let source = Arc::new(CountingSource {
            calls: AtomicUsize::new(0),
        });
        let service = QueryService::new(source.clone());

        service
            .snapshot(&[InfoSection::Cpu])
            .expect("first query should succeed");
        service
            .snapshot(&[InfoSection::Cpu])
            .expect("second query should succeed");

        assert_eq!(source.calls.load(Ordering::Relaxed), 2);
    }
}
