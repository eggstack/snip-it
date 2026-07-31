//! **Layer: Test-only**
//!
//! Test-only event emission for detached-helper lifecycle tracking.
//!
//! When the `test-support` feature is enabled and the `SNP_TEST_EVENTS_DIR`
//! environment variable is set, the worker emits structured
//! JSON-lines events to `<SNP_TEST_EVENTS_DIR>/test-events.jsonl`. This allows
//! integration tests to observe lifecycle events without timing-dependent
//! coordination.
//!
//! When the `test-support` feature is disabled (production builds), all
//! functions are compile-time no-ops — the environment variable check is
//! entirely absent from the binary.

#[cfg(feature = "test-support")]
use std::fs::OpenOptions;
#[cfg(feature = "test-support")]
use std::io::Write;
#[cfg(feature = "test-support")]
use std::path::{Path, PathBuf};
#[cfg(feature = "test-support")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "test-support")]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(feature = "test-support")]
static SEQ: AtomicU64 = AtomicU64::new(1);

/// Check if test event emission is enabled.
#[cfg(feature = "test-support")]
pub fn enabled() -> bool {
    std::env::var("SNP_TEST_EVENTS_DIR").is_ok()
}

/// Production no-op for event emission checks.
#[cfg(not(feature = "test-support"))]
#[inline(always)]
pub fn enabled() -> bool {
    false
}

/// Return the event sink file path, if enabled.
#[cfg(feature = "test-support")]
pub fn sink_path() -> Option<PathBuf> {
    let dir = std::env::var("SNP_TEST_EVENTS_DIR").ok()?;
    Some(Path::new(&dir).join("test-events.jsonl"))
}

/// Production no-op for sink path resolution.
#[cfg(not(feature = "test-support"))]
#[inline(always)]
pub fn sink_path() -> Option<std::path::PathBuf> {
    None
}

/// Emit a lifecycle event if test event emission is enabled.
#[cfg(feature = "test-support")]
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

/// Production no-op for event emission.
#[cfg(not(feature = "test-support"))]
#[inline(always)]
pub fn emit(
    _component: &str,
    _event: &str,
    _pid: u32,
    _generation: Option<u64>,
    _detail: Option<String>,
) {
}

#[cfg(feature = "test-support")]
fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
