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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug)]
pub struct LocalDataLock {
    lock_path: PathBuf,
    info: LocalDataLockInfo,
}

impl Drop for LocalDataLock {
    fn drop(&mut self) {
        // Only remove if we still own the lock. Verify nonce, PID, and
        // start token (when present) to prevent removal by a wrong owner.
        if let Ok(content) = fs::read_to_string(&self.lock_path)
            && let Ok(existing) = toml::from_str::<LocalDataLockInfo>(&content)
            && existing.nonce == self.info.nonce
            && existing.pid == self.info.pid
            && existing.start_token == self.info.start_token
        {
            let _ = fs::remove_file(&self.lock_path);
        }
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

    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_file) => {
                // Write lock record
                let content = toml::to_string_pretty(&info)
                    .map_err(|e| SnipError::toml_error("serialize local-data lock info", e))?;
                fs::write(&lock_path, &content).map_err(|e| {
                    SnipError::io_error("write local-data lock record", lock_path.clone(), e)
                })?;
                return Ok(LocalDataLock { lock_path, info });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Lock exists — read and classify the owner.
                let content = fs::read_to_string(&lock_path).map_err(|e| {
                    SnipError::io_error("read existing local-data lock", lock_path.clone(), e)
                })?;

                let existing: LocalDataLockInfo = match toml::from_str(&content) {
                    Ok(info) => info,
                    Err(_) => {
                        // Malformed lock — quarantine, then loop back to create_new.
                        tracing::warn!("Malformed local-data lock record, quarantining");
                        quarantine_local_data_lock(&lock_path)?;
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
                        quarantine_local_data_lock(&lock_path)?;
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
                        quarantine_local_data_lock(&lock_path)?;
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
fn quarantine_local_data_lock(lock_path: &Path) -> SnipResult<PathBuf> {
    let quarantine_name = format!("local-data.lock.quarantine.{}", uuid::Uuid::new_v4());
    let quarantine_path = lock_path
        .parent()
        .unwrap_or(lock_path)
        .join(&quarantine_name);
    fs::rename(lock_path, &quarantine_path).map_err(|e| {
        SnipError::io_error(
            "quarantine stale local-data lock",
            quarantine_path.clone(),
            e,
        )
    })?;
    Ok(quarantine_path)
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

/// Derive the state directory for the local-data lock.
///
/// Returns `<config_dir>/.transaction` — the same directory used by the
/// transaction module.
pub fn derive_local_data_state_dir() -> PathBuf {
    crate::auto_sync::notification::derive_state_dir().join(".transaction")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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
    fn test_derive_local_data_state_dir() {
        let state_dir = derive_local_data_state_dir();
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
    fn test_malformed_local_data_lock_quarantined() {
        let dir = TempDir::new().unwrap();
        let lock_path = dir.path().join("local-data.lock");
        // Write malformed content
        fs::write(&lock_path, "not valid toml {{{").unwrap();
        // Acquisition should quarantine the malformed lock and succeed
        let lock = acquire_local_data_lock(dir.path()).unwrap();
        assert!(lock.lock_path.exists());
        // Quarantine file should exist
        let quarantines: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("local-data.lock.quarantine.")
            })
            .collect();
        assert_eq!(quarantines.len(), 1, "malformed lock should be quarantined");
        drop(lock);
    }
}
