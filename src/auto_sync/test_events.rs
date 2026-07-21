//! Test-only event emission for worker/executor lifecycle tracking.
//!
//! When the `test-support` feature is enabled **and** the `SNP_TEST_EVENTS_DIR`
//! environment variable is set, worker and executor processes emit structured
//! JSON-lines events to `<SNP_TEST_EVENTS_DIR>/test-events.jsonl`. This allows
//! integration tests to observe lifecycle events without timing-dependent coordination.
//!
//! When the feature is not enabled, all functions are compile-time no-ops.
//! When the feature is enabled but the env var is not set, all functions are
//! runtime no-ops with zero cost.

#[cfg(feature = "test-support")]
mod inner {
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static SEQ: AtomicU64 = AtomicU64::new(1);

    /// Check if test event emission is enabled.
    pub fn enabled() -> bool {
        std::env::var("SNP_TEST_EVENTS_DIR").is_ok()
    }

    /// Return the event sink file path, if enabled.
    pub fn sink_path() -> Option<PathBuf> {
        let dir = std::env::var("SNP_TEST_EVENTS_DIR").ok()?;
        Some(Path::new(&dir).join("test-events.jsonl"))
    }

    /// Emit a lifecycle event if test event emission is enabled.
    pub fn emit(
        component: &str,
        event: &str,
        pid: u32,
        generation: Option<u64>,
        detail: Option<String>,
    ) {
        let path = match sink_path() {
            Some(p) => p,
            None => return,
        };

        let record = serde_json::json!({
            "schema": 1,
            "seq": SEQ.fetch_add(1, Ordering::Relaxed),
            "component": component,
            "event": event,
            "pid": pid,
            "generation": generation,
            "at_unix_ms": unix_ms(),
            "detail": detail,
        });

        let mut line = serde_json::to_string(&record).unwrap_or_default();
        line.push('\n');

        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }

    fn unix_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

#[cfg(not(feature = "test-support"))]
mod inner {
    /// Check if test event emission is enabled.
    pub fn enabled() -> bool {
        false
    }

    /// Return the event sink file path, if enabled.
    pub fn sink_path() -> Option<std::path::PathBuf> {
        None
    }

    /// Emit a lifecycle event — compile-time no-op without `test-support` feature.
    #[inline(always)]
    pub fn emit(
        _component: &str,
        _event: &str,
        _pid: u32,
        _generation: Option<u64>,
        _detail: Option<String>,
    ) {
    }
}

pub use inner::{emit, enabled, sink_path};
