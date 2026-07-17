//! Durable auto-sync status persistence.
//!
//! Status is a bounded, private, secret-free artifact that records
//! the outcome of each sync attempt independently of the pending
//! intent marker. It provides operational visibility and drives
//! retry scheduling through `next_attempt_at_unix_ms`.
//!
//! Status write failure must never clear pending. Status is informative
//! and may influence scheduling but is not the source of truth for
//! whether pending work exists.

use crate::auto_sync::pending_lock;
use crate::auto_sync::policy::FailureClass;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const STATUS_FILE_NAME: &str = "auto-sync-status.toml";

/// Maximum length for the `message` field to prevent unbounded growth.
const MAX_MESSAGE_LEN: usize = 512;

/// Schema version for forward-compatible migration.
const SCHEMA_VERSION: u32 = 1;

/// Durable auto-sync status.
///
/// Recorded after each sync attempt (success or failure). Provides
/// operational visibility and drives retry scheduling.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AutoSyncStatus {
    /// Schema version for forward-compatible deserialization.
    pub schema: u32,
    /// The pending generation this status corresponds to.
    pub pending_generation: u64,
    /// The generation of the last completed attempt.
    pub last_attempt_generation: u64,
    /// Unix timestamp (ms) of the last attempt.
    pub last_attempt_at_unix_ms: u64,
    /// Unix timestamp (ms) of the last successful sync.
    pub last_success_at_unix_ms: u64,
    /// Short result code (e.g. "success", "network_failure").
    pub last_result: String,
    /// Failure class code for the last attempt.
    pub last_failure_class: String,
    /// Number of consecutive failures without an intervening success.
    pub consecutive_failures: u32,
    /// Unix timestamp (ms) when the next attempt is eligible.
    /// 0 means no backoff is active.
    pub next_attempt_at_unix_ms: u64,
    /// The executor exit code for the last attempt.
    pub executor_exit_code: i32,
    /// Whether operator attention is required.
    pub attention_required: bool,
    /// Bounded human-readable message (sanitized, no secrets).
    pub message: String,
    /// CRC32 integrity over schema + generation + timestamp + result + failure_class + consecutive_failures.
    pub integrity: u32,
}

impl Default for AutoSyncStatus {
    fn default() -> Self {
        Self {
            schema: SCHEMA_VERSION,
            pending_generation: 0,
            last_attempt_generation: 0,
            last_attempt_at_unix_ms: 0,
            last_success_at_unix_ms: 0,
            last_result: String::new(),
            last_failure_class: String::new(),
            consecutive_failures: 0,
            next_attempt_at_unix_ms: 0,
            executor_exit_code: 0,
            attention_required: false,
            message: String::new(),
            integrity: 0,
        }
    }
}

/// Path to the status file within the state directory.
pub fn status_path(state_dir: &Path) -> PathBuf {
    state_dir.join(STATUS_FILE_NAME)
}

/// Read the status file. Returns `None` if not found or corrupted.
pub fn read_status(state_dir: &Path) -> Option<AutoSyncStatus> {
    let path = status_path(state_dir);
    let content = fs::read_to_string(&path).ok()?;
    let mut status: AutoSyncStatus = toml::from_str(&content).ok()?;

    // Validate integrity
    let stored = status.integrity;
    status.integrity = 0;
    let computed = compute_integrity(&status);
    if computed != stored {
        tracing::warn!(
            expected = stored,
            computed,
            "auto-sync status integrity mismatch; treating as corrupt"
        );
        return None;
    }
    status.integrity = stored;
    Some(status)
}

/// Write the status file atomically with integrity.
///
/// Ownership/permissions are equivalent to the pending marker (0o600).
/// Write failure is logged but does not propagate — status is best-effort.
pub fn write_status(state_dir: &Path, status: &AutoSyncStatus) -> Result<(), String> {
    let path = status_path(state_dir);

    let mut to_write = status.clone();
    to_write.integrity = 0;
    to_write.integrity = compute_integrity(&to_write);

    let content = toml::to_string_pretty(&to_write).map_err(|e| format!("serialize: {e}"))?;

    // Enforce bounded file size
    if content.len() > 8192 {
        return Err(format!("status file too large: {} bytes", content.len()));
    }

    pending_lock::atomic_write_unique(&path, content.as_bytes())
        .map_err(|e| format!("write: {e}"))?;

    // Best-effort fsync
    pending_lock::fsync_parent_dir(&path);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

/// Compute CRC32 integrity over behavior-driving fields.
fn compute_integrity(status: &AutoSyncStatus) -> u32 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    status.schema.hash(&mut hasher);
    status.pending_generation.hash(&mut hasher);
    status.last_attempt_generation.hash(&mut hasher);
    status.last_attempt_at_unix_ms.hash(&mut hasher);
    status.last_success_at_unix_ms.hash(&mut hasher);
    status.last_result.hash(&mut hasher);
    status.last_failure_class.hash(&mut hasher);
    status.consecutive_failures.hash(&mut hasher);
    status.next_attempt_at_unix_ms.hash(&mut hasher);
    status.executor_exit_code.hash(&mut hasher);
    status.attention_required.hash(&mut hasher);
    // Note: message is NOT included in integrity — it's informational only
    hasher.finish() as u32
}

/// Record a successful sync attempt.
pub fn record_success(
    state_dir: &Path,
    pending_generation: u64,
    message: &str,
) -> Result<(), String> {
    let now_ms = unix_now_ms();
    let mut status = read_status(state_dir).unwrap_or_default();

    status.pending_generation = pending_generation;
    status.last_attempt_generation = pending_generation;
    status.last_attempt_at_unix_ms = now_ms;
    status.last_success_at_unix_ms = now_ms;
    status.last_result = "success".to_string();
    status.last_failure_class.clear();
    status.consecutive_failures = 0;
    status.next_attempt_at_unix_ms = 0;
    status.executor_exit_code = 0;
    status.attention_required = false;
    status.message = sanitize_message(message);

    write_status(state_dir, &status)
}

/// Record a failed sync attempt.
pub fn record_failure(
    state_dir: &Path,
    pending_generation: u64,
    failure_class: FailureClass,
    exit_code: i32,
    consecutive_failures: u32,
    next_attempt_at_unix_ms: u64,
    message: &str,
) -> Result<(), String> {
    let now_ms = unix_now_ms();
    let mut status = read_status(state_dir).unwrap_or_default();

    status.pending_generation = pending_generation;
    status.last_attempt_generation = pending_generation;
    status.last_attempt_at_unix_ms = now_ms;
    status.last_result = format!("{}_failure", failure_class.as_code());
    status.last_failure_class = failure_class.as_code().to_string();
    status.consecutive_failures = consecutive_failures;
    status.next_attempt_at_unix_ms = next_attempt_at_unix_ms;
    status.executor_exit_code = exit_code;
    status.attention_required = failure_class.is_deferred()
        || matches!(
            failure_class,
            FailureClass::Authentication
                | FailureClass::Configuration
                | FailureClass::CredentialStore
                | FailureClass::Conflict
                | FailureClass::Partial
                | FailureClass::LocalPersistence
        );
    status.message = sanitize_message(message);

    write_status(state_dir, &status)
}

/// Sanitize a message string for safe persistence.
///
/// Truncates to `MAX_MESSAGE_LEN` and strips any characters that
/// could be used for log injection.
fn sanitize_message(msg: &str) -> String {
    msg.chars()
        .filter(|c| *c != '\n' && *c != '\r' && *c != '\0')
        .take(MAX_MESSAGE_LEN)
        .collect()
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_status_default() {
        let s = AutoSyncStatus::default();
        assert_eq!(s.schema, SCHEMA_VERSION);
        assert_eq!(s.consecutive_failures, 0);
        assert!(!s.attention_required);
        assert!(s.message.is_empty());
    }

    #[test]
    fn test_status_roundtrip() {
        let dir = TempDir::new().unwrap();
        let status = AutoSyncStatus {
            pending_generation: 42,
            last_attempt_generation: 42,
            last_attempt_at_unix_ms: 1000,
            last_success_at_unix_ms: 500,
            last_result: "transient_network_failure".to_string(),
            last_failure_class: "transient_network".to_string(),
            consecutive_failures: 3,
            next_attempt_at_unix_ms: 2000,
            executor_exit_code: 4,
            attention_required: false,
            message: "connection failed".to_string(),
            ..AutoSyncStatus::default()
        };
        write_status(dir.path(), &status).unwrap();
        let loaded = read_status(dir.path()).unwrap();
        assert_eq!(loaded.pending_generation, 42);
        assert_eq!(loaded.last_attempt_generation, 42);
        assert_eq!(loaded.consecutive_failures, 3);
        assert_eq!(loaded.next_attempt_at_unix_ms, 2000);
        assert_eq!(loaded.executor_exit_code, 4);
        assert!(!loaded.attention_required);
        assert_eq!(loaded.message, "connection failed");
    }

    #[test]
    fn test_status_integrity_detection() {
        let dir = TempDir::new().unwrap();
        let status = AutoSyncStatus {
            pending_generation: 1,
            last_result: "success".to_string(),
            ..AutoSyncStatus::default()
        };
        write_status(dir.path(), &status).unwrap();

        // Tamper with the file
        let path = status_path(dir.path());
        let mut content = fs::read_to_string(&path).unwrap();
        content = content.replace("pending_generation = 1", "pending_generation = 999");
        fs::write(&path, content).unwrap();

        // Should return None due to integrity mismatch
        assert!(read_status(dir.path()).is_none());
    }

    #[test]
    fn test_status_not_found() {
        let dir = TempDir::new().unwrap();
        assert!(read_status(dir.path()).is_none());
    }

    #[test]
    fn test_record_success() {
        let dir = TempDir::new().unwrap();
        record_success(dir.path(), 1, "sync completed").unwrap();
        let status = read_status(dir.path()).unwrap();
        assert_eq!(status.pending_generation, 1);
        assert_eq!(status.last_result, "success");
        assert_eq!(status.consecutive_failures, 0);
        assert!(!status.attention_required);
        assert_eq!(status.message, "sync completed");
    }

    #[test]
    fn test_record_failure() {
        let dir = TempDir::new().unwrap();
        record_failure(
            dir.path(),
            1,
            FailureClass::TransientNetwork,
            4,
            1,
            5000,
            "connection failed",
        )
        .unwrap();
        let status = read_status(dir.path()).unwrap();
        assert_eq!(status.pending_generation, 1);
        assert_eq!(status.last_result, "transient_network_failure");
        assert_eq!(status.last_failure_class, "transient_network");
        assert_eq!(status.consecutive_failures, 1);
        assert_eq!(status.next_attempt_at_unix_ms, 5000);
        assert_eq!(status.executor_exit_code, 4);
        assert!(!status.attention_required);
    }

    #[test]
    fn test_record_failure_attention_required() {
        let dir = TempDir::new().unwrap();
        record_failure(
            dir.path(),
            1,
            FailureClass::Authentication,
            3,
            1,
            0,
            "bad api key",
        )
        .unwrap();
        let status = read_status(dir.path()).unwrap();
        assert!(status.attention_required);
    }

    #[test]
    fn test_sanitize_message_strips_newlines() {
        let sanitized = sanitize_message("line1\nline2\rline3\0line4");
        assert_eq!(sanitized, "line1line2line3line4");
    }

    #[test]
    fn test_sanitize_message_truncates() {
        let long = "x".repeat(1000);
        let sanitized = sanitize_message(&long);
        assert_eq!(sanitized.len(), MAX_MESSAGE_LEN);
    }

    #[test]
    fn test_status_file_permissions() {
        let dir = TempDir::new().unwrap();
        let status = AutoSyncStatus::default();
        write_status(dir.path(), &status).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(status_path(dir.path())).unwrap();
            let mode = meta.permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn test_status_write_does_not_exist_before_first_write() {
        let dir = TempDir::new().unwrap();
        assert!(!status_path(dir.path()).exists());
        write_status(dir.path(), &AutoSyncStatus::default()).unwrap();
        assert!(status_path(dir.path()).exists());
    }
}
