//! **Layer: Domain/Core**
//!
//! Short-lived exclusive lock for serializing local TOML mutations against
//! backup snapshot capture.
//!
//! The [`LocalDataLock`] ensures that a backup snapshot captures either the
//! complete before-state or complete after-state of all local data, never a
//! mixed state where some files reflect a mutation while others don't.
//!
//! # Usage
//!
//! - **Backup snapshot**: Acquire the lock for the entire duration of file
//!   enumeration and byte capture. Release before writing the backup output.
//! - **Snippet mutations**: Acquire the lock for the duration of the write
//!   (save_library, atomic_replace, etc.).
//! - **Library/import/restore**: Already serialized by the transaction lock;
//!   the local-data lock is not required for these paths.

use crate::error::{SnipError, SnipResult};
use std::fs;
use std::path::{Path, PathBuf};

/// Short-lived exclusive lock on local configuration data.
///
/// Held during backup snapshot capture and local TOML mutations to prevent
/// mixed-state snapshots. The lock file is created atomically and removed
/// on drop.
///
/// # Lock file location
///
/// The lock is stored at `<state_dir>/local-data.lock` where `state_dir`
/// is the `.transaction` subdirectory of the config directory.
#[derive(Debug)]
pub struct LocalDataLock {
    lock_path: PathBuf,
}

impl Drop for LocalDataLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

/// Acquire the local-data lock.
///
/// The lock is an exclusive file-based lock in the `.transaction` directory.
/// Retries with exponential backoff up to 30 seconds if the lock is held.
pub fn acquire_local_data_lock(state_dir: &Path) -> SnipResult<LocalDataLock> {
    fs::create_dir_all(state_dir)
        .map_err(|e| SnipError::io_error("create state directory", state_dir, e))?;

    let lock_path = state_dir.join("local-data.lock");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut backoff = std::time::Duration::from_millis(10);

    loop {
        // Atomic lock acquisition.
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_file) => return Ok(LocalDataLock { lock_path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::time::Instant::now() >= deadline {
                    return Err(SnipError::runtime_error(
                        "Local data lock held",
                        Some("Timed out waiting for local data lock after 30 seconds."),
                    ));
                }
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(std::time::Duration::from_secs(1));
            }
            Err(e) => {
                return Err(SnipError::io_error("acquire local data lock", lock_path, e));
            }
        }
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
}
