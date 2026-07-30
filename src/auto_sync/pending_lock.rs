//! Short-lived transaction lock for pending-marker operations.
//!
//! `PendingTxnGuard` serializes read-modify-write critical sections on the
//! pending marker. It is intentionally distinct from the long-lived worker
//! execution lock (`lock::WorkerLock`): parent mutation commands hold this
//! guard only for the minimum time needed to read, compute, and write.
//!
//! Authority for mutual exclusion is the operating system kernel; the
//! persistent lock file is diagnostic metadata only. See
//! [`crate::process_file_lock`] for the underlying primitive.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::process_file_lock::{self, ProcessFileLock, ProcessFileLockError};

pub const PENDING_TXN_LOCK_NAME: &str = "auto-sync-pending.lock";
pub const PENDING_TXN_LOCK_PURPOSE: &str = "auto-sync-pending";

#[derive(Debug)]
pub enum PendingTxnLockError {
    Io(std::io::Error),
    Busy { timeout_ms: u64 },
}

impl std::fmt::Display for PendingTxnLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Busy { timeout_ms } => {
                write!(f, "pending transaction lock busy after {timeout_ms}ms")
            }
        }
    }
}

impl std::error::Error for PendingTxnLockError {}

impl From<ProcessFileLockError> for PendingTxnLockError {
    fn from(err: ProcessFileLockError) -> Self {
        match err {
            ProcessFileLockError::Busy { .. } => Self::Busy { timeout_ms: 0 },
            ProcessFileLockError::Timeout { .. } => Self::Busy { timeout_ms: 0 },
            ProcessFileLockError::Io(e) => Self::Io(e),
            ProcessFileLockError::UnsupportedPlatform => Self::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "process file lock is not supported on this platform",
            )),
        }
    }
}

/// RAII guard for the pending transaction lock.
pub struct PendingTxnGuard {
    inner: ProcessFileLock,
}

impl PendingTxnGuard {
    pub fn nonce(&self) -> &str {
        self.inner.nonce()
    }
}

impl std::fmt::Debug for PendingTxnGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingTxnGuard")
            .field("path", &self.inner.path())
            .field("nonce", &self.inner.nonce())
            .finish()
    }
}

pub fn pending_txn_lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join(PENDING_TXN_LOCK_NAME)
}

/// Acquires the pending transaction lock with bounded retry.
///
/// Polls the kernel lock every 100 ms up to `timeout`. Live owners are
/// never reclaimed — the kernel alone arbitrates. Once the kernel lock is
/// acquired the metadata is overwritten with the new acquirer's identity.
/// An unreadable, empty, or malformed file does **not** block acquisition:
/// it is overwritten by the next acquirer.
pub fn acquire_pending_txn(
    state_dir: &Path,
    timeout: Duration,
) -> Result<PendingTxnGuard, PendingTxnLockError> {
    let path = pending_txn_lock_path(state_dir);
    let inner = process_file_lock::wait_acquire(&path, PENDING_TXN_LOCK_PURPOSE, timeout)?;
    Ok(PendingTxnGuard { inner })
}

/// Generates a unique temporary file path in the same directory as `final_path`.
///
/// The name includes the PID and a nanosecond timestamp to prevent conflicts
/// between concurrent writers.
pub fn unique_temp_path(final_path: &Path) -> PathBuf {
    let parent = final_path.parent().unwrap_or_else(|| Path::new("."));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let stem = final_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("auto-sync-pending");
    let ext = final_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("toml");
    parent.join(format!(".{stem}-{}-{nanos}.{ext}.tmp", std::process::id()))
}

/// Atomically writes bytes to `path` via a unique temporary file in the same
/// directory, then renames over the target. Returns the temp path for
/// diagnostics.
pub fn atomic_write_unique(final_path: &Path, bytes: &[u8]) -> Result<PathBuf, std::io::Error> {
    if let Some(parent) = final_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = unique_temp_path(final_path);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    replace_existing(&tmp, final_path)?;
    Ok(tmp)
}

/// Rename a temporary file over the target. Windows needs an explicit
/// replace-existing flag; plain `std::fs::rename` fails when the target exists.
#[cfg(unix)]
fn replace_existing(from: &Path, to: &Path) -> Result<(), std::io::Error> {
    std::fs::rename(from, to)
}

#[cfg(windows)]
fn replace_existing(from: &Path, to: &Path) -> Result<(), std::io::Error> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };

    let from_wide: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to_wide: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Attempts to fsync the parent directory. Best-effort on platforms that
/// support it; no-op on others.
pub fn fsync_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        #[cfg(unix)]
        {
            use std::fs::OpenOptions;
            if let Ok(f) = OpenOptions::new().read(true).open(parent) {
                let _ = f.sync_all();
            }
        }
        #[cfg(not(unix))]
        {
            let _ = parent;
        }
    }
}

use std::io::Write;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_acquire_and_drop_releases() {
        let dir = TempDir::new().unwrap();
        let path = pending_txn_lock_path(dir.path());
        {
            let _guard = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
            assert!(path.exists());
        }
        // Canonical file persists after release.
        assert!(path.exists());
    }

    #[test]
    fn test_concurrent_acquire_fails() {
        let dir = TempDir::new().unwrap();
        let _guard1 = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        let result = acquire_pending_txn(dir.path(), Duration::from_millis(50));
        assert!(matches!(result, Err(PendingTxnLockError::Busy { .. })));
    }

    #[test]
    fn test_ownership_checked_drop() {
        let dir = TempDir::new().unwrap();
        let path = pending_txn_lock_path(dir.path());
        let guard1 = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        let nonce1 = guard1.nonce().to_string();
        drop(guard1);
        // Canonical file persists after release.
        assert!(path.exists());
        let guard2 = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        let nonce2 = guard2.nonce().to_string();
        assert_ne!(nonce1, nonce2);
    }

    #[test]
    fn test_lock_permissions() {
        let dir = TempDir::new().unwrap();
        let _guard = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(pending_txn_lock_path(dir.path())).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_no_secrets_in_lock_file() {
        let dir = TempDir::new().unwrap();
        let _guard = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        let raw = std::fs::read_to_string(pending_txn_lock_path(dir.path())).unwrap();
        let raw_lower = raw.to_lowercase();
        let value_only = raw_lower
            .lines()
            .filter_map(|line| line.split_once('=').map(|(_, v)| v))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "command",
            "description",
            "password",
            "secret",
            "api_key",
            "apikey",
            "credential",
        ] {
            assert!(
                !value_only.contains(forbidden),
                "pending txn lock must not contain {forbidden} in a value"
            );
        }
    }

    #[test]
    fn test_unique_temp_path_is_distinct() {
        let dir = TempDir::new().unwrap();
        let final_path = dir.path().join("auto-sync-pending.toml");
        let a = unique_temp_path(&final_path);
        let b = unique_temp_path(&final_path);
        assert_ne!(a, b);
        assert!(a.starts_with(dir.path()));
    }

    #[test]
    fn test_atomic_write_unique_creates_file() {
        let dir = TempDir::new().unwrap();
        let final_path = dir.path().join("test.toml");
        let tmp = atomic_write_unique(&final_path, b"hello").unwrap();
        assert!(final_path.exists());
        assert_eq!(std::fs::read_to_string(&final_path).unwrap(), "hello");
        assert!(!tmp.exists());
    }

    #[test]
    fn test_fsync_parent_dir_does_not_panic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.toml");
        std::fs::write(&path, "content").unwrap();
        fsync_parent_dir(&path);
    }

    #[test]
    fn test_fsync_parent_dir_no_panic_on_missing_parent() {
        let path = PathBuf::from("/nonexistent/path/test.toml");
        fsync_parent_dir(&path);
    }

    #[test]
    fn test_fsync_parent_dir_no_panic_on_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("does_not_exist.toml");
        fsync_parent_dir(&path);
    }

    #[test]
    fn test_atomic_write_unique_same_bytes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.toml");
        atomic_write_unique(&path, b"hello").unwrap();
        atomic_write_unique(&path, b"world").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "world");
    }

    #[test]
    fn test_atomic_write_unique_overwrites_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.toml");
        std::fs::write(&path, "old content").unwrap();
        atomic_write_unique(&path, b"new content").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new content");
    }

    #[test]
    fn test_pending_txn_lock_error_display() {
        let err = PendingTxnLockError::Busy { timeout_ms: 500 };
        assert!(err.to_string().contains("500"));
    }

    #[test]
    fn test_pending_txn_lock_error_is_error() {
        let err: Box<dyn std::error::Error> =
            Box::new(PendingTxnLockError::Busy { timeout_ms: 100 });
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn test_empty_lock_file_does_not_block_acquisition() {
        let dir = TempDir::new().unwrap();
        let path = pending_txn_lock_path(dir.path());
        std::fs::write(&path, "").unwrap();
        let _guard = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        // Acquisition succeeded despite empty file.
        let observed = crate::process_file_lock::read_owner(&path).unwrap();
        assert_eq!(observed.purpose, PENDING_TXN_LOCK_PURPOSE);
    }

    #[test]
    fn test_malformed_lock_file_does_not_block_acquisition() {
        let dir = TempDir::new().unwrap();
        let path = pending_txn_lock_path(dir.path());
        std::fs::write(&path, "garbage data").unwrap();
        let _guard = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        let observed = crate::process_file_lock::read_owner(&path).unwrap();
        assert_eq!(observed.purpose, PENDING_TXN_LOCK_PURPOSE);
    }
}
