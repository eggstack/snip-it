//! Shared execution lock for all sync operations.
//!
//! Every sync operation — detached worker, manual `snp sync`, explicit
//! `--sync` flag, and cron — must acquire this lock before performing
//! actual sync work. This prevents concurrent sync operations from
//! interfering with each other.
//!
//! Foreground callers may wait for a bounded period; detached workers
//! should preserve pending work and exit/retry later when the lock is
//! busy.

use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const EXECUTION_LOCK_NAME: &str = "auto-sync-execution.lock";
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionLockContents {
    pub pid: u32,
    pub started_at_unix_ms: u64,
    pub nonce: String,
}

#[derive(Debug)]
pub enum ExecutionLockError {
    Io(std::io::Error),
    AlreadyHeld {
        pid: u32,
        started_at_unix_ms: u64,
        nonce: String,
    },
    Timeout {
        owner_pid: u32,
        owner_started_at: u64,
    },
}

impl std::fmt::Display for ExecutionLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::AlreadyHeld {
                pid,
                started_at_unix_ms,
                nonce,
            } => {
                write!(
                    f,
                    "sync execution lock already held (pid={pid}, started_at={started_at_unix_ms}ms, nonce={nonce})"
                )
            }
            Self::Timeout {
                owner_pid,
                owner_started_at,
            } => {
                write!(
                    f,
                    "timed out waiting for sync execution lock held by pid={owner_pid} (started_at={owner_started_at}ms)"
                )
            }
        }
    }
}

impl std::error::Error for ExecutionLockError {}

pub struct SyncExecutionLock {
    path: PathBuf,
    nonce: String,
}

impl SyncExecutionLock {
    pub fn nonce(&self) -> &str {
        &self.nonce
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SyncExecutionLock {
    fn drop(&mut self) {
        if let Some(contents) = inspect(&self.path)
            && contents.pid == std::process::id()
            && contents.nonce == self.nonce
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub fn execution_lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join(EXECUTION_LOCK_NAME)
}

/// Try to acquire the execution lock without waiting.
///
/// Returns `Err(AlreadyHeld)` if another process holds a live lock.
/// Stale locks (dead PID) are reclaimed automatically.
pub fn try_acquire(state_dir: &Path) -> Result<SyncExecutionLock, ExecutionLockError> {
    let path = execution_lock_path(state_dir);

    if let Some(contents) = inspect(&path) {
        if !is_stale(&contents) {
            return Err(ExecutionLockError::AlreadyHeld {
                pid: contents.pid,
                started_at_unix_ms: contents.started_at_unix_ms,
                nonce: contents.nonce,
            });
        }
        let _ = std::fs::remove_file(&path);
    } else if path.exists() {
        let _ = std::fs::remove_file(&path);
    }

    acquire_inner(&path)
}

/// Acquire the execution lock, waiting up to `timeout` for a busy lock.
///
/// Polls every 250ms. If the lock is still held after the timeout,
/// returns `Err(Timeout)`.
pub fn wait_acquire(
    state_dir: &Path,
    timeout: Duration,
) -> Result<SyncExecutionLock, ExecutionLockError> {
    let path = execution_lock_path(state_dir);
    let start = std::time::Instant::now();
    let poll_interval = Duration::from_millis(250);

    loop {
        // Try non-blocking acquire first
        if let Some(contents) = inspect(&path) {
            if is_stale(&contents) {
                let _ = std::fs::remove_file(&path);
            } else {
                if start.elapsed() >= timeout {
                    return Err(ExecutionLockError::Timeout {
                        owner_pid: contents.pid,
                        owner_started_at: contents.started_at_unix_ms,
                    });
                }
                std::thread::sleep(poll_interval.min(timeout.saturating_sub(start.elapsed())));
                continue;
            }
        } else if path.exists() {
            let _ = std::fs::remove_file(&path);
        }

        // Try to acquire
        return acquire_inner(&path);
    }
}

fn acquire_inner(path: &Path) -> Result<SyncExecutionLock, ExecutionLockError> {
    let nonce = generate_nonce();
    let contents = ExecutionLockContents {
        pid: std::process::id(),
        started_at_unix_ms: unix_now_ms(),
        nonce: nonce.clone(),
    };

    let serialized = toml::to_string_pretty(&contents)
        .map_err(|e| ExecutionLockError::Io(std::io::Error::other(e)))?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(ExecutionLockError::Io)?;
    f.write_all(serialized.as_bytes())
        .map_err(ExecutionLockError::Io)?;
    f.sync_all().map_err(ExecutionLockError::Io)?;
    restrict_permissions(path);

    Ok(SyncExecutionLock {
        path: path.to_path_buf(),
        nonce,
    })
}

pub fn inspect(path: &Path) -> Option<ExecutionLockContents> {
    let contents = std::fs::read_to_string(path).ok()?;
    toml::from_str(&contents).ok()
}

pub fn is_stale(contents: &ExecutionLockContents) -> bool {
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
    let _ = pid;
    true
}

fn generate_nonce() -> String {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{:x}", std::process::id(), nanos)
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
    fn test_acquire_release() {
        let dir = TempDir::new().unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        assert!(execution_lock_path(dir.path()).exists());
        drop(lock);
        assert!(!execution_lock_path(dir.path()).exists());
    }

    #[test]
    fn test_double_acquire_fails() {
        let dir = TempDir::new().unwrap();
        let _first = try_acquire(dir.path()).unwrap();
        let result = try_acquire(dir.path());
        assert!(matches!(
            result,
            Err(ExecutionLockError::AlreadyHeld { .. })
        ));
    }

    #[test]
    fn test_dead_pid_lock_replaced() {
        let dir = TempDir::new().unwrap();
        let contents = ExecutionLockContents {
            pid: 1,
            started_at_unix_ms: unix_now_ms(),
            nonce: "dead-pid".to_string(),
        };
        let serialized = toml::to_string_pretty(&contents).unwrap();
        std::fs::write(execution_lock_path(dir.path()), serialized).unwrap();

        let lock = try_acquire(dir.path()).unwrap();
        assert_ne!(lock.nonce(), "dead-pid");
    }

    #[test]
    fn test_live_owner_not_stolen_by_age() {
        let dir = TempDir::new().unwrap();
        let lock1 = try_acquire(dir.path()).unwrap();
        let nonce1 = lock1.nonce().to_string();

        let result = try_acquire(dir.path());
        assert!(matches!(
            result,
            Err(ExecutionLockError::AlreadyHeld { .. })
        ));

        drop(lock1);
        let lock2 = try_acquire(dir.path()).unwrap();
        assert_ne!(lock2.nonce(), nonce1);
    }

    #[test]
    fn test_old_guard_does_not_remove_replacement_lock() {
        let dir = TempDir::new().unwrap();
        let lock1_path = execution_lock_path(dir.path());

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
            let meta = std::fs::metadata(execution_lock_path(dir.path())).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_no_secrets_in_lock_file() {
        let dir = TempDir::new().unwrap();
        let _lock = try_acquire(dir.path()).unwrap();
        let raw = std::fs::read_to_string(execution_lock_path(dir.path())).unwrap();
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
        assert_eq!(
            execution_lock_path(dir.path()),
            dir.path().join(EXECUTION_LOCK_NAME)
        );
    }

    #[test]
    fn test_nonce_uniqueness() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b);
    }

    #[test]
    fn test_lock_error_display() {
        let err = ExecutionLockError::AlreadyHeld {
            pid: 12345,
            started_at_unix_ms: 1000,
            nonce: "abc".to_string(),
        };
        assert!(err.to_string().contains("12345"));
        assert!(err.to_string().contains("abc"));
    }

    #[test]
    fn test_timeout_error_display() {
        let err = ExecutionLockError::Timeout {
            owner_pid: 9999,
            owner_started_at: 5000,
        };
        assert!(err.to_string().contains("9999"));
        assert!(err.to_string().contains("5000"));
    }

    #[test]
    fn test_contents_roundtrip() {
        let contents = ExecutionLockContents {
            pid: 999,
            started_at_unix_ms: 1000,
            nonce: "test-nonce".to_string(),
        };
        let serialized = toml::to_string_pretty(&contents).unwrap();
        let deserialized: ExecutionLockContents = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.pid, 999);
        assert_eq!(deserialized.started_at_unix_ms, 1000);
        assert_eq!(deserialized.nonce, "test-nonce");
    }

    #[test]
    fn test_malformed_lock_file_treated_as_stale() {
        let dir = TempDir::new().unwrap();
        let path = execution_lock_path(dir.path());
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
        let path = execution_lock_path(dir.path());
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
        let path = execution_lock_path(dir.path());
        std::fs::write(&path, "").unwrap();
        let result = try_acquire(dir.path());
        assert!(result.is_ok(), "empty lock file should be treated as stale");
    }

    #[test]
    fn test_inspect_returns_none_for_malformed() {
        let dir = TempDir::new().unwrap();
        let path = execution_lock_path(dir.path());
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
        let path = execution_lock_path(dir.path());
        std::fs::write(&path, "").unwrap();
        assert!(inspect(&path).is_none());
    }

    #[test]
    fn test_is_stale_with_dead_pid() {
        let contents = ExecutionLockContents {
            pid: 1,
            started_at_unix_ms: unix_now_ms(),
            nonce: "test".to_string(),
        };
        assert!(is_stale(&contents));
    }

    #[test]
    fn test_is_stale_with_live_pid() {
        let contents = ExecutionLockContents {
            pid: std::process::id(),
            started_at_unix_ms: unix_now_ms(),
            nonce: "test".to_string(),
        };
        #[cfg(unix)]
        assert!(!is_stale(&contents));
    }

    #[test]
    fn test_lock_error_is_error() {
        let err: Box<dyn std::error::Error> = Box::new(ExecutionLockError::AlreadyHeld {
            pid: 1,
            started_at_unix_ms: 0,
            nonce: "test".to_string(),
        });
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn test_wait_acquire_success() {
        let dir = TempDir::new().unwrap();
        let lock = wait_acquire(dir.path(), Duration::from_secs(5)).unwrap();
        assert!(execution_lock_path(dir.path()).exists());
        drop(lock);
        assert!(!execution_lock_path(dir.path()).exists());
    }

    #[test]
    fn test_wait_acquire_timeout() {
        let dir = TempDir::new().unwrap();
        let _first = try_acquire(dir.path()).unwrap();
        let start = std::time::Instant::now();
        let result = wait_acquire(dir.path(), Duration::from_millis(300));
        let elapsed = start.elapsed();
        assert!(matches!(result, Err(ExecutionLockError::Timeout { .. })));
        assert!(elapsed >= Duration::from_millis(250));
    }

    #[test]
    fn test_contention_one_holder_blocks_another() {
        let dir = TempDir::new().unwrap();
        let holder = try_acquire(dir.path()).unwrap();

        // Second acquire should fail
        let second = try_acquire(dir.path());
        assert!(matches!(
            second,
            Err(ExecutionLockError::AlreadyHeld { .. })
        ));

        // Drop the holder
        drop(holder);

        // Now a third acquire should succeed
        let third = try_acquire(dir.path());
        assert!(third.is_ok());
    }

    #[test]
    fn test_wait_acquire_resolves_after_drop() {
        let dir = TempDir::new().unwrap();
        let holder = try_acquire(dir.path()).unwrap();

        // Spawn a thread that drops the lock after 100ms
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            drop(holder);
        });

        // wait_acquire should eventually succeed
        let result = wait_acquire(dir.path(), Duration::from_secs(2));
        assert!(result.is_ok());
        assert_ne!(result.unwrap().nonce(), "should-not-match");
    }
}
