use prometheus::{IntCounter, Registry};
use std::sync::Arc;

#[derive(Clone)]
pub struct Metrics {
    pub registry: Arc<Registry>,
    pub requests_total: IntCounter,
    pub sync_operations_total: IntCounter,
    pub library_operations_total: IntCounter,
    pub rate_limit_hits: IntCounter,
    pub auth_failures: IntCounter,
}

impl Metrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Arc::new(Registry::new());

        let requests_total =
            IntCounter::new("snip_sync_requests_total", "Total number of requests")?;

        let sync_operations_total = IntCounter::new(
            "snip_sync_sync_operations_total",
            "Total number of sync operations",
        )?;

        let library_operations_total = IntCounter::new(
            "snip_sync_library_operations_total",
            "Total number of library operations (create, list, delete)",
        )?;

        let rate_limit_hits = IntCounter::new(
            "snip_sync_rate_limit_hits_total",
            "Total number of rate limit hits",
        )?;

        let auth_failures = IntCounter::new(
            "snip_sync_auth_failures_total",
            "Total number of authentication failures",
        )?;

        let reg = registry.clone();
        reg.register(Box::new(requests_total.clone()))?;
        let reg = registry.clone();
        reg.register(Box::new(sync_operations_total.clone()))?;
        let reg = registry.clone();
        reg.register(Box::new(library_operations_total.clone()))?;
        let reg = registry.clone();
        reg.register(Box::new(rate_limit_hits.clone()))?;
        let reg = registry.clone();
        reg.register(Box::new(auth_failures.clone()))?;

        Ok(Self {
            registry,
            requests_total,
            sync_operations_total,
            library_operations_total,
            rate_limit_hits,
            auth_failures,
        })
    }
}

impl Default for Metrics {
    /// Creates a default Metrics instance.
    ///
    /// # Panics
    ///
    /// Panics if prometheus metrics cannot be created. This should only happen
    /// if there's a fundamental issue with the prometheus library.
    fn default() -> Self {
        Self::new().expect("Failed to create metrics (this should never happen)")
    }
}
