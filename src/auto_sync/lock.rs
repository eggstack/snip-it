//! RAII worker lock backed by the shared kernel primitive.
//!
//! Authority for mutual exclusion is the operating system kernel; the
//! persistent lock file is diagnostic metadata only. See
//! [`crate::process_file_lock`] for the underlying primitive.
//!
//! The canonical lock file persists on disk after release. Dropping the
//! guard only releases the kernel lock. Other processes may read the
//! remaining metadata for status output.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::process_file_lock::{self, LockIdentity, ProcessFileLock, ProcessFileLockError};

pub const WORKER_LOCK_NAME: &str = "auto-sync-worker.lock";
pub const WORKER_LOCK_PURPOSE: &str = "auto-sync-worker";

#[derive(Debug)]
pub enum LockError {
    Io(std::io::Error),
    AlreadyHeld { pid: u32, nonce: String },
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::AlreadyHeld { pid, nonce } => write!(
                f,
                "auto-sync worker lock already held (pid={pid}, nonce={nonce})"
            ),
        }
    }
}

impl std::error::Error for LockError {}

impl From<ProcessFileLockError> for LockError {
    fn from(err: ProcessFileLockError) -> Self {
        match err {
            ProcessFileLockError::Busy { owner } => {
                let pid = owner.as_ref().map(|o| o.pid).unwrap_or(0);
                let nonce = owner.as_ref().map(|o| o.nonce.clone()).unwrap_or_default();
                Self::AlreadyHeld { pid, nonce }
            }
            ProcessFileLockError::Timeout { owner } => {
                let pid = owner.as_ref().map(|o| o.pid).unwrap_or(0);
                let nonce = owner.as_ref().map(|o| o.nonce.clone()).unwrap_or_default();
                Self::AlreadyHeld { pid, nonce }
            }
            ProcessFileLockError::Io(e) => Self::Io(e),
            ProcessFileLockError::UnsupportedPlatform => Self::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "process file lock is not supported on this platform",
            )),
        }
    }
}

/// RAII guard for the auto-sync worker lock.
pub struct WorkerLock {
    inner: ProcessFileLock,
}

impl WorkerLock {
    pub fn nonce(&self) -> &str {
        self.inner.nonce()
    }

    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    /// Read the on-disk lock identity using the already-open file handle.
    /// Avoids opening a second handle which would conflict with the
    /// kernel lock on Windows.
    pub fn read_owner_via_handle(&self) -> Option<crate::process_file_lock::LockIdentity> {
        self.inner.read_identity_via_handle().ok().flatten()
    }
}

impl std::fmt::Debug for WorkerLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerLock")
            .field("path", &self.inner.path())
            .field("nonce", &self.inner.nonce())
            .finish()
    }
}

pub fn lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join(WORKER_LOCK_NAME)
}

pub fn try_acquire(state_dir: &Path) -> Result<WorkerLock, LockError> {
    let path = lock_path(state_dir);
    let inner = process_file_lock::try_acquire(&path, WORKER_LOCK_PURPOSE)?;
    Ok(WorkerLock { inner })
}

/// Best-effort read of the on-disk worker lock metadata.
///
/// The metadata is diagnostic only — a missing, empty, or malformed file
/// is reported as `None` and does **not** imply the lock is free. Use
/// [`try_acquire`] to determine actual availability.
pub fn inspect(path: &Path) -> Option<LockIdentity> {
    process_file_lock::read_owner(path)
}

/// Returns `true` when the metadata records a PID that is no longer alive.
///
/// Stale metadata may persist on disk after the kernel lock has been
/// released. This helper is for diagnostics (status output, doctor
/// repair) and must not authorize lock stealing.
pub fn is_stale(contents: &LockIdentity) -> bool {
    !process_alive(contents.pid)
}

#[cfg(unix)]
pub fn process_alive(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    const SIGNAL_NOOP: i32 = 0;
    unsafe { kill(pid as i32, SIGNAL_NOOP) == 0 }
}

#[cfg(not(unix))]
pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return true;
    }
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        if ok == 0 {
            return false;
        }
        exit_code == STILL_ACTIVE as u32
    }
}

#[allow(dead_code)]
pub fn process_alive_pub(pid: u32) -> bool {
    process_alive(pid)
}

/// Wait at most `timeout` for the worker lock to become available.
///
/// On timeout the underlying kernel-backed primitive returns
/// `Busy` which we map to `LockError::AlreadyHeld` with the recorded
/// owner identity (when readable). The lock file is never deleted as
/// part of acquisition or release.
#[allow(dead_code)]
pub fn wait_acquire(state_dir: &Path, timeout: Duration) -> Result<WorkerLock, LockError> {
    let path = lock_path(state_dir);
    let inner = process_file_lock::wait_acquire(&path, WORKER_LOCK_PURPOSE, timeout)?;
    Ok(WorkerLock { inner })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_acquire_and_drop() {
        let dir = TempDir::new().unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        assert!(lock_path(dir.path()).exists());
        let nonce1 = lock.nonce().to_string();
        drop(lock);
        // Canonical file persists after release.
        assert!(lock_path(dir.path()).exists());
        // Second acquisition produces a fresh nonce.
        let lock2 = try_acquire(dir.path()).unwrap();
        assert_ne!(lock2.nonce(), nonce1);
    }

    #[test]
    fn test_double_acquire_fails() {
        let dir = TempDir::new().unwrap();
        let _first = try_acquire(dir.path()).unwrap();
        let result = try_acquire(dir.path());
        assert!(matches!(result, Err(LockError::AlreadyHeld { .. })));
    }

    #[test]
    fn test_live_owner_not_stolen_by_age() {
        let dir = TempDir::new().unwrap();
        let lock1 = try_acquire(dir.path()).unwrap();
        let nonce1 = lock1.nonce().to_string();

        let result = try_acquire(dir.path());
        assert!(matches!(result, Err(LockError::AlreadyHeld { .. })));

        drop(lock1);
        let lock2 = try_acquire(dir.path()).unwrap();
        assert_ne!(lock2.nonce(), nonce1);
    }

    #[test]
    fn test_lock_path_is_in_state_dir() {
        let dir = TempDir::new().unwrap();
        assert_eq!(lock_path(dir.path()), dir.path().join(WORKER_LOCK_NAME));
    }

    #[test]
    fn test_no_secrets_in_lock_file() {
        let dir = TempDir::new().unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        // Read via the existing file handle to avoid a second handle
        // that conflicts with the Windows kernel lock range.
        let id = lock
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
                "lock file must not contain {forbidden} in a value"
            );
        }
    }

    #[test]
    fn test_inspect_returns_contents() {
        let dir = TempDir::new().unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        let contents = lock.read_owner_via_handle().unwrap();
        assert_eq!(contents.pid, std::process::id());
        assert_eq!(contents.nonce, lock.nonce());
        assert_eq!(contents.purpose, WORKER_LOCK_PURPOSE);
    }

    #[test]
    fn test_inspect_returns_none_for_missing() {
        let path = PathBuf::from("/nonexistent/path/lock");
        assert!(inspect(&path).is_none());
    }

    #[test]
    fn test_inspect_returns_none_for_empty() {
        let dir = TempDir::new().unwrap();
        let path = lock_path(dir.path());
        std::fs::write(&path, "").unwrap();
        assert!(inspect(&path).is_none());
    }

    #[test]
    fn test_inspect_returns_none_for_malformed() {
        let dir = TempDir::new().unwrap();
        let path = lock_path(dir.path());
        std::fs::write(&path, "garbage").unwrap();
        assert!(inspect(&path).is_none());
    }

    #[test]
    fn test_lock_permissions() {
        let dir = TempDir::new().unwrap();
        let _lock = try_acquire(dir.path()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(lock_path(dir.path())).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_lock_error_display() {
        let err = LockError::AlreadyHeld {
            pid: 12345,
            nonce: "abc".to_string(),
        };
        assert!(err.to_string().contains("12345"));
        assert!(err.to_string().contains("abc"));
    }

    #[test]
    fn test_lock_error_is_error() {
        let err: Box<dyn std::error::Error> = Box::new(LockError::AlreadyHeld {
            pid: 1,
            nonce: "test".to_string(),
        });
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn test_process_alive_zero_pid() {
        assert!(process_alive(0));
    }

    #[test]
    fn test_process_alive_current_pid() {
        assert!(process_alive(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn test_process_alive_nonexistent_pid() {
        assert!(!process_alive(99999999));
    }

    #[test]
    fn test_is_stale_with_dead_pid() {
        let contents = LockIdentity {
            schema_version: 1,
            purpose: WORKER_LOCK_PURPOSE.to_string(),
            pid: 99999999,
            start_token: None,
            nonce: "test".to_string(),
            acquired_at_unix_ms: 0,
        };
        assert!(is_stale(&contents));
    }

    #[test]
    fn test_is_stale_with_live_pid() {
        let contents = LockIdentity {
            schema_version: 1,
            purpose: WORKER_LOCK_PURPOSE.to_string(),
            pid: std::process::id(),
            start_token: None,
            nonce: "test".to_string(),
            acquired_at_unix_ms: 0,
        };
        assert!(!is_stale(&contents));
    }

    #[test]
    fn test_empty_lock_file_does_not_block_acquisition() {
        // An empty file means no one holds the kernel lock — the new
        // acquirer overwrites the file with its identity.
        let dir = TempDir::new().unwrap();
        let path = lock_path(dir.path());
        std::fs::write(&path, "").unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        let observed = lock.read_owner_via_handle().unwrap();
        assert_eq!(observed.nonce, lock.nonce());
    }

    #[test]
    fn test_malformed_lock_file_does_not_block_acquisition() {
        let dir = TempDir::new().unwrap();
        let path = lock_path(dir.path());
        std::fs::write(&path, "not toml garbage").unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        // Old contents are overwritten; reading via the held handle
        // returns the new identity.
        let observed = lock.read_owner_via_handle().unwrap();
        assert_eq!(observed.nonce, lock.nonce());
    }

    #[test]
    fn test_legacy_owner_does_not_block_when_process_dead() {
        // A previous run wrote a structured identity with a dead PID.
        // The kernel lock is free (no process holds the file), so the
        // new acquirer must overwrite without consulting the dead owner.
        let dir = TempDir::new().unwrap();
        let path = lock_path(dir.path());
        let legacy = LockIdentity {
            schema_version: 1,
            purpose: WORKER_LOCK_PURPOSE.to_string(),
            pid: 99999999,
            start_token: None,
            nonce: "legacy".to_string(),
            acquired_at_unix_ms: 0,
        };
        std::fs::write(&path, toml::to_string_pretty(&legacy).unwrap()).unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        assert_ne!(lock.nonce(), "legacy");
    }
}
