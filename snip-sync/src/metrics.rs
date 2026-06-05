use prometheus::{Histogram, HistogramOpts, IntCounter, Registry};
use std::sync::Arc;

#[derive(Clone)]
pub struct Metrics {
    pub registry: Arc<Registry>,
    pub requests_total: IntCounter,
    pub request_duration_seconds: Histogram,
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

        let request_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "snip_sync_request_duration_seconds",
                "Request duration in seconds",
            )
            .buckets(vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ]),
        )?;

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
        reg.register(Box::new(request_duration_seconds.clone()))?;
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
            request_duration_seconds,
            sync_operations_total,
            library_operations_total,
            rate_limit_hits,
            auth_failures,
        })
    }

    /// Creates a fallback instance with no-op counters for when metrics
    /// initialization fails. All counters are created with dummy names
    /// that won't conflict with real metrics.
    pub fn fallback() -> Self {
        let registry = Arc::new(Registry::new());
        let dummy_counter = || {
            IntCounter::new("snip_sync_fallback_dummy", "Dummy counter for fallback")
                .unwrap_or_else(|_| IntCounter::new("snip_sync_fallback", "fallback").unwrap())
        };
        let dummy_histogram = || {
            Histogram::with_opts(HistogramOpts::new(
                "snip_sync_fallback_duration",
                "Dummy histogram for fallback",
            ))
            .unwrap_or_else(|_| {
                Histogram::with_opts(HistogramOpts::new(
                    "snip_sync_fallback_duration_v2",
                    "fallback",
                ))
                .unwrap()
            })
        };
        Self {
            registry,
            requests_total: dummy_counter(),
            request_duration_seconds: dummy_histogram(),
            sync_operations_total: dummy_counter(),
            library_operations_total: dummy_counter(),
            rate_limit_hits: dummy_counter(),
            auth_failures: dummy_counter(),
        }
    }
}

impl Default for Metrics {
    /// Creates a fallback Metrics instance with no-op counters.
    ///
    /// Used when metrics initialization fails to allow the server to continue
    /// operating without metrics collection.
    fn default() -> Self {
        Self::fallback()
    }
}
