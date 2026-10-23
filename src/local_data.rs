//! **Layer: Domain/Core**
//!
//! Short-lived exclusive lock for serializing local TOML mutations against
//! backup snapshot capture.
//!
//! The [`LocalDataLock`] ensures that a backup snapshot captures either the
//! complete before-state or complete after-state of all local data, never a
//! mixed state where some files reflect a mutation while others don't.
//!
//! Uses the same owned-lock protocol as the transaction lock: the lock file
//! contains an ownership record (PID, nonce, start_token) so that a dead
//! owner can be safely reclaimed and a live owner always blocks.
//!
//! # Usage
//!
//! - **Backup snapshot**: Acquire the lock for the entire duration of file
//!   enumeration and byte capture. Release before writing the backup output.
//! - **Snippet mutations**: Acquire the lock for the duration of the write
//!   (save_library, atomic_replace, etc.).
//! - **Library/import/restore**: Acquire the lock across the complete logical
//!   mutation, including preparation revalidation, commit, and pending
//!   finalization. Internal save functions that skip the gate are valid only
//!   while the caller holds this lock.

use crate::error::{SnipError, SnipResult};
use crate::record_lock::{
    OwnedLockGuard, OwnedLockNaming, OwnedLockOwner, OwnedLockRequest, OwnedLockWaitPolicy,
    acquire_owned_lock,
};
use crate::transaction::ProcessIdentity;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Ownership record persisted inside the local-data lock file.
///
/// Mirrors the transaction lock record so that both locks use the same
/// reclaim and release protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalDataLockInfo {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Process ID of the lock owner.
    pub pid: u32,
    /// Random nonce to prevent PID-reuse lock theft.
    pub nonce: String,
    /// Unix timestamp (ms) when the lock was created.
    pub created_at_unix_ms: i64,
    /// Start-time token for the lock owner process.
    /// `None` when the platform does not support start-time detection.
    #[serde(default)]
    pub start_token: Option<String>,
}

/// Short-lived exclusive lock on local configuration data.
///
/// Held during backup snapshot capture and local TOML mutations to prevent
/// mixed-state snapshots. The lock file contains an ownership record
/// (PID, nonce, start_token) so that a dead owner can be safely reclaimed
/// and a live owner always blocks.
///
/// # Lock file location
///
/// The lock is stored at `<state_dir>/local-data.lock` where `state_dir`
/// is the `.transaction` subdirectory of the config directory.
///
/// The guard is the shared owned-record guard; the local-data record type is
/// what makes the lock record distinguishable.
pub type LocalDataLock = OwnedLockGuard<LocalDataLockInfo>;

/// Diagnostic labels of the local-data lock file.
///
/// The shared protocol keeps the caller's exact diagnostic text so the two
/// owned-record locks stay distinguishable ("Transaction lock held" vs
/// "Local data lock held").
const LOCAL_DATA_LOCK_NAMING: OwnedLockNaming = OwnedLockNaming {
    label: "local-data lock",
    quarantine_prefix: "local-data.lock.quarantine.",
    write_operation: "write local-data lock record",
    read_operation: "read existing local-data lock",
    acquire_operation: "acquire local data lock",
    quarantine_read_operation: "read local-data lock before quarantine",
    quarantine_operation: "quarantine stale local-data lock",
    pid_reuse_message: "Local-data lock owner PID reused (start token mismatch), reclaiming",
};

/// The error reported when the local-data lock stops changing hands.
fn local_data_lock_held_error(_existing: &LocalDataLockInfo, _lock_path: &Path) -> SnipError {
    SnipError::runtime_error(
        "Local data lock held",
        Some(
            "Timed out waiting for the local data lock; the lock did \
             not change hands for 30 seconds.",
        ),
    )
}

/// The error reported when the lock record stays empty.
fn local_data_lock_empty_timeout_error(_lock_path: &Path) -> SnipError {
    SnipError::runtime_error(
        "Local data lock held",
        Some(
            "Timed out waiting for the local data lock; the lock record \
             stayed empty for 30 seconds, which usually means a writer \
             was interrupted between creating and writing the lock file.",
        ),
    )
}

/// Extract the ownership identity from a local-data lock record.
fn local_data_lock_owner(info: &LocalDataLockInfo) -> OwnedLockOwner {
    OwnedLockOwner {
        pid: info.pid,
        nonce: info.nonce.clone(),
        start_token: info.start_token.clone(),
    }
}

/// Acquire the local-data lock.
///
/// The lock is an exclusive file-based lock in the `.transaction` directory.
/// Uses the same owned-lock protocol as the transaction lock: the lock file
/// contains an ownership record (PID, nonce, start_token). Dead owners are
/// reclaimed via `ProcessIdentity::observe`; live owners always block.
///
/// The protocol itself lives in [`crate::record_lock`]; this function only
/// builds the local-data lock record and selects the **wait-for-progress**
/// wait policy: the lock is handed off serially, so the deadline measures
/// time *without progress* — it is extended every time the lock changes
/// hands — and is capped so a genuinely stuck owner still fails.
pub fn acquire_local_data_lock(state_dir: &Path) -> SnipResult<LocalDataLock> {
    fs::create_dir_all(state_dir)
        .map_err(|e| SnipError::io_error("create state directory", state_dir, e))?;

    let lock_path = state_dir.join("local-data.lock");
    let nonce = uuid::Uuid::new_v4().to_string();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let identity = ProcessIdentity::current();

    let info = LocalDataLockInfo {
        schema_version: 1,
        pid: identity.pid,
        nonce: nonce.clone(),
        created_at_unix_ms: now_ms,
        start_token: identity.start_token.clone(),
    };

    // The lock is handed off serially, one writer at a time. A flat deadline
    // fails the tail of a long but healthy queue: the tail waiter sleeps
    // through one poll interval per hand-off and is then told the lock is
    // "held". The deadline below therefore measures time *without progress* —
    // it is extended every time the lock changes hands — and is capped so a
    // genuinely stuck owner still fails.
    const BASE_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
    const MAX_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(300);
    // Poll interval ceiling. One second quantises every hand-off to whole
    // seconds, so a queue of N writers costs the tail waiter up to N
    // seconds of pure sleeping.
    const MAX_POLL_BACKOFF: std::time::Duration = std::time::Duration::from_millis(250);

    // Pre-serialize the lock record so we can write it to the file
    // handle immediately, minimizing the empty-file window between
    // create_new succeeding and content being written.
    let content = toml::to_string_pretty(&info)
        .map_err(|e| SnipError::toml_error("serialize local-data lock info", e))?;

    let request = OwnedLockRequest {
        lock_path: &lock_path,
        record: &info,
        record_content: &content,
        naming: &LOCAL_DATA_LOCK_NAMING,
        // The local-data lock waits for the lock to change hands.
        policy: OwnedLockWaitPolicy::WaitForProgress {
            base_wait: BASE_LOCK_WAIT,
            max_wait: MAX_LOCK_WAIT,
            max_backoff: MAX_POLL_BACKOFF,
            live_owner_error: local_data_lock_held_error,
            empty_timeout_error: local_data_lock_empty_timeout_error,
        },
    };

    acquire_owned_lock(&request, local_data_lock_owner)
}

/// Execute a closure while holding the local-data lock.
///
/// Acquires the lock, runs the closure, and releases the lock on completion
/// (or panic). Returns the closure's result.
// Retained: closure-scoped wrapper over `acquire_local_data_lock`, exercised
// by the local-data lock unit tests (the production paths acquire the lock
// directly).
#[allow(dead_code)]
pub fn with_local_data_lock<T>(state_dir: &Path, f: impl FnOnce() -> T) -> SnipResult<T> {
    let _lock = acquire_local_data_lock(state_dir)?;
    Ok(f())
}

/// Return the transaction directory shared by local-data locks and journals.
///
/// Returns `<config_dir>/.transaction`.
pub fn transaction_dir() -> PathBuf {
    crate::config::derive_sync_state_dir().join(".transaction")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Count quarantine files left behind in `dir`.
    fn count_quarantines(dir: &Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("local-data.lock.quarantine.")
            })
            .count()
    }

    #[test]
    fn test_acquire_and_release_local_data_lock() {
        let dir = TempDir::new().unwrap();
        let lock = acquire_local_data_lock(dir.path()).unwrap();
        let lock_path = lock.lock_path.clone();
        assert!(lock_path.exists());
        // Lock file contains valid TOML with PID, nonce, and start_token
        let content = fs::read_to_string(&lock_path).unwrap();
        let info: LocalDataLockInfo = toml::from_str(&content).unwrap();
        assert_eq!(info.schema_version, 1);
        assert_eq!(info.pid, std::process::id());
        assert!(!info.nonce.is_empty());
        drop(lock);
        assert!(!lock_path.exists());
    }

    #[test]
    fn test_local_data_lock_conflict() {
        let dir = TempDir::new().unwrap();
        let _lock1 = acquire_local_data_lock(dir.path()).unwrap();
        // Second acquire should block/retry; use a short-lived thread to test contention.
        let dir_path = dir.path().to_path_buf();
        let handle = std::thread::spawn(move || {
            // This will retry for 30s — we'll drop lock1 before that.
            acquire_local_data_lock(&dir_path)
        });
        // Give the retry a moment to attempt, then release lock1.
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(_lock1);
        // The second acquire should now succeed.
        let result = handle.join().unwrap();
        assert!(
            result.is_ok(),
            "second acquire should succeed after lock release"
        );
    }

    /// A local-data lock record written by the pre-unification build.
    ///
    /// The record format is part of the on-disk contract: a lock taken by an
    /// older build must stay recognizable to this build.
    const BASELINE_LOCAL_DATA_LOCK_RECORD: &str = "schema_version = 1\npid = 4242\nnonce = \"baseline-nonce\"\ncreated_at_unix_ms = 1700000000000\nstart_token = \"12345\"\n";

    #[test]
    fn test_baseline_local_data_lock_record_parses_and_serializes_identically() {
        let info: LocalDataLockInfo = toml::from_str(BASELINE_LOCAL_DATA_LOCK_RECORD).unwrap();
        assert_eq!(info.schema_version, 1);
        assert_eq!(info.pid, 4242);
        assert_eq!(info.nonce, "baseline-nonce");
        assert_eq!(info.created_at_unix_ms, 1_700_000_000_000);
        assert_eq!(info.start_token.as_deref(), Some("12345"));
        assert_eq!(
            toml::to_string_pretty(&info).unwrap(),
            BASELINE_LOCAL_DATA_LOCK_RECORD,
            "the lock record serialization must stay byte-identical"
        );
    }

    #[test]
    fn test_local_data_lock_timeout_error_text() {
        let record = LocalDataLockInfo {
            schema_version: 1,
            pid: 4242,
            nonce: "baseline-nonce".to_string(),
            created_at_unix_ms: 0,
            start_token: None,
        };
        assert_eq!(
            local_data_lock_held_error(&record, Path::new("/tmp/local-data.lock")).to_string(),
            "Runtime error: Local data lock held: Timed out waiting for the local data lock; \
             the lock did not change hands for 30 seconds."
        );
        assert_eq!(
            local_data_lock_empty_timeout_error(Path::new("/tmp/local-data.lock")).to_string(),
            "Runtime error: Local data lock held: Timed out waiting for the local data lock; \
             the lock record stayed empty for 30 seconds, which usually means a writer was \
             interrupted between creating and writing the lock file."
        );
    }

    #[test]
    fn test_with_local_data_lock_executes_closure() {
        let dir = TempDir::new().unwrap();
        let result = with_local_data_lock(dir.path(), || 42);
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_with_local_data_lock_releases_on_completion() {
        let dir = TempDir::new().unwrap();
        let _lock1 = acquire_local_data_lock(dir.path()).unwrap();
        let result = with_local_data_lock(dir.path(), || ());
        assert!(result.is_err());
    }

    #[test]
    fn test_transaction_dir() {
        let state_dir = transaction_dir();
        assert!(state_dir.ends_with(".transaction"));
    }

    #[test]
    fn test_wrong_nonce_cannot_remove_local_data_lock() {
        let dir = TempDir::new().unwrap();
        let lock = acquire_local_data_lock(dir.path()).unwrap();
        let lock_path = lock.lock_path.clone();
        // A different nonce cannot remove the lock
        let fake_info = LocalDataLockInfo {
            schema_version: 1,
            pid: 99999,
            nonce: "fake-nonce".to_string(),
            created_at_unix_ms: 0,
            start_token: None,
        };
        let fake_content = toml::to_string_pretty(&fake_info).unwrap();
        // Write a different nonce to simulate wrong owner
        fs::write(&lock_path, &fake_content).unwrap();
        drop(lock);
        // Lock file still exists because nonce didn't match
        assert!(lock_path.exists());
        // Clean up manually
        fs::remove_file(&lock_path).unwrap();
    }

    #[test]
    fn test_zero_byte_local_data_lock_is_reclaimed_not_spun_on() {
        // A 0-byte lock file is what a crashed writer leaves behind: the
        // record can never parse, so without a bounded ladder the acquirer
        // would spin forever instead of reclaiming.
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("local-data.lock");
        fs::write(&lock_path, "").unwrap();

        let lock = acquire_local_data_lock(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(&lock_path).unwrap(),
            toml::to_string_pretty(&lock.info).unwrap(),
            "reclaimed lock must be replaced by a live ownership record"
        );
        let quarantines = count_quarantines(dir.path());
        assert_eq!(quarantines, 1, "empty leftover lock should be quarantined");
        drop(lock);
        assert!(!lock_path.exists(), "live owner must remove its own lock");
    }

    #[test]
    fn test_quarantine_does_not_steal_a_live_lock() {
        // The record that triggered a reclaim can be replaced by a live lock
        // before the rename happens. The reclaim must then leave that lock
        // alone, or two processes end up inside the critical section.
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("local-data.lock");

        let observed = LocalDataLockInfo {
            schema_version: 1,
            pid: 99999,
            nonce: "stale-nonce".to_string(),
            created_at_unix_ms: 0,
            start_token: Some("stale-start-token".to_string()),
        };
        fs::write(&lock_path, toml::to_string_pretty(&observed).unwrap()).unwrap();

        // A different, live-looking owner now holds the path.
        let live = LocalDataLockInfo {
            schema_version: 1,
            pid: std::process::id(),
            nonce: "live-nonce".to_string(),
            created_at_unix_ms: 1,
            start_token: Some("live-start-token".to_string()),
        };
        fs::write(&lock_path, toml::to_string_pretty(&live).unwrap()).unwrap();

        crate::record_lock::quarantine_owned_lock(
            &lock_path,
            Some(&observed),
            &LOCAL_DATA_LOCK_NAMING,
        )
        .unwrap();

        assert!(lock_path.exists(), "live lock must not be quarantined away");
        assert_eq!(
            fs::read_to_string(&lock_path).unwrap(),
            toml::to_string_pretty(&live).unwrap(),
            "live record must be untouched"
        );
        assert_eq!(count_quarantines(dir.path()), 0);
    }

    #[test]
    fn test_quarantine_removes_lock_that_still_matches_observed() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("local-data.lock");
        let observed = LocalDataLockInfo {
            schema_version: 1,
            pid: 99999,
            nonce: "stale-nonce".to_string(),
            created_at_unix_ms: 0,
            start_token: Some("stale-start-token".to_string()),
        };
        fs::write(&lock_path, toml::to_string_pretty(&observed).unwrap()).unwrap();

        crate::record_lock::quarantine_owned_lock(
            &lock_path,
            Some(&observed),
            &LOCAL_DATA_LOCK_NAMING,
        )
        .unwrap();

        assert!(!lock_path.exists());
        assert_eq!(count_quarantines(dir.path()), 1);
    }

    #[test]
    fn test_malformed_local_data_lock_quarantined() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("local-data.lock");
        // Write malformed content
        fs::write(&lock_path, "not valid toml {{{").unwrap();
        // Acquisition should quarantine the malformed lock and succeed
        let lock = acquire_local_data_lock(dir.path()).unwrap();
        assert!(lock.lock_path.exists());
        assert_eq!(
            count_quarantines(dir.path()),
            1,
            "malformed lock should be quarantined"
        );
        drop(lock);
    }
}
