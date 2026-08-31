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

use crate::auto_sync::policy::FailureClass;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

pub const STATUS_FILE_NAME: &str = "auto-sync-status.toml";

/// Maximum length for the `message` field to prevent unbounded growth.
const MAX_MESSAGE_LEN: usize = 512;

/// Schema version for forward-compatible migration.
const SCHEMA_VERSION: u32 = 1;

static SECRET_ASSIGNMENT_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r#"(?i)((?:api[_-]?key|token|password|passwd|secret|credential|authorization)\s*=\s*)(?:"[^"]*"|'[^']*'|[^\s&;]+)"#,
    )
    .expect("secret assignment pattern is valid")
});
static BEARER_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?i)\b(Bearer)\s+[^\s"']+"#).expect("bearer pattern is valid")
});
static URL_CREDENTIALS_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?i)(https?://)[^/\s:@]+(?::[^/\s@]*)?@"#)
        .expect("URL credential pattern is valid")
});

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
    /// Configuration fingerprint at the time of the last failure.
    /// Used to detect when config changes should release deferred failures.
    /// Contains only non-secret structural inputs (server URL hash, flags).
    #[serde(default)]
    pub config_fingerprint: u64,
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
            config_fingerprint: 0,
            integrity: 0,
        }
    }
}

/// Path to the status file within the state directory.
pub fn status_path(state_dir: &Path) -> PathBuf {
    state_dir.join(STATUS_FILE_NAME)
}

/// Typed result of reading the status file.
#[derive(Debug)]
pub enum StatusRead {
    /// Status file does not exist.
    Missing,
    /// Status file exists and is valid.
    Valid(AutoSyncStatus),
    /// Status file exists but is corrupted or unreadable.
    Corrupt(String),
}

/// Read the status file with typed error handling.
pub fn read_status_typed(state_dir: &Path) -> StatusRead {
    let path = status_path(state_dir);
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return StatusRead::Missing,
        Err(e) => return StatusRead::Corrupt(format!("read error: {e}")),
    };
    let mut status: AutoSyncStatus = match toml::from_str(&content) {
        Ok(s) => s,
        Err(e) => return StatusRead::Corrupt(format!("parse error: {e}")),
    };

    // Reject unknown schema versions — a file written by a newer version
    // must not be silently processed as if current.
    if status.schema > SCHEMA_VERSION {
        return StatusRead::Corrupt(format!(
            "unsupported schema version {}: expected <= {}",
            status.schema, SCHEMA_VERSION
        ));
    }

    // Validate integrity
    let stored = status.integrity;
    status.integrity = 0;
    let computed = compute_integrity(&status);
    if computed != stored {
        return StatusRead::Corrupt(format!(
            "integrity mismatch: stored={stored:#010x} computed={computed:#010x}"
        ));
    }
    status.integrity = stored;
    StatusRead::Valid(status)
}

/// Read the status file. Returns `None` if not found or corrupted.
pub fn read_status(state_dir: &Path) -> Option<AutoSyncStatus> {
    match read_status_typed(state_dir) {
        StatusRead::Valid(s) => Some(s),
        _ => None,
    }
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

    crate::utils::atomic::atomic_write_bytes(
        &path,
        content.as_bytes(),
        crate::utils::atomic::Durability::DurableUserData,
    )
    .map_err(|e| format!("write: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

/// Compute CRC32 integrity over all behavior-driving fields.
fn compute_integrity(status: &AutoSyncStatus) -> u32 {
    use std::hash::Hash;

    let mut hasher = crc32fast::Hasher::new();
    status.schema.hash(&mut hasher);
    status.pending_generation.to_le_bytes().hash(&mut hasher);
    status
        .last_attempt_generation
        .to_le_bytes()
        .hash(&mut hasher);
    status
        .last_attempt_at_unix_ms
        .to_le_bytes()
        .hash(&mut hasher);
    status
        .last_success_at_unix_ms
        .to_le_bytes()
        .hash(&mut hasher);
    status.last_result.hash(&mut hasher);
    status.last_failure_class.hash(&mut hasher);
    status.consecutive_failures.to_le_bytes().hash(&mut hasher);
    status
        .next_attempt_at_unix_ms
        .to_le_bytes()
        .hash(&mut hasher);
    status.executor_exit_code.to_le_bytes().hash(&mut hasher);
    status.attention_required.hash(&mut hasher);
    status.config_fingerprint.to_le_bytes().hash(&mut hasher);
    // Note: message is NOT included in integrity — it's informational only
    hasher.finalize()
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
    config_fingerprint: u64,
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
        || matches!(failure_class, FailureClass::LocalFailure)
        || (matches!(failure_class, FailureClass::Internal) && consecutive_failures >= 3);
    status.message = sanitize_message(message);
    status.config_fingerprint = config_fingerprint;

    write_status(state_dir, &status)
}

/// Sanitize a message string for safe persistence.
///
/// Truncates to `MAX_MESSAGE_LEN`, strips control characters, and
/// redacts potential secrets (API keys, bearer tokens, URLs with credentials).
fn sanitize_message(msg: &str) -> String {
    let redacted = redact_secrets(msg);
    redacted
        .chars()
        .filter(|c| *c != '\n' && *c != '\r' && *c != '\0')
        .take(MAX_MESSAGE_LEN)
        .collect()
}

/// Redact potential secrets from a message string.
///
/// Strips API keys, bearer tokens, and URLs with embedded credentials.
/// Uses simple pattern matching — this is best-effort redaction, not a
/// security boundary.
pub(crate) fn redact_secrets(msg: &str) -> String {
    let result = SECRET_ASSIGNMENT_RE.replace_all(msg, "$1[REDACTED]");
    let result = BEARER_RE.replace_all(&result, "$1 [REDACTED]");
    URL_CREDENTIALS_RE
        .replace_all(&result, "$1[REDACTED]@")
        .into_owned()
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_else(|_| {
            tracing::warn!(
                "system clock is before UNIX epoch; auto-sync status timestamps will \
                 report 0 and sync will fail with ClockSkew until the clock is corrected"
            );
            0
        })
}

/// Compute a configuration fingerprint from sync settings.
///
/// Contains only non-secret structural inputs: server URL, enabled flags,
/// sync direction, and a credential revision counter. The API key value is
/// NOT included — only the monotonically-increasing `credential_revision`
/// counter captures key replacement.
pub fn compute_config_fingerprint(settings: &crate::config::SyncSettings) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    settings.server_url.hash(&mut hasher);
    settings.enabled.hash(&mut hasher);
    settings.auto_sync.hash(&mut hasher);
    format!("{:?}", settings.sync_direction).hash(&mut hasher);
    settings.credential_revision.hash(&mut hasher);
    hasher.finish()
}

/// Check if configuration has changed since the last deferred failure.
///
/// If the config fingerprint differs from the one stored in the status,
/// the deferral is released by clearing `attention_required` and resetting
/// `consecutive_failures` to 0, permitting a new attempt. Returns `true`
/// if the deferral was released.
pub fn release_deferral_on_config_change(state_dir: &Path, current_fingerprint: u64) -> bool {
    let _execution_lock = match crate::auto_sync::execution_lock::try_acquire(state_dir) {
        Ok(lock) => lock,
        Err(error) => {
            tracing::debug!(%error, "status deferral release skipped while worker owns execution lock");
            return false;
        }
    };

    let mut status = match read_status_typed(state_dir) {
        StatusRead::Valid(s) => s,
        StatusRead::Corrupt(e) => {
            tracing::warn!(error = %e, "corrupt status cannot release deferral");
            return false;
        }
        StatusRead::Missing => return false,
    };

    if !status.attention_required || status.config_fingerprint == 0 {
        return false;
    }

    if status.config_fingerprint == current_fingerprint {
        return false;
    }

    // Config changed — release the deferral
    tracing::info!(
        old_fingerprint = status.config_fingerprint,
        new_fingerprint = current_fingerprint,
        "config changed; releasing deferred failure"
    );
    status.attention_required = false;
    status.consecutive_failures = 0;
    status.next_attempt_at_unix_ms = 0;
    status.config_fingerprint = current_fingerprint;
    let _ = write_status(state_dir, &status);
    true
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
    fn test_status_rejects_future_schema_version() {
        let dir = TempDir::new().unwrap();
        // Write a status file with schema = 999 (future version)
        let content = r#"
schema = 999
pending_generation = 1
last_attempt_generation = 1
last_attempt_at_unix_ms = 0
last_success_at_unix_ms = 0
last_result = "success"
last_failure_class = ""
consecutive_failures = 0
next_attempt_at_unix_ms = 0
executor_exit_code = 0
attention_required = false
message = ""
config_fingerprint = 0
integrity = 0
"#;
        std::fs::write(status_path(dir.path()), content).unwrap();

        match read_status_typed(dir.path()) {
            StatusRead::Corrupt(msg) => {
                assert!(
                    msg.contains("unsupported schema"),
                    "error should mention schema: {msg}"
                );
            }
            other => panic!("expected Corrupt for future schema, got {other:?}"),
        }
    }

    #[test]
    fn test_status_accepts_current_schema_version() {
        let dir = TempDir::new().unwrap();
        let status = AutoSyncStatus::default();
        write_status(dir.path(), &status).unwrap();
        let loaded = read_status(dir.path()).unwrap();
        assert_eq!(loaded.schema, SCHEMA_VERSION);
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
            FailureClass::Transient,
            4,
            1,
            5000,
            "connection failed",
            0,
        )
        .unwrap();
        let status = read_status(dir.path()).unwrap();
        assert_eq!(status.pending_generation, 1);
        assert_eq!(status.last_result, "transient_failure");
        assert_eq!(status.last_failure_class, "transient");
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
            FailureClass::Configuration,
            3,
            1,
            0,
            "bad api key",
            0,
        )
        .unwrap();
        let status = read_status(dir.path()).unwrap();
        assert!(status.attention_required);
    }

    #[test]
    fn test_internal_failure_requires_attention_after_three_attempts() {
        let dir = TempDir::new().unwrap();
        record_failure(
            dir.path(),
            1,
            FailureClass::Internal,
            1,
            2,
            0,
            "internal failure",
            0,
        )
        .unwrap();
        assert!(!read_status(dir.path()).unwrap().attention_required);

        record_failure(
            dir.path(),
            1,
            FailureClass::Internal,
            1,
            3,
            0,
            "internal failure",
            0,
        )
        .unwrap();
        assert!(read_status(dir.path()).unwrap().attention_required);
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
    fn test_redacts_all_repeated_secret_patterns() {
        let message = concat!(
            "Bearer first-token and Bearer second-token; ",
            "https://alice:one@example.test and https://bob:two@example.test; ",
            "api_key=first-key api_key=second-key"
        );
        let redacted = redact_secrets(message);

        for secret in [
            "first-token",
            "second-token",
            "alice:one",
            "bob:two",
            "first-key",
            "second-key",
        ] {
            assert!(!redacted.contains(secret), "secret leaked: {secret}");
        }
        assert_eq!(redacted.matches("[REDACTED]").count(), 6, "{redacted}");
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

    #[test]
    fn test_bounded_size_after_repeated_failures() {
        let dir = TempDir::new().unwrap();
        // Simulate 100 consecutive failures with long messages
        for i in 0..100 {
            let msg = format!("failure {i}: {}", "x".repeat(1000));
            record_failure(
                dir.path(),
                i as u64,
                FailureClass::Transient,
                4,
                i,
                0,
                &msg,
                0,
            )
            .unwrap();
        }
        let meta = fs::metadata(status_path(dir.path())).unwrap();
        // Status file must stay bounded (well under 8KB)
        assert!(
            meta.len() < 8192,
            "status file too large after 100 failures: {} bytes",
            meta.len()
        );
    }

    #[test]
    fn test_no_api_key_leakage_in_status() {
        let dir = TempDir::new().unwrap();
        record_failure(
            dir.path(),
            1,
            FailureClass::Configuration,
            3,
            1,
            0,
            "Bearer sk-secret-api-key-12345",
            0,
        )
        .unwrap();
        let content = fs::read_to_string(status_path(dir.path())).unwrap();
        assert!(
            !content.contains("sk-secret-api-key"),
            "status file must not contain API key"
        );
    }

    #[test]
    fn test_no_server_url_leakage_in_status() {
        let dir = TempDir::new().unwrap();
        record_failure(
            dir.path(),
            1,
            FailureClass::Transient,
            4,
            1,
            0,
            "connection to https://sync.example.com:8443 failed",
            0,
        )
        .unwrap();
        // The message field is preserved (it's informational), but the message
        // is sanitized. Server URLs in the message are OK — they're not secrets.
        let content = fs::read_to_string(status_path(dir.path())).unwrap();
        assert!(content.contains("sync.example.com"));
    }

    #[test]
    fn test_config_fingerprint_stored() {
        let dir = TempDir::new().unwrap();
        record_failure(
            dir.path(),
            1,
            FailureClass::Configuration,
            3,
            1,
            0,
            "bad key",
            42,
        )
        .unwrap();
        let status = read_status(dir.path()).unwrap();
        assert_eq!(status.config_fingerprint, 42);
    }

    #[test]
    fn test_config_fingerprint_zero_by_default() {
        let dir = TempDir::new().unwrap();
        write_status(dir.path(), &AutoSyncStatus::default()).unwrap();
        let status = read_status(dir.path()).unwrap();
        assert_eq!(status.config_fingerprint, 0);
    }

    #[test]
    fn test_release_deferral_on_config_change() {
        let dir = TempDir::new().unwrap();
        // Record an auth failure with attention_required
        record_failure(
            dir.path(),
            1,
            FailureClass::Configuration,
            3,
            1,
            0,
            "bad key",
            100, // old fingerprint
        )
        .unwrap();
        let status = read_status(dir.path()).unwrap();
        assert!(status.attention_required);
        assert_eq!(status.config_fingerprint, 100);

        // Config changed — new fingerprint
        let released = release_deferral_on_config_change(dir.path(), 200);
        assert!(released, "deferral should be released on config change");

        let status = read_status(dir.path()).unwrap();
        assert!(!status.attention_required);
        assert_eq!(status.consecutive_failures, 0);
        assert_eq!(status.config_fingerprint, 200);
    }

    #[test]
    fn test_no_release_when_config_unchanged() {
        let dir = TempDir::new().unwrap();
        record_failure(
            dir.path(),
            1,
            FailureClass::Configuration,
            3,
            1,
            0,
            "bad key",
            100,
        )
        .unwrap();

        let released = release_deferral_on_config_change(dir.path(), 100);
        assert!(
            !released,
            "deferral should NOT be released when config unchanged"
        );
    }

    #[test]
    fn test_no_release_when_no_attention_required() {
        let dir = TempDir::new().unwrap();
        // TransientNetwork doesn't set attention_required
        record_failure(
            dir.path(),
            1,
            FailureClass::Transient,
            4,
            1,
            5000,
            "connection failed",
            100,
        )
        .unwrap();
        let status = read_status(dir.path()).unwrap();
        assert!(!status.attention_required);

        let released = release_deferral_on_config_change(dir.path(), 200);
        assert!(!released);
    }

    #[test]
    fn test_release_skips_status_race_while_execution_lock_is_held() {
        let dir = TempDir::new().unwrap();
        record_failure(
            dir.path(),
            1,
            FailureClass::Configuration,
            3,
            1,
            0,
            "bad key",
            100,
        )
        .unwrap();
        let _lock = crate::auto_sync::execution_lock::try_acquire(dir.path()).unwrap();

        assert!(!release_deferral_on_config_change(dir.path(), 200));
        let status = read_status(dir.path()).unwrap();
        assert!(status.attention_required);
        assert_eq!(status.config_fingerprint, 100);
    }

    #[test]
    fn test_no_reset_on_success_preserves_fingerprint() {
        let dir = TempDir::new().unwrap();
        record_failure(
            dir.path(),
            1,
            FailureClass::Configuration,
            3,
            1,
            0,
            "bad key",
            100,
        )
        .unwrap();
        record_success(dir.path(), 1, "sync ok").unwrap();
        let status = read_status(dir.path()).unwrap();
        assert_eq!(status.consecutive_failures, 0);
        assert!(!status.attention_required);
        // Fingerprint is preserved (not cleared on success)
        assert_eq!(status.config_fingerprint, 100);
    }
}
