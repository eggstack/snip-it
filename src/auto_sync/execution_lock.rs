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
//!
//! Authority for mutual exclusion is the operating system kernel; the
//! persistent lock file is diagnostic metadata only. See
//! [`crate::process_file_lock`] for the underlying primitive.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::process_file_lock::{self, LockIdentity, ProcessFileLock, ProcessFileLockError};

pub const EXECUTION_LOCK_NAME: &str = "auto-sync-execution.lock";
pub const EXECUTION_LOCK_PURPOSE: &str = "auto-sync-execution";
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(30);

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

impl From<ProcessFileLockError> for ExecutionLockError {
    fn from(err: ProcessFileLockError) -> Self {
        match err {
            ProcessFileLockError::Busy { owner } => match owner {
                Some(o) => Self::AlreadyHeld {
                    pid: o.pid,
                    started_at_unix_ms: o.acquired_at_unix_ms,
                    nonce: o.nonce,
                },
                None => Self::AlreadyHeld {
                    pid: 0,
                    started_at_unix_ms: 0,
                    nonce: String::new(),
                },
            },
            ProcessFileLockError::Timeout { owner } => match owner {
                Some(o) => Self::Timeout {
                    owner_pid: o.pid,
                    owner_started_at: o.acquired_at_unix_ms,
                },
                None => Self::Timeout {
                    owner_pid: 0,
                    owner_started_at: 0,
                },
            },
            ProcessFileLockError::Io(e) => Self::Io(e),
            ProcessFileLockError::UnsupportedPlatform => Self::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "process file lock is not supported on this platform",
            )),
        }
    }
}

/// RAII guard for the sync execution lock.
pub struct SyncExecutionLock {
    inner: ProcessFileLock,
}

impl SyncExecutionLock {
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

impl std::fmt::Debug for SyncExecutionLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncExecutionLock")
            .field("path", &self.inner.path())
            .field("nonce", &self.inner.nonce())
            .finish()
    }
}

pub fn execution_lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join(EXECUTION_LOCK_NAME)
}

/// Try to acquire the execution lock without waiting.
///
/// Returns `Err(AlreadyHeld)` if another process holds the kernel lock.
/// Stale metadata (a leftover file with a dead PID) is overwritten by the
/// next acquirer — no inspect-then-rename race can occur.
pub fn try_acquire(state_dir: &Path) -> Result<SyncExecutionLock, ExecutionLockError> {
    let path = execution_lock_path(state_dir);
    let inner = process_file_lock::try_acquire(&path, EXECUTION_LOCK_PURPOSE)?;
    Ok(SyncExecutionLock { inner })
}

/// Acquire the execution lock, polling the kernel lock every 100 ms until
/// `timeout` elapses. If the lock is still held after the timeout,
/// returns `Err(Timeout)` with best-effort owner metadata.
pub fn wait_acquire(
    state_dir: &Path,
    timeout: Duration,
) -> Result<SyncExecutionLock, ExecutionLockError> {
    let path = execution_lock_path(state_dir);
    let inner = process_file_lock::wait_acquire(&path, EXECUTION_LOCK_PURPOSE, timeout)?;
    Ok(SyncExecutionLock { inner })
}

/// Best-effort read of the on-disk execution lock metadata.
///
/// Diagnostic only — a missing, empty, or malformed file is reported as
/// `None` and does **not** imply the lock is free. Use [`try_acquire`] to
/// determine actual availability.
pub fn inspect(path: &Path) -> Option<LockIdentity> {
    process_file_lock::read_owner(path)
}

/// Returns `true` when the metadata records a PID that is no longer alive.
pub fn is_stale(contents: &LockIdentity) -> bool {
    !process_alive(contents.pid)
}

/// Check whether a process with the given PID is alive.
///
/// Delegates to the shared implementation in [`crate::utils::process`] so
/// liveness semantics stay identical across all lock implementations.
#[cfg(unix)]
pub fn process_alive(pid: u32) -> bool {
    crate::utils::process::is_process_alive(pid)
}

/// Check whether a process with the given PID is alive.
///
/// Delegates to the shared implementation in [`crate::utils::process`] so
/// liveness semantics stay identical across all lock implementations.
#[cfg(not(unix))]
pub fn process_alive(pid: u32) -> bool {
    crate::utils::process::is_process_alive(pid)
}

// ── Worker lock (merged from lock.rs) ──────────────────────────

pub const WORKER_LOCK_NAME: &str = "auto-sync-worker.lock";
pub const WORKER_LOCK_PURPOSE: &str = "auto-sync-worker";

#[derive(Debug)]
pub enum WorkerLockError {
    Io(std::io::Error),
    AlreadyHeld { pid: u32, nonce: String },
    Timeout { pid: u32, nonce: String },
}

impl std::fmt::Display for WorkerLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::AlreadyHeld { pid, nonce } => write!(
                f,
                "auto-sync worker lock already held (pid={pid}, nonce={nonce})"
            ),
            Self::Timeout { pid, nonce } => write!(
                f,
                "timed out waiting for auto-sync worker lock (pid={pid}, nonce={nonce})"
            ),
        }
    }
}

impl std::error::Error for WorkerLockError {}

impl From<ProcessFileLockError> for WorkerLockError {
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
                Self::Timeout { pid, nonce }
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

    pub fn read_owner_via_handle(&self) -> Option<LockIdentity> {
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

pub fn worker_lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join(WORKER_LOCK_NAME)
}

pub fn try_acquire_worker(state_dir: &Path) -> Result<WorkerLock, WorkerLockError> {
    let path = worker_lock_path(state_dir);
    let inner = process_file_lock::try_acquire(&path, WORKER_LOCK_PURPOSE)?;
    Ok(WorkerLock { inner })
}

pub fn worker_inspect(path: &Path) -> Option<LockIdentity> {
    process_file_lock::read_owner(path)
}

pub fn worker_is_stale(contents: &LockIdentity) -> bool {
    !process_alive(contents.pid)
}

pub fn wait_acquire_worker(
    state_dir: &Path,
    timeout: Duration,
) -> Result<WorkerLock, WorkerLockError> {
    let path = worker_lock_path(state_dir);
    let inner = process_file_lock::wait_acquire(&path, WORKER_LOCK_PURPOSE, timeout)?;
    Ok(WorkerLock { inner })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_acquire_release() {
        let dir = TempDir::new().unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        let nonce1 = lock.nonce().to_string();
        assert!(execution_lock_path(dir.path()).exists());
        drop(lock);
        // Canonical file persists after release.
        assert!(execution_lock_path(dir.path()).exists());
        // Second acquisition produces a fresh nonce.
        let lock2 = try_acquire(dir.path()).unwrap();
        assert_ne!(lock2.nonce(), nonce1);
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
    fn test_lock_path_is_in_state_dir() {
        let dir = TempDir::new().unwrap();
        assert_eq!(
            execution_lock_path(dir.path()),
            dir.path().join(EXECUTION_LOCK_NAME)
        );
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
        assert_eq!(contents.purpose, EXECUTION_LOCK_PURPOSE);
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
    fn test_already_held_includes_owner_identity() {
        let dir = TempDir::new().unwrap();
        let first = try_acquire(dir.path()).unwrap();
        let nonce = first.nonce().to_string();
        let err = try_acquire(dir.path()).unwrap_err();
        match err {
            ExecutionLockError::AlreadyHeld { nonce: n, .. } => {
                assert_eq!(n, nonce);
            }
            other => panic!("expected AlreadyHeld, got {other:?}"),
        }
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
    fn test_is_stale_with_dead_pid() {
        let contents = LockIdentity {
            schema_version: 1,
            purpose: EXECUTION_LOCK_PURPOSE.to_string(),
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
            purpose: EXECUTION_LOCK_PURPOSE.to_string(),
            pid: std::process::id(),
            start_token: None,
            nonce: "test".to_string(),
            acquired_at_unix_ms: 0,
        };
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
    fn test_process_alive_zero_pid() {
        assert!(!process_alive(0));
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

    #[cfg(unix)]
    #[test]
    fn test_kill_zero_error_classification_is_conservative() {
        use crate::utils::process::classify_kill_zero_error;
        assert!(classify_kill_zero_error(Some(libc::EPERM)));
        assert!(!classify_kill_zero_error(Some(libc::ESRCH)));
        assert!(classify_kill_zero_error(Some(libc::EINVAL)));
    }

    #[test]
    fn test_wait_acquire_succeeds_when_lock_free() {
        let dir = TempDir::new().unwrap();
        let lock = wait_acquire(dir.path(), Duration::from_secs(5)).unwrap();
        assert!(execution_lock_path(dir.path()).exists());
        drop(lock);
        // Canonical file persists after release.
        assert!(execution_lock_path(dir.path()).exists());
    }

    #[test]
    fn test_wait_acquire_times_out() {
        let dir = TempDir::new().unwrap();
        let _first = try_acquire(dir.path()).unwrap();
        let start = std::time::Instant::now();
        let result = wait_acquire(dir.path(), Duration::from_millis(250));
        let elapsed = start.elapsed();
        assert!(matches!(result, Err(ExecutionLockError::Timeout { .. })));
        assert!(elapsed >= Duration::from_millis(200));
    }

    #[test]
    fn test_wait_acquire_resolves_after_drop() {
        let dir = TempDir::new().unwrap();
        let holder = try_acquire(dir.path()).unwrap();
        let dir_path = dir.path().to_path_buf();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(120));
            drop(holder);
        });
        let result = wait_acquire(&dir_path, Duration::from_secs(2));
        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_lock_file_does_not_block_acquisition() {
        let dir = TempDir::new().unwrap();
        let path = execution_lock_path(dir.path());
        std::fs::write(&path, "").unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        let observed = lock.read_owner_via_handle().unwrap();
        assert_eq!(observed.nonce, lock.nonce());
    }

    #[test]
    fn test_malformed_lock_file_does_not_block_acquisition() {
        let dir = TempDir::new().unwrap();
        let path = execution_lock_path(dir.path());
        std::fs::write(&path, "garbage data").unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        // Old contents are overwritten; reading via the held handle
        // returns the new identity.
        let observed = lock.read_owner_via_handle().unwrap();
        assert_eq!(observed.nonce, lock.nonce());
    }

    #[test]
    fn test_legacy_dead_owner_does_not_block() {
        let dir = TempDir::new().unwrap();
        let path = execution_lock_path(dir.path());
        let legacy = LockIdentity {
            schema_version: 1,
            purpose: EXECUTION_LOCK_PURPOSE.to_string(),
            pid: 99999999,
            start_token: None,
            nonce: "legacy-dead".to_string(),
            acquired_at_unix_ms: 0,
        };
        std::fs::write(&path, toml::to_string_pretty(&legacy).unwrap()).unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        assert_ne!(lock.nonce(), "legacy-dead");
    }
}

// ── Worker spawning ───────────────────────────────────────────────

pub const WORKER_SUBCOMMAND: &str = "auto-sync-worker";

#[derive(Debug)]
#[non_exhaustive]
pub enum SpawnError {
    Spawn(std::io::Error),
}

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "spawn failed: {e}"),
        }
    }
}

impl std::error::Error for SpawnError {}

pub fn spawn_worker(state_dir: &Path) -> Result<u32, SpawnError> {
    let exe = std::env::current_exe().map_err(SpawnError::Spawn)?;
    let mut cmd = Command::new(&exe);
    cmd.arg(WORKER_SUBCOMMAND);
    cmd.arg("--state-dir");
    cmd.arg(state_dir.as_os_str());

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    // SNP_AUTO_SYNC_WORKER_LOG is an opt-in debug aid: when set, the
    // worker's stderr is appended to this file (in addition to being
    // returned to the parent via the kernel's normal pipes). It is
    // not enabled by default and adds no production cost.
    let stderr = match std::env::var("SNP_AUTO_SYNC_WORKER_LOG") {
        Ok(log) => std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .map(Stdio::from)
            .unwrap_or_else(|_| Stdio::null()),
        Err(_) => Stdio::null(),
    };
    cmd.stderr(stderr);

    apply_platform_detach(&mut cmd);

    let child = cmd.spawn().map_err(SpawnError::Spawn)?;
    Ok(child.id())
}

#[cfg(unix)]
fn apply_platform_detach(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn apply_platform_detach(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
}

#[cfg(test)]
mod spawn_tests {
    use super::*;

    #[test]
    fn test_worker_subcommand_name() {
        assert_eq!(WORKER_SUBCOMMAND, "auto-sync-worker");
    }

    #[test]
    fn test_spawn_error_io_display() {
        let io_err = std::io::Error::other("boom");
        let e = SpawnError::Spawn(io_err);
        assert!(e.to_string().contains("spawn failed"));
        assert!(e.to_string().contains("boom"));
    }
}
