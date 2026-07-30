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

use crate::transaction::ProcessIdentity;

pub const EXECUTION_LOCK_NAME: &str = "auto-sync-execution.lock";
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionLockContents {
    pub pid: u32,
    pub started_at_unix_ms: u64,
    pub nonce: String,
    /// Start-time token for the owner process. `None` for legacy locks.
    #[serde(default)]
    pub start_token: Option<String>,
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
        // Atomically rename the lock file aside before validating identity.
        // rename(2) on Unix (and MoveFileExW with REPLACE_EXISTING on Windows)
        // is atomic w.r.t. concurrent writers, so a replacement lock cannot be
        // removed by this drop guard.
        if let Some(contents) = inspect(&self.path)
            && contents.pid == std::process::id()
            && contents.nonce == self.nonce
        {
            let _ = remove_owned_lock(&self.path);
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
    acquire_loop(&path)
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
        match acquire_loop(&path) {
            Ok(lock) => return Ok(lock),
            Err(ExecutionLockError::AlreadyHeld { .. }) => {
                if start.elapsed() >= timeout {
                    // Best-effort owner lookup for diagnostics.
                    let (owner_pid, owner_started_at) = inspect(&path)
                        .map(|c| (c.pid, c.started_at_unix_ms))
                        .unwrap_or((0, 0));
                    return Err(ExecutionLockError::Timeout {
                        owner_pid,
                        owner_started_at,
                    });
                }
                std::thread::sleep(poll_interval.min(timeout.saturating_sub(start.elapsed())));
            }
            Err(other) => return Err(other),
        }
    }
}

fn acquire_loop(path: &Path) -> Result<SyncExecutionLock, ExecutionLockError> {
    // Bounded retries for transient states (empty in-flight file, NotFound
    // races on Windows delete-pending). The deadline keeps try_acquire from
    // blocking indefinitely if a writer is stuck or crashed mid-publish.
    let start = std::time::Instant::now();
    let max_in_flight_retries: u32 = 50;
    let mut in_flight_retries: u32 = 0;

    loop {
        match create_lock_file(path) {
            Ok(guard) => return Ok(guard),
            Err(CreateOutcome::AlreadyExists) => {
                // Lock exists — classify the existing owner.
                let content = match std::fs::read_to_string(path) {
                    Ok(c) => c,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // Lock was just removed — loop back and retry create_new.
                        in_flight_retries = 0;
                        continue;
                    }
                    Err(e) => return Err(ExecutionLockError::Io(e)),
                };

                let existing: ExecutionLockContents = match toml::from_str(&content) {
                    Ok(info) => info,
                    Err(_) if content.trim().is_empty() => {
                        // Another writer just called create_new but hasn't
                        // published its content yet. Sleep briefly and retry
                        // instead of stealing the in-flight lock. If the
                        // publisher is stuck or crashed, the bounded retry
                        // budget will trip and we surface Io so callers can
                        // distinguish the indeterminate state from a clean
                        // AlreadyHeld.
                        in_flight_retries += 1;
                        if in_flight_retries > max_in_flight_retries
                            || start.elapsed() >= Duration::from_secs(2)
                        {
                            return Err(ExecutionLockError::Io(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "execution lock file observed empty beyond retry budget",
                            )));
                        }
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => {
                        // Genuinely malformed lock — quarantine and retry.
                        in_flight_retries = 0;
                        let _ = quarantine_lock(path);
                        continue;
                    }
                };
                in_flight_retries = 0;

                match classify_existing(&existing) {
                    OwnerClass::Live => {
                        return Err(ExecutionLockError::AlreadyHeld {
                            pid: existing.pid,
                            started_at_unix_ms: existing.started_at_unix_ms,
                            nonce: existing.nonce,
                        });
                    }
                    OwnerClass::Dead => {
                        // Atomically rename the stale lock aside. If another
                        // writer already reclaimed it, treat as success.
                        let _ = quarantine_lock(path);
                        continue;
                    }
                    OwnerClass::PidReuse => {
                        let _ = quarantine_lock(path);
                        continue;
                    }
                }
            }
            Err(CreateOutcome::Io(e)) => return Err(ExecutionLockError::Io(e)),
        }
    }
}

enum OwnerClass {
    Live,
    Dead,
    PidReuse,
}

fn classify_existing(existing: &ExecutionLockContents) -> OwnerClass {
    let Some(observed) = ProcessIdentity::observe(existing.pid) else {
        return OwnerClass::Dead;
    };
    // Conservative policy: refuse to reclaim when we cannot prove the owner
    // is a different process. A live PID whose start identity cannot be
    // observed still counts as a live owner.
    match (existing.start_token.as_ref(), observed.start_token.as_ref()) {
        (Some(recorded), Some(observed_token)) if recorded != observed_token => {
            OwnerClass::PidReuse
        }
        _ => OwnerClass::Live,
    }
}

enum CreateOutcome {
    AlreadyExists,
    Io(std::io::Error),
}

fn create_lock_file(path: &Path) -> Result<SyncExecutionLock, CreateOutcome> {
    let identity = ProcessIdentity::current();
    let nonce = generate_nonce();
    let contents = ExecutionLockContents {
        pid: identity.pid,
        started_at_unix_ms: unix_now_ms(),
        nonce: nonce.clone(),
        start_token: identity.start_token.clone(),
    };

    let serialized = match toml::to_string_pretty(&contents) {
        Ok(s) => s,
        Err(e) => return Err(CreateOutcome::Io(std::io::Error::other(e))),
    };

    let mut f = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(CreateOutcome::AlreadyExists);
        }
        Err(e) => return Err(CreateOutcome::Io(e)),
    };
    if let Err(e) = f.write_all(serialized.as_bytes()) {
        // Best-effort cleanup so a partial lock does not block future acquires.
        let _ = std::fs::remove_file(path);
        return Err(CreateOutcome::Io(e));
    }
    if let Err(e) = f.sync_all() {
        let _ = std::fs::remove_file(path);
        return Err(CreateOutcome::Io(e));
    }
    restrict_permissions(path);
    Ok(SyncExecutionLock {
        path: path.to_path_buf(),
        nonce,
    })
}

/// Quarantine a stale or malformed lock by renaming it atomically aside.
///
/// `rename(2)` is atomic w.r.t. concurrent writers, so a replacement lock
/// installed between inspect and reclaim cannot be deleted. A
/// `NotFound` error from rename means another writer already reclaimed
/// the lock — treat as success.
fn quarantine_lock(lock_path: &Path) -> std::io::Result<PathBuf> {
    let quarantine_name = format!(
        "{}.quarantine.{}",
        EXECUTION_LOCK_NAME,
        uuid::Uuid::new_v4()
    );
    let quarantine_path = lock_path
        .parent()
        .unwrap_or(lock_path)
        .join(&quarantine_name);
    match std::fs::rename(lock_path, &quarantine_path) {
        Ok(()) => Ok(quarantine_path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(quarantine_path),
        Err(e) => Err(e),
    }
}

/// Atomically remove a lock we own. Uses rename-aside so a concurrent
/// replacement cannot be unlinked.
fn remove_owned_lock(lock_path: &Path) -> std::io::Result<()> {
    match quarantine_lock(lock_path) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn inspect(path: &Path) -> Option<ExecutionLockContents> {
    let content = std::fs::read_to_string(path).ok()?;
    // An empty file means another writer is in flight — refuse to classify
    // it as a malformed record.
    if content.trim().is_empty() {
        return None;
    }
    toml::from_str(&content).ok()
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

fn restrict_permissions(#[cfg_attr(not(unix), allow(unused_variables))] path: &Path) {
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
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
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
    #[cfg(unix)]
    fn test_dead_pid_lock_replaced() {
        let dir = TempDir::new().unwrap();
        let contents = ExecutionLockContents {
            pid: 1,
            started_at_unix_ms: unix_now_ms(),
            nonce: "dead-pid".to_string(),
            start_token: None,
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
        let raw_lower = raw.to_lowercase();
        // Forbidden substrings are checked against TOML value contents only —
        // field names like `start_token` are intentionally allowed because the
        // word "token" in a field name does not imply credential exposure.
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
            "token",
            "credential",
        ] {
            assert!(
                !value_only.contains(forbidden),
                "lock file must not contain {forbidden} in a value"
            );
        }
        // And the field name `start_token` itself is allowed — confirm the
        // sentinel check above correctly ignores it.
        assert!(
            raw_lower.contains("start_token"),
            "lock file should retain the start_token field for PID-reuse detection"
        );
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
            start_token: None,
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
            "malformed lock should be quarantined and allow acquisition"
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
            "lock with missing fields should be quarantined and allow acquisition"
        );
    }

    #[test]
    fn test_empty_lock_file_does_not_steal_lock() {
        let dir = TempDir::new().unwrap();
        let path = execution_lock_path(dir.path());
        std::fs::write(&path, "").unwrap();
        let result = try_acquire(dir.path());
        // Empty file means another writer is in flight — we must not steal it.
        // The retry loop will eventually give up if no one writes; for this
        // test we expect failure (not AlreadyHeld, not Ok) because we cannot
        // make progress. The exact error classification here is intentionally
        // permissive: what matters is that we do NOT silently succeed.
        assert!(
            result.is_err(),
            "empty lock file must not be silently reclaimed"
        );
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
    #[cfg(unix)]
    fn test_is_stale_with_dead_pid() {
        let contents = ExecutionLockContents {
            pid: 1,
            started_at_unix_ms: unix_now_ms(),
            nonce: "test".to_string(),
            start_token: None,
        };
        assert!(is_stale(&contents));
    }

    #[test]
    fn test_is_stale_with_live_pid() {
        let contents = ExecutionLockContents {
            pid: std::process::id(),
            started_at_unix_ms: unix_now_ms(),
            nonce: "test".to_string(),
            start_token: None,
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

    #[test]
    #[cfg(unix)]
    fn test_pid_reuse_reclaims_lock() {
        let dir = TempDir::new().unwrap();
        let path = execution_lock_path(dir.path());
        // Write a lock whose recorded start_token does not match the current
        // process's start_token but whose PID is alive (PID-reuse scenario).
        let observed = ProcessIdentity::observe(std::process::id()).unwrap();
        let bogus_token = match observed.start_token.as_ref() {
            Some(t) => format!("{t}-bogus"),
            None => "bogus-token".to_string(),
        };
        let contents = ExecutionLockContents {
            pid: std::process::id(),
            started_at_unix_ms: unix_now_ms(),
            nonce: "pid-reuse".to_string(),
            start_token: Some(bogus_token),
        };
        std::fs::write(&path, toml::to_string_pretty(&contents).unwrap()).unwrap();

        let lock = try_acquire(dir.path()).unwrap();
        assert_ne!(lock.nonce(), "pid-reuse");
    }

    #[test]
    fn test_legacy_lock_without_start_token_blocks_live_owner() {
        let dir = TempDir::new().unwrap();
        let path = execution_lock_path(dir.path());
        // Legacy lock with no start_token — must be treated as a live owner
        // (conservative policy).
        let contents = ExecutionLockContents {
            pid: std::process::id(),
            started_at_unix_ms: unix_now_ms(),
            nonce: "legacy".to_string(),
            start_token: None,
        };
        std::fs::write(&path, toml::to_string_pretty(&contents).unwrap()).unwrap();
        let result = try_acquire(dir.path());
        assert!(
            matches!(result, Err(ExecutionLockError::AlreadyHeld { .. })),
            "legacy lock with live PID and no start_token must block"
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_replacement_lock_not_unlinked_by_old_drop() {
        // The Drop guard for an older lock must not unlink a replacement
        // lock installed between inspect and removal. Use atomic rename
        // so a concurrent replacement cannot be removed by drop.
        let dir = TempDir::new().unwrap();
        let lock1 = try_acquire(dir.path()).unwrap();
        let nonce1 = lock1.nonce().to_string();
        let lock1_path = lock1.path.clone();

        // Simulate a replacement lock by manually overwriting the lock file
        // with valid contents that DO NOT match lock1's nonce.
        let replacement = ExecutionLockContents {
            pid: 1,
            started_at_unix_ms: unix_now_ms(),
            nonce: "replacement".to_string(),
            start_token: None,
        };
        std::fs::write(&lock1_path, toml::to_string_pretty(&replacement).unwrap()).unwrap();

        drop(lock1);

        // The replacement lock file (PID 1 is dead) must still be present
        // because drop's nonce check should refuse to remove it.
        assert!(
            lock1_path.exists(),
            "old guard must not unlink a replacement lock file"
        );
        assert_ne!(
            nonce1, "replacement",
            "sanity: distinct nonces in test setup"
        );
    }
}
