//! RAII worker lock with PID-file stale detection.

use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const WORKER_LOCK_NAME: &str = "auto-sync-worker.lock";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerLockContents {
    pub pid: u32,
    pub started_at_unix_ms: u64,
    pub nonce: String,
}

#[derive(Debug)]
pub enum LockError {
    Io(std::io::Error),
    AlreadyHeld { pid: u32, nonce: String },
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::AlreadyHeld { pid, nonce } => {
                write!(
                    f,
                    "auto-sync worker lock already held (pid={pid}, nonce={nonce})"
                )
            }
        }
    }
}

impl std::error::Error for LockError {}

pub struct WorkerLock {
    path: PathBuf,
    nonce: String,
}

impl WorkerLock {
    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for WorkerLock {
    fn drop(&mut self) {
        if let Some(contents) = inspect(&self.path)
            && contents.pid == std::process::id()
            && contents.nonce == self.nonce
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub fn lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join(WORKER_LOCK_NAME)
}

pub fn try_acquire(state_dir: &Path) -> Result<WorkerLock, LockError> {
    let path = lock_path(state_dir);

    if let Some(contents) = inspect(&path) {
        if !is_stale(&contents) {
            return Err(LockError::AlreadyHeld {
                pid: contents.pid,
                nonce: contents.nonce,
            });
        }
        let _ = std::fs::remove_file(&path);
    } else if path.exists() {
        // Malformed or empty lock file — remove it to allow acquisition.
        let _ = std::fs::remove_file(&path);
    }

    let nonce = generate_nonce();
    let contents = WorkerLockContents {
        pid: std::process::id(),
        started_at_unix_ms: unix_now_ms(),
        nonce: nonce.clone(),
    };

    let serialized =
        toml::to_string_pretty(&contents).map_err(|e| LockError::Io(std::io::Error::other(e)))?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(LockError::Io)?;
    f.write_all(serialized.as_bytes()).map_err(LockError::Io)?;
    f.sync_all().map_err(LockError::Io)?;
    restrict_permissions(&path);

    Ok(WorkerLock { path, nonce })
}

pub fn inspect(path: &Path) -> Option<WorkerLockContents> {
    let contents = std::fs::read_to_string(path).ok()?;
    toml::from_str(&contents).ok()
}

pub fn is_stale(contents: &WorkerLockContents) -> bool {
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
            return true;
        }
        let mut exit_code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        if ok == 0 {
            return true;
        }
        exit_code == STILL_ACTIVE as u32
    }
}

fn generate_nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{:x}-{:x}", std::process::id(), nanos, seq)
}

fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_lock_acquire_release() {
        let dir = TempDir::new().unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        assert!(lock_path(dir.path()).exists());
        drop(lock);
        assert!(!lock_path(dir.path()).exists());
    }

    #[test]
    fn test_double_acquire_fails() {
        let dir = TempDir::new().unwrap();
        let _first = try_acquire(dir.path()).unwrap();
        let result = try_acquire(dir.path());
        assert!(matches!(result, Err(LockError::AlreadyHeld { .. })));
    }

    #[test]
    #[cfg(unix)]
    fn test_dead_pid_lock_replaced() {
        let dir = TempDir::new().unwrap();
        let contents = WorkerLockContents {
            pid: 1,
            started_at_unix_ms: unix_now_ms(),
            nonce: "dead-pid".to_string(),
        };
        let serialized = toml::to_string_pretty(&contents).unwrap();
        std::fs::write(lock_path(dir.path()), serialized).unwrap();

        let lock = try_acquire(dir.path()).unwrap();
        assert_ne!(lock.nonce(), "dead-pid");
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
    fn test_old_guard_does_not_remove_replacement_lock() {
        let dir = TempDir::new().unwrap();
        let lock1_path = lock_path(dir.path());

        let lock1 = try_acquire(dir.path()).unwrap();
        let nonce1 = lock1.nonce().to_string();

        drop(lock1);

        let lock2 = try_acquire(dir.path()).unwrap();
        let nonce2 = lock2.nonce().to_string();
        assert_ne!(nonce1, nonce2);

        assert!(lock1_path.exists());
    }

    #[test]
    fn test_inspect_returns_contents() {
        let dir = TempDir::new().unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        let contents = inspect(&lock.path).unwrap();
        assert_eq!(contents.pid, std::process::id());
        assert_eq!(contents.nonce, lock.nonce);
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
    fn test_no_secrets_in_lock_file() {
        let dir = TempDir::new().unwrap();
        let _lock = try_acquire(dir.path()).unwrap();
        let raw = std::fs::read_to_string(lock_path(dir.path())).unwrap();
        for forbidden in [
            "command",
            "description",
            "password",
            "secret",
            "api_key",
            "apikey",
            "token",
            "credential",
        ] {
            assert!(
                !raw.to_lowercase().contains(forbidden),
                "lock file must not contain {forbidden}"
            );
        }
    }

    #[test]
    fn test_lock_path_is_in_state_dir() {
        let dir = TempDir::new().unwrap();
        assert_eq!(lock_path(dir.path()), dir.path().join(WORKER_LOCK_NAME));
    }

    #[test]
    fn test_nonce_uniqueness() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b);
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
    fn test_lock_contents_roundtrip() {
        let contents = WorkerLockContents {
            pid: 999,
            started_at_unix_ms: 1000,
            nonce: "test-nonce".to_string(),
        };
        let serialized = toml::to_string_pretty(&contents).unwrap();
        let deserialized: WorkerLockContents = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.pid, 999);
        assert_eq!(deserialized.started_at_unix_ms, 1000);
        assert_eq!(deserialized.nonce, "test-nonce");
    }

    #[test]
    fn test_malformed_lock_file_treated_as_stale() {
        let dir = TempDir::new().unwrap();
        let path = lock_path(dir.path());
        std::fs::write(&path, "this is not valid toml {{{").unwrap();
        let result = try_acquire(dir.path());
        assert!(
            result.is_ok(),
            "malformed lock should be treated as stale and allow acquisition"
        );
    }

    #[test]
    fn test_malformed_lock_with_missing_fields_treated_as_stale() {
        let dir = TempDir::new().unwrap();
        let path = lock_path(dir.path());
        std::fs::write(&path, "pid = 999\n").unwrap();
        let result = try_acquire(dir.path());
        assert!(
            result.is_ok(),
            "lock with missing fields should be treated as stale"
        );
    }

    #[test]
    fn test_empty_lock_file_treated_as_stale() {
        let dir = TempDir::new().unwrap();
        let path = lock_path(dir.path());
        std::fs::write(&path, "").unwrap();
        let result = try_acquire(dir.path());
        assert!(result.is_ok(), "empty lock file should be treated as stale");
    }

    #[test]
    fn test_inspect_returns_none_for_malformed() {
        let dir = TempDir::new().unwrap();
        let path = lock_path(dir.path());
        std::fs::write(&path, "garbage").unwrap();
        assert!(inspect(&path).is_none());
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
    #[cfg(unix)]
    fn test_is_stale_with_dead_pid() {
        let contents = WorkerLockContents {
            pid: 1,
            started_at_unix_ms: unix_now_ms(),
            nonce: "test".to_string(),
        };
        assert!(is_stale(&contents));
    }

    #[test]
    fn test_is_stale_with_live_pid() {
        let contents = WorkerLockContents {
            pid: std::process::id(),
            started_at_unix_ms: unix_now_ms(),
            nonce: "test".to_string(),
        };
        #[cfg(unix)]
        assert!(!is_stale(&contents));
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

    #[test]
    #[cfg(unix)]
    fn test_process_alive_nonexistent_pid() {
        assert!(!process_alive(99999999));
    }
}
