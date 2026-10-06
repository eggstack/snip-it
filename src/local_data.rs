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

/// Whether a lock-file I/O error kind should be treated as transient lock
/// contention (retry) rather than a hard failure.
///
/// On Windows a just-deleted file can briefly surface `PermissionDenied`
/// while in a pending-delete state, so it retries like `AlreadyExists`.
/// On Unix a `PermissionDenied` here is a genuine misconfiguration (e.g.
/// wrong mode on the lock directory) and must fail fast instead of spinning
/// in the contention loop for ~30 s.
#[cfg(windows)]
fn is_transient_lock_contention(kind: std::io::ErrorKind) -> bool {
    kind == std::io::ErrorKind::PermissionDenied
}

/// Unix has no pending-delete aliasing: permission errors fail fast.
#[cfg(not(windows))]
fn is_transient_lock_contention(_kind: std::io::ErrorKind) -> bool {
    false
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
#[derive(Debug)]
pub struct LocalDataLock {
    lock_path: PathBuf,
    info: LocalDataLockInfo,
}

impl Drop for LocalDataLock {
    fn drop(&mut self) {
        // Only remove if we still own the lock. Verify nonce, PID, and
        // start token (when present) through an opened handle, and confirm
        // via handle metadata that the verified file is still the one at
        // the path before unlinking — same invariant as `TransactionLock`.
        crate::utils::process::remove_owned_lock_file(
            &self.lock_path,
            &self.info.nonce,
            self.info.pid,
            self.info.start_token.as_deref(),
        );
    }
}

/// Acquire the local-data lock.
///
/// The lock is an exclusive file-based lock in the `.transaction` directory.
/// Uses the same owned-lock protocol as the transaction lock: the lock file
/// contains an ownership record (PID, nonce, start_token). Dead owners are
/// reclaimed via `ProcessIdentity::observe`; live owners always block.
///
/// Retries with exponential backoff up to 30 seconds if the lock is held by
/// a live process.
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

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut backoff = std::time::Duration::from_millis(10);
    let mut empty_retries = 0u32;
    let mut malformed_retries = 0u32;

    // Pre-serialize the lock record so we can write it to the file
    // handle immediately, minimizing the empty-file window between
    // create_new succeeding and content being written.
    let content = toml::to_string_pretty(&info)
        .map_err(|e| SnipError::toml_error("serialize local-data lock info", e))?;

    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                // Write the lock record to the open file handle
                // immediately, then sync before dropping. This
                // minimizes the empty-file window. A concurrent
                // reader that sees empty content will retry instead
                // of quarantining (see below).
                use std::io::Write;
                if let Err(e) = file.write_all(content.as_bytes()) {
                    // The file was created by this call and is empty. Leaving
                    // it behind makes every later acquirer spin forever on an
                    // unparseable record, so drop it before reporting.
                    drop(file);
                    let _ = fs::remove_file(&lock_path);
                    return Err(SnipError::io_error(
                        "write local-data lock record",
                        lock_path,
                        e,
                    ));
                }
                if let Err(e) = file.sync_all() {
                    tracing::warn!(error = %e, "failed to sync local-data lock record");
                }
                return Ok(LocalDataLock { lock_path, info });
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::AlreadyExists
                // On Windows, a just-deleted file can briefly return
                // PermissionDenied when in a pending-delete state.
                // Treat it the same as AlreadyExists.
                || is_transient_lock_contention(e.kind()) =>
            {
                // Lock exists — read and classify the owner.
                // Handle TOCTOU: another writer may have removed the lock
                // between create_new failing and read_to_string.
                let content = match fs::read_to_string(&lock_path) {
                    Ok(c) => c,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::NotFound
                        // On Windows, a pending-delete file may be unreadable.
                        || is_transient_lock_contention(e.kind()) =>
                    {
                        // Lock was removed or is in a transient state — loop back and retry.
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                    Err(e) => {
                        return Err(SnipError::io_error(
                            "read existing local-data lock",
                            lock_path.clone(),
                            e,
                        ));
                    }
                };

                let existing: LocalDataLockInfo = match toml::from_str(&content) {
                    Ok(info) => {
                        // A readable record resets the ladder below: a waiter
                        // that sees scattered empty reads over a long
                        // legitimate acquisition must not eventually quarantine
                        // a perfectly good lock.
                        empty_retries = 0;
                        malformed_retries = 0;
                        info
                    }
                    Err(_) if content.trim().is_empty() => {
                        // Empty file — normally another writer just called
                        // create_new but hasn't written yet. Retry briefly,
                        // then reclaim a file left behind by a crashed writer.
                        // Without this ladder a 0-byte leftover (failed
                        // write_all, or SIGKILL between create_new and
                        // write_all) would spin forever: the record can
                        // never become valid, so no reclaim path is reachable.
                        empty_retries = empty_retries.saturating_add(1);
                        let is_old = fs::metadata(&lock_path)
                            .and_then(|m| m.modified())
                            .ok()
                            .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
                            .is_some_and(|age| age > std::time::Duration::from_millis(100));
                        if empty_retries > 200 || (empty_retries > 50 && is_old) {
                            tracing::warn!(
                                "Empty local-data lock record did not become valid; quarantining"
                            );
                            quarantine_local_data_lock(&lock_path, None)?;
                            empty_retries = 0;
                        } else if std::time::Instant::now() >= deadline {
                            return Err(SnipError::runtime_error(
                                "Local data lock held",
                                Some(
                                    "Timed out waiting for local data lock after 30 seconds; the \
                                     lock record stayed empty, which usually means a writer was \
                                     interrupted between creating and writing the lock file.",
                                ),
                            ));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                    Err(_) => {
                        // Malformed *or* partially written. A record that is
                        // only partly written looks malformed too, so it gets
                        // the same bounded ladder as the empty arm rather than
                        // being quarantined out from under a live owner.
                        malformed_retries = malformed_retries.saturating_add(1);
                        if malformed_retries > 200 {
                            tracing::warn!(
                                "Malformed local-data lock record did not become valid; quarantining"
                            );
                            quarantine_local_data_lock(&lock_path, None)?;
                            malformed_retries = 0;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                };

                // Observe the process identified by the existing lock record.
                match ProcessIdentity::observe(existing.pid) {
                    None => {
                        // Owner process is dead — reclaim immediately.
                        tracing::info!(
                            pid = existing.pid,
                            "Reclaiming stale local-data lock (owner process is dead)"
                        );
                        quarantine_local_data_lock(&lock_path, Some(&existing))?;
                        continue;
                    }
                    Some(observed) => {
                        // Owner is alive. Refuse if we cannot verify ownership
                        // (conservative policy):
                        // - existing.start_token is None (old lock without token)
                        // - observed.start_token is None (can't observe identity)
                        // - start tokens match (same process)
                        // Only reclaim when both tokens are present and differ.
                        if existing.start_token.is_none()
                            || observed.start_token.is_none()
                            || observed.start_token == existing.start_token
                        {
                            if std::time::Instant::now() >= deadline {
                                return Err(SnipError::runtime_error(
                                    "Local data lock held",
                                    Some("Timed out waiting for local data lock after 30 seconds."),
                                ));
                            }
                            std::thread::sleep(backoff);
                            backoff = (backoff * 2).min(std::time::Duration::from_secs(1));
                            continue;
                        }
                        // PID reuse detected — reclaim.
                        tracing::info!(
                            pid = existing.pid,
                            "Local-data lock owner PID reused (start token mismatch), reclaiming"
                        );
                        quarantine_local_data_lock(&lock_path, Some(&existing))?;
                        continue;
                    }
                }
            }
            Err(e) => {
                return Err(SnipError::io_error("acquire local data lock", lock_path, e));
            }
        }
    }
}

/// Quarantine a stale or malformed local-data lock by renaming it.
///
/// When `observed` is supplied, the record currently at `lock_path` must still
/// match it before the rename happens. Reading the record and checking the
/// owner's liveness are non-atomic, so without this check a writer that
/// decided to reclaim could rename away a *live* lock that another writer had
/// already reclaimed and re-acquired in the meantime — silently putting two
/// processes inside the critical section. Passing `None` (malformed or empty
/// record) skips verification because there is no owner identity to compare,
/// which is the same trade-off `Drop` makes for unparseable content.
///
/// If the lock file has already been quarantined by a concurrent writer
/// (race on stale-lock reclaim), the `NotFound` error is treated as success.
fn quarantine_local_data_lock(
    lock_path: &Path,
    observed: Option<&LocalDataLockInfo>,
) -> SnipResult<PathBuf> {
    if let Some(expected) = observed {
        match fs::read_to_string(lock_path) {
            Ok(content) => match toml::from_str::<LocalDataLockInfo>(&content) {
                Ok(current) if &current == expected => {}
                Ok(_) => {
                    // A different record is in place: the stale observation is
                    // obsolete and the current owner must not be disturbed.
                    tracing::debug!(
                        "local-data lock record changed since it was observed; not quarantining"
                    );
                    return Ok(lock_path.to_path_buf());
                }
                Err(_) => {
                    tracing::debug!(
                        "local-data lock record is no longer parseable; not quarantining"
                    );
                    return Ok(lock_path.to_path_buf());
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(SnipError::io_error(
                    "read local-data lock before quarantine",
                    lock_path.to_path_buf(),
                    e,
                ));
            }
        }
    }

    let quarantine_name = format!("local-data.lock.quarantine.{}", uuid::Uuid::new_v4());
    let quarantine_path = lock_path
        .parent()
        .unwrap_or(lock_path)
        .join(&quarantine_name);
    match fs::rename(lock_path, &quarantine_path) {
        Ok(()) => {
            // Opportunistic GC: without it, quarantine files accumulate
            // without bound under pathological spawn/kill churn.
            crate::transaction::prune_quarantine_files(
                quarantine_path.parent().unwrap_or(&quarantine_path),
                "local-data.lock.quarantine.",
                crate::transaction::QUARANTINE_RETENTION,
            );
            Ok(quarantine_path)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Another writer already quarantined the lock — treat as success.
            tracing::debug!("local-data lock already quarantined by another writer");
            Ok(quarantine_path)
        }
        Err(e) => Err(SnipError::io_error(
            "quarantine stale local-data lock",
            quarantine_path.clone(),
            e,
        )),
    }
}

/// Execute a closure while holding the local-data lock.
///
/// Acquires the lock, runs the closure, and releases the lock on completion
/// (or panic). Returns the closure's result.
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

        quarantine_local_data_lock(&lock_path, Some(&observed)).unwrap();

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

        quarantine_local_data_lock(&lock_path, Some(&observed)).unwrap();

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
