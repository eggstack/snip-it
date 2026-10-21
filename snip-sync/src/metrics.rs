use prometheus::{HistogramOpts, HistogramVec, IntCounter, IntCounterVec, Registry};
use std::sync::Arc;

#[derive(Clone)]
pub struct Metrics {
    pub registry: Arc<Registry>,
    pub requests_total: IntCounterVec,
    pub request_duration_seconds: HistogramVec,
    pub sync_operations_total: IntCounterVec,
    pub library_operations_total: IntCounterVec,
    pub rate_limit_hits: IntCounter,
    pub auth_failures: IntCounter,
    /// Database failures observed while serving a request (for example an
    /// upsert that failed inside a write transaction).
    pub db_errors_total: IntCounter,
}

impl Metrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Arc::new(Registry::new());

        let requests_total = IntCounterVec::new(
            prometheus::opts!("snip_sync_requests_total", "Total number of requests"),
            &["method"],
        )?;

        let request_duration_seconds = HistogramVec::new(
            HistogramOpts::new(
                "snip_sync_request_duration_seconds",
                "Request duration in seconds",
            )
            .buckets(vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            ]),
            &["method"],
        )?;

        let sync_operations_total = IntCounterVec::new(
            prometheus::opts!(
                "snip_sync_sync_operations_total",
                "Total number of sync operations"
            ),
            &["direction"],
        )?;

        let library_operations_total = IntCounterVec::new(
            prometheus::opts!(
                "snip_sync_library_operations_total",
                "Total number of library operations (create, list, delete)"
            ),
            &["operation"],
        )?;

        let rate_limit_hits = IntCounter::new(
            "snip_sync_rate_limit_hits_total",
            "Total number of rate limit hits",
        )?;

        let auth_failures = IntCounter::new(
            "snip_sync_auth_failures_total",
            "Total number of authentication failures",
        )?;

        let db_errors_total = IntCounter::new(
            "snip_sync_db_errors_total",
            "Total number of database errors",
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
        let reg = registry.clone();
        reg.register(Box::new(db_errors_total.clone()))?;

        Ok(Self {
            registry,
            requests_total,
            request_duration_seconds,
            sync_operations_total,
            library_operations_total,
            rate_limit_hits,
            auth_failures,
            db_errors_total,
        })
    }

    /// Creates a fallback instance for when metrics initialization fails.
    ///
    /// The counters are registered in a fresh registry, exactly like
    /// [`Metrics::new`] does, so `/metrics` keeps exporting a usable metric
    /// set instead of answering `200 OK` with an empty body. The metric names
    /// are distinct from the real ones, so a fallback registry can never
    /// collide with the real registry.
    pub fn fallback() -> Self {
        let registry = Arc::new(Registry::new());
        // All names are valid ASCII identifiers, so these unwraps are safe.
        // A fresh registry is used each call, avoiding name collisions.
        let requests_total = IntCounterVec::new(
            prometheus::opts!("snip_sync_fallback_requests", "fallback"),
            &["method"],
        )
        .unwrap();
        let request_duration_seconds = HistogramVec::new(
            HistogramOpts::new("snip_sync_fallback_duration", "fallback"),
            &["method"],
        )
        .unwrap();
        let sync_operations_total = IntCounterVec::new(
            prometheus::opts!("snip_sync_fallback_sync_ops", "fallback"),
            &["direction"],
        )
        .unwrap();
        let library_operations_total = IntCounterVec::new(
            prometheus::opts!("snip_sync_fallback_lib_ops", "fallback"),
            &["operation"],
        )
        .unwrap();
        let rate_limit_hits = IntCounter::new("snip_sync_fallback_rate_limit", "fallback").unwrap();
        let auth_failures = IntCounter::new("snip_sync_fallback_auth_fail", "fallback").unwrap();
        let db_errors_total = IntCounter::new("snip_sync_fallback_db_errors", "fallback").unwrap();

        // Registration cannot fail: the registry is fresh and every metric
        // name is unique within it. Registering them is what makes the
        // fallback observable through `/metrics`.
        let reg = registry.clone();
        reg.register(Box::new(requests_total.clone()))
            .expect("fallback metric registration cannot fail in a fresh registry");
        let reg = registry.clone();
        reg.register(Box::new(request_duration_seconds.clone()))
            .expect("fallback metric registration cannot fail in a fresh registry");
        let reg = registry.clone();
        reg.register(Box::new(sync_operations_total.clone()))
            .expect("fallback metric registration cannot fail in a fresh registry");
        let reg = registry.clone();
        reg.register(Box::new(library_operations_total.clone()))
            .expect("fallback metric registration cannot fail in a fresh registry");
        let reg = registry.clone();
        reg.register(Box::new(rate_limit_hits.clone()))
            .expect("fallback metric registration cannot fail in a fresh registry");
        let reg = registry.clone();
        reg.register(Box::new(auth_failures.clone()))
            .expect("fallback metric registration cannot fail in a fresh registry");
        let reg = registry.clone();
        reg.register(Box::new(db_errors_total.clone()))
            .expect("fallback metric registration cannot fail in a fresh registry");

        Self {
            registry,
            requests_total,
            request_duration_seconds,
            sync_operations_total,
            library_operations_total,
            rate_limit_hits,
            auth_failures,
            db_errors_total,
        }
    }
}

impl Default for Metrics {
    /// Creates a fallback Metrics instance with counters registered in a
    /// fresh registry.
    ///
    /// Used when metrics initialization fails to allow the server to continue
    /// operating without metrics collection.
    fn default() -> Self {
        Self::fallback()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::Encoder;

    fn exposition(metrics: &Metrics) -> String {
        let mut buffer = Vec::new();
        prometheus::TextEncoder::new()
            .encode(&metrics.registry.gather(), &mut buffer)
            .unwrap();
        String::from_utf8(buffer).unwrap()
    }

    #[test]
    fn fallback_registers_its_metrics() {
        let metrics = Metrics::fallback();

        // Before anything is counted: the fallback registry still exports
        // the plain counters, so `/metrics` never answers with an empty
        // body.
        let body = exposition(&metrics);
        assert!(
            !body.is_empty(),
            "fallback registry must export a non-empty exposition"
        );
        assert!(body.contains("snip_sync_fallback_rate_limit"));
        assert!(body.contains("snip_sync_fallback_auth_fail"));
        assert!(body.contains("snip_sync_fallback_db_errors"));

        metrics.rate_limit_hits.inc();
        metrics.auth_failures.inc();
        metrics.db_errors_total.inc();
        metrics
            .requests_total
            .with_label_values(&["push_snippets"])
            .inc();

        let body = exposition(&metrics);
        assert!(
            !body.is_empty(),
            "fallback registry must export a non-empty exposition"
        );
        assert!(body.contains("snip_sync_fallback_requests"));
        assert!(body.contains("snip_sync_fallback_rate_limit"));
        assert!(body.contains("snip_sync_fallback_auth_fail"));
        assert!(body.contains("snip_sync_fallback_db_errors"));
    }

    #[test]
    fn real_registry_exposes_the_db_error_counter() {
        let metrics = Metrics::new().unwrap();
        metrics.db_errors_total.inc();
        let body = exposition(&metrics);
        assert!(body.contains("snip_sync_db_errors_total"));
    }
}
