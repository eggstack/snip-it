//! Test-only event emission for worker/executor lifecycle tracking.
//!
//! When the `SNP_TEST_EVENTS_DIR` environment variable is set, worker
//! and executor processes emit structured JSON-lines events to
//! `<SNP_TEST_EVENTS_DIR>/test-events.jsonl`. This allows integration
//! tests to observe lifecycle events without timing-dependent coordination.
//!
//! When the env var is not set, all functions are no-ops with zero cost.

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
///
/// `component` is typically `"worker"` or `"executor"`.
/// `event` is the lifecycle event name (e.g., `"started"`, `"sync_completed"`).
/// `pid` is the current process PID.
/// `generation` is an optional pending generation marker.
/// `detail` is an optional JSON-encoded extra detail string.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enabled_returns_false_without_env() {
        // Ensure the env var is not set for this test
        let _guard = EnvGuard::new("SNP_TEST_EVENTS_DIR");
        assert!(!enabled());
    }

    #[test]
    fn test_sink_path_returns_none_without_env() {
        let _guard = EnvGuard::new("SNP_TEST_EVENTS_DIR");
        assert!(sink_path().is_none());
    }

    #[test]
    fn test_emit_is_noop_without_env() {
        let _guard = EnvGuard::new("SNP_TEST_EVENTS_DIR");
        // Should not panic or create files
        emit("worker", "started", 1234, Some(1), None);
    }

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn new(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: tests run single-threaded per test, and these env
            // vars are test-only (SNP_TEST_EVENTS_DIR).
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: tests run single-threaded per test, and these env
            // vars are test-only (SNP_TEST_EVENTS_DIR).
            unsafe {
                if let Some(val) = &self.original {
                    std::env::set_var(self.key, val);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }
}
