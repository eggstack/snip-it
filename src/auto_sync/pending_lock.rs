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

    /// Read the on-disk lock identity using the already-open file handle.
    /// Avoids opening a second handle which would conflict with the
    /// kernel lock on Windows.
    pub fn read_owner_via_handle(&self) -> Option<crate::process_file_lock::LockIdentity> {
        self.inner.read_identity_via_handle().ok().flatten()
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
        let guard = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        // Read via the existing file handle to avoid a second handle
        // that conflicts with the Windows kernel lock range.
        let id = guard
            .read_owner_via_handle()
            .expect("identity must be readable via handle");
        let serialized = toml::to_string_pretty(&id).unwrap();
        let raw_lower = serialized.to_lowercase();
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
        let guard = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        // Acquisition succeeded; old empty file was overwritten. Read
        // via the held handle to avoid opening a second handle.
        let observed = guard
            .read_owner_via_handle()
            .expect("identity must be readable via handle");
        assert_eq!(observed.purpose, PENDING_TXN_LOCK_PURPOSE);
    }

    #[test]
    fn test_malformed_lock_file_does_not_block_acquisition() {
        let dir = TempDir::new().unwrap();
        let path = pending_txn_lock_path(dir.path());
        std::fs::write(&path, "garbage data").unwrap();
        let guard = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        let observed = guard
            .read_owner_via_handle()
            .expect("identity must be readable via handle");
        assert_eq!(observed.purpose, PENDING_TXN_LOCK_PURPOSE);
    }
}
