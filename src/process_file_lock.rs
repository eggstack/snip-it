//! Kernel-backed cross-process file lock.
//!
//! This module provides a single authoritative mutual-exclusion primitive
//! built on the operating system's advisory file-lock facility. On Unix
//! the kernel guarantees that `flock(fd, LOCK_EX | LOCK_NB)` is granted to
//! at most one process at a time. On Windows `LockFileEx` provides the
//! same guarantee over a fixed byte range. Unsupported platforms return
//! [`ProcessFileLockError::UnsupportedPlatform`] rather than silently
//! weakening exclusion.
//!
//! The lock file persists on disk. The presence of the file does **not**
//! indicate ownership — only the kernel lock state does. Acquire-after-crash
//! is therefore immediate and does not require stale-PID classification.
//! Stale metadata may be overwritten by the next acquirer without
//! investigation.
//!
//! # Identity metadata
//!
//! After the kernel lock is acquired the guard publishes a structured
//! [`LockIdentity`] record into the lock file. The metadata is diagnostic
//! only: it can be used in error messages, status output, and process
//! signaling decisions, but it must never authorize lock stealing. A
//! contender that observes a busy kernel lock with malformed, empty, or
//! legacy metadata must treat it as a live owner.
//!
//! # Lifecycle invariants
//!
//! 1. The parent directory is created at acquisition time.
//! 2. The persistent lock file is opened in read/write/create mode.
//! 3. The kernel exclusive lock is attempted nonblocking.
//! 4. On kernel failure, [`ProcessFileLockError::Busy`] is returned.
//! 5. After successful acquisition the file is truncated, the identity
//!    record is written and `sync_all` is called, then Unix permissions
//!    are tightened to `0o600` where supported.
//! 6. If metadata publication fails after kernel acquisition, the kernel
//!    lock is released and an error is returned; no one is left believing
//!    they own the lock.
//! 7. `Drop` releases the kernel lock and closes the file. It does not
//!    unlink or rename the lock file.
//!
//! # Cancellation safety
//!
//! `Drop` does not touch the filesystem. A second acquirer that follows
//! a released first acquirer is independent of the first — the kernel
//! alone arbitrates.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Schema version of [`LockIdentity`]. Bumped when the on-disk format
/// changes in a backwards-incompatible way.
pub const LOCK_IDENTITY_SCHEMA_VERSION: u32 = 1;

/// Diagnostic owner metadata published after the kernel lock is acquired.
///
/// The kernel owns mutual exclusion; this record is for diagnostics only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockIdentity {
    /// Schema version of the lock record.
    pub schema_version: u32,
    /// Free-form purpose label (e.g. "auto-sync-worker", "server-singleton").
    pub purpose: String,
    /// Process ID of the owner.
    pub pid: u32,
    /// Start-time token of the owner, when the platform exposes one.
    #[serde(default)]
    pub start_token: Option<String>,
    /// Random nonce generated at acquisition time.
    pub nonce: String,
    /// Unix timestamp in milliseconds when the kernel lock was acquired.
    pub acquired_at_unix_ms: u64,
}

/// Errors returned by [`try_acquire`] and [`wait_acquire`].
#[derive(Debug)]
pub enum ProcessFileLockError {
    /// The kernel lock is currently held by another process.
    Busy {
        /// Best-effort parsed metadata of the current owner, when readable.
        owner: Option<LockIdentity>,
    },
    /// `wait_acquire` exhausted its deadline while the kernel lock was held.
    Timeout {
        /// Best-effort parsed metadata of the current owner, when readable.
        owner: Option<LockIdentity>,
    },
    /// Underlying I/O failure.
    Io(std::io::Error),
    /// The current platform has no kernel-backed lock implementation.
    UnsupportedPlatform,
}

impl std::fmt::Display for ProcessFileLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy { owner } => match owner {
                Some(id) => write!(
                    f,
                    "process lock busy (pid={}, purpose={}, nonce={})",
                    id.pid, id.purpose, id.nonce
                ),
                None => write!(f, "process lock busy"),
            },
            Self::Timeout { owner } => match owner {
                Some(id) => write!(
                    f,
                    "timed out waiting for process lock (pid={}, purpose={}, nonce={})",
                    id.pid, id.purpose, id.nonce
                ),
                None => write!(f, "timed out waiting for process lock"),
            },
            Self::Io(e) => write!(f, "process lock io error: {e}"),
            Self::UnsupportedPlatform => write!(
                f,
                "process lock is not supported on this platform; refusing to weaken exclusion"
            ),
        }
    }
}

impl std::error::Error for ProcessFileLockError {}

impl From<std::io::Error> for ProcessFileLockError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// RAII guard for an acquired kernel-backed file lock.
///
/// Drop releases the kernel lock and closes the file. It does not unlink
/// or rename the lock file. Acquiring a different guard at the same path
/// is possible only after the kernel releases the existing lock, which
/// happens when every [`ProcessFileLock`] referencing it is dropped or
/// the owner process exits (and the kernel cleans up its file
/// descriptors).
#[derive(Debug)]
pub struct ProcessFileLock {
    file: Option<File>,
    path: PathBuf,
    identity: LockIdentity,
}

impl ProcessFileLock {
    /// Path of the persistent lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Identity record published after acquisition.
    pub fn identity(&self) -> &LockIdentity {
        &self.identity
    }

    /// Nonce generated at acquisition time.
    pub fn nonce(&self) -> &str {
        &self.identity.nonce
    }

    /// Read the current contents of the lock file using the already-open
    /// file handle. Useful for callers that hold the kernel lock and
    /// need to read the file's bytes — opening a second handle on
    /// Windows would conflict with the locked byte range.
    pub fn read_identity_via_handle(&self) -> std::io::Result<Option<LockIdentity>> {
        use std::io::{Read, Seek};
        let Some(file) = self.file.as_ref() else {
            return Ok(None);
        };
        let mut f = file.try_clone()?;
        f.seek(std::io::SeekFrom::Start(0))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        let raw = match std::str::from_utf8(&buf) {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };
        if raw.trim().is_empty() {
            return Ok(None);
        }
        Ok(toml::from_str(raw).ok())
    }
}

impl Drop for ProcessFileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        release_unix_lock(self.file.as_ref());
        #[cfg(windows)]
        release_windows_lock(self.file.as_ref());
        // Dropping the File releases the underlying handle and closes
        // the descriptor. The lock file remains on disk.
        self.file = None;
    }
}

#[cfg(unix)]
fn release_unix_lock(file: Option<&File>) {
    if let Some(file) = file {
        use std::os::fd::AsRawFd;
        unsafe {
            libc::flock(file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(windows)]
fn release_windows_lock(file: Option<&File>) {
    if let Some(file) = file {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
        let handle = file.as_raw_handle();
        let mut overlapped: windows_sys::Win32::System::IO::OVERLAPPED =
            unsafe { std::mem::zeroed() };
        // Lock/unlock ranges must match the LockFileEx call in inner_acquire.
        overlapped.Anonymous.Anonymous.Offset = LOCKED_BYTE_OFFSET;
        overlapped.Anonymous.Anonymous.OffsetHigh = 0;
        unsafe {
            UnlockFileEx(handle, 0, LOCKED_BYTE_COUNT, 0, &mut overlapped);
        }
    }
}

#[cfg(windows)]
const LOCKED_BYTE_COUNT: u32 = 1;
// Lock a single byte at offset `u32::MAX`. This range is far beyond any
// real file content, so Windows does not deny read or write access to
// the actual lock-file bytes. Only another attempt to lock the same
// range returns ERROR_LOCK_VIOLATION.
#[cfg(windows)]
const LOCKED_BYTE_OFFSET: u32 = u32::MAX;

/// Try to acquire the kernel lock at `path` without waiting.
///
/// `purpose` is a free-form diagnostic label recorded in [`LockIdentity`].
pub fn try_acquire(path: &Path, purpose: &str) -> Result<ProcessFileLock, ProcessFileLockError> {
    inner_acquire(path, purpose)
}

/// Acquire the kernel lock at `path`, polling every `poll_interval` until
/// `timeout` elapses.
///
/// Returns [`ProcessFileLockError::Timeout`] if the deadline is reached
/// while another process still holds the kernel lock. The default poll
/// interval is 100 milliseconds.
pub fn wait_acquire(
    path: &Path,
    purpose: &str,
    timeout: Duration,
) -> Result<ProcessFileLock, ProcessFileLockError> {
    let deadline = std::time::Instant::now() + timeout;
    let poll_interval = Duration::from_millis(100);
    loop {
        match inner_acquire(path, purpose) {
            Ok(guard) => return Ok(guard),
            Err(ProcessFileLockError::Busy { owner }) => {
                if std::time::Instant::now() >= deadline {
                    return Err(ProcessFileLockError::Timeout { owner });
                }
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                std::thread::sleep(poll_interval.min(remaining));
            }
            Err(other) => return Err(other),
        }
    }
}

/// Best-effort read of the current owner identity. Returns `None` if the
/// lock file is missing or contains unreadable content. The lock state is
/// not consulted — this is a diagnostic helper only.
pub fn read_owner(path: &Path) -> Option<LockIdentity> {
    let raw = std::fs::read_to_string(path).ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    toml::from_str(&raw).ok()
}

fn inner_acquire(path: &Path, purpose: &str) -> Result<ProcessFileLock, ProcessFileLockError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Open the persistent file with read/write/create so the descriptor
    // exists for the lifetime of the guard.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;

    // Attempt the kernel lock.
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            if matches!(err.raw_os_error(), Some(libc::EWOULDBLOCK)) {
                let owner = read_owner(path);
                return Err(ProcessFileLockError::Busy { owner });
            }
            return Err(ProcessFileLockError::Io(err));
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
        };
        let handle = file.as_raw_handle();
        let mut overlapped: windows_sys::Win32::System::IO::OVERLAPPED =
            unsafe { std::mem::zeroed() };
        overlapped.Anonymous.Anonymous.Offset = LOCKED_BYTE_OFFSET;
        overlapped.Anonymous.Anonymous.OffsetHigh = 0;
        let ok = unsafe {
            LockFileEx(
                handle,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                LOCKED_BYTE_COUNT,
                0,
                &mut overlapped,
            )
        };
        if ok == 0 {
            let err = std::io::Error::last_os_error();
            // ERROR_LOCK_VIOLATION (33) and ERROR_SHARING_VIOLATION (32)
            // both indicate another process holds the conflicting range.
            if matches!(err.raw_os_error(), Some(33) | Some(32)) {
                let owner = read_owner(path);
                return Err(ProcessFileLockError::Busy { owner });
            }
            return Err(ProcessFileLockError::Io(err));
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = file;
        return Err(ProcessFileLockError::UnsupportedPlatform);
    }

    // Build the identity record. This must succeed before we publish.
    let identity = LockIdentity {
        schema_version: LOCK_IDENTITY_SCHEMA_VERSION,
        purpose: purpose.to_string(),
        pid: std::process::id(),
        start_token: current_start_token(),
        nonce: generate_nonce(),
        acquired_at_unix_ms: unix_now_ms(),
    };

    // Tighten permissions on Unix so the persistent file is private.
    #[cfg(unix)]
    {
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            if let Err(e) = std::fs::set_permissions(path, perms) {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "failed to tighten lock file permissions to 0600"
                );
            }
        }
    }

    // Publish the identity. On any failure, release the kernel lock and
    // return — no caller may believe it owns the lock with partial
    // metadata on disk.
    if let Err(e) = publish_identity(&mut file, &identity) {
        #[cfg(unix)]
        release_unix_lock(Some(&file));
        #[cfg(windows)]
        release_windows_lock(Some(&file));
        return Err(ProcessFileLockError::Io(e));
    }

    Ok(ProcessFileLock {
        file: Some(file),
        path: path.to_path_buf(),
        identity,
    })
}

fn publish_identity(file: &mut File, identity: &LockIdentity) -> std::io::Result<()> {
    file.set_len(0)?;
    file.seek(std::io::SeekFrom::Start(0))?;
    let serialized = toml::to_string_pretty(identity)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    file.write_all(serialized.as_bytes())?;
    file.sync_all()?;
    Ok(())
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

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn current_start_token() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let pid = std::process::id();
        let stat_path = format!("/proc/{pid}/stat");
        let content = std::fs::read_to_string(&stat_path).ok()?;
        parse_linux_proc_start_token(&content)
    }
    #[cfg(target_os = "macos")]
    {
        use libc::{PROC_PIDTBSDINFO, c_int, proc_bsdinfo, proc_pidinfo};
        let pid = std::process::id();
        let mut info: proc_bsdinfo = unsafe { std::mem::zeroed() };
        let ret = unsafe {
            proc_pidinfo(
                pid as c_int,
                PROC_PIDTBSDINFO,
                0,
                &mut info as *mut _ as *mut _,
                std::mem::size_of::<proc_bsdinfo>() as i32,
            )
        };
        if ret <= 0 {
            return None;
        }
        Some(format!(
            "{}.{:06}",
            info.pbi_start_tvsec, info.pbi_start_tvusec
        ))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
        use windows_sys::Win32::System::Threading::{
            GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, std::process::id());
            if handle.is_null() {
                return None;
            }
            let mut creation: FILETIME = std::mem::zeroed();
            let mut exit_time: FILETIME = std::mem::zeroed();
            let mut kernel_time: FILETIME = std::mem::zeroed();
            let mut user_time: FILETIME = std::mem::zeroed();
            let ok = GetProcessTimes(
                handle,
                &mut creation,
                &mut exit_time,
                &mut kernel_time,
                &mut user_time,
            );
            CloseHandle(handle);
            if ok == 0 {
                return None;
            }
            let creation =
                ((creation.dwHighDateTime as u64) << 32) | (creation.dwLowDateTime as u64);
            Some(creation.to_string())
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = std::process::id();
        None
    }
}

#[cfg(target_os = "linux")]
fn parse_linux_proc_start_token(stat: &str) -> Option<String> {
    let after_comm = stat.rfind(')')?;
    let fields: Vec<&str> = stat.get(after_comm + 2..)?.split_whitespace().collect();
    fields.get(19).map(|value| (*value).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_start_token_parser_reads_field_22() {
        let stat = "42 (name with ) parens) S f4 f5 f6 f7 f8 f9 f10 f11 f12 f13 f14 f15 f16 f17 f18 f19 f20 FIELD21 START22";
        assert_eq!(parse_linux_proc_start_token(stat), Some("START22".into()));
        assert_ne!(parse_linux_proc_start_token(stat), Some("FIELD21".into()));
        assert_eq!(parse_linux_proc_start_token("1 (short) S f3"), None);
    }

    #[test]
    fn first_acquisition_succeeds() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        let guard = try_acquire(&path, "test-first").unwrap();
        assert_eq!(guard.identity().purpose, "test-first");
        assert!(guard.identity().pid > 0);
        assert!(!guard.identity().nonce.is_empty());
    }

    #[test]
    fn second_acquisition_in_same_process_returns_busy() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        let _first = try_acquire(&path, "first").unwrap();
        let second = try_acquire(&path, "second");
        match second {
            Err(ProcessFileLockError::Busy { owner }) => {
                assert_eq!(owner.as_ref().unwrap().nonce, _first.nonce());
            }
            other => panic!("expected Busy, got {other:?}"),
        }
    }

    #[test]
    fn drop_allows_later_acquisition() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        let first = try_acquire(&path, "first").unwrap();
        let nonce1 = first.nonce().to_string();
        drop(first);
        let second = try_acquire(&path, "second").unwrap();
        assert_ne!(second.nonce(), nonce1);
    }

    #[test]
    fn canonical_file_persists_after_release() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        {
            let _g = try_acquire(&path, "test").unwrap();
        }
        assert!(
            path.exists(),
            "canonical lock file must persist after release"
        );
    }

    #[test]
    fn repeated_acquire_drop_cycles_create_no_extra_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        for i in 0..100 {
            let guard = try_acquire(&path, &format!("cycle-{i}")).unwrap();
            drop(guard);
        }
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "only the canonical lock file may remain (found {} entries)",
            entries.len()
        );
    }

    #[test]
    fn malformed_metadata_does_not_block_acquisition_when_lock_free() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        std::fs::write(&path, "garbage not toml").unwrap();
        // The kernel lock is free (no process holds the file), so the
        // malformed metadata must be overwritten by the new acquirer.
        let guard = try_acquire(&path, "recover-malformed").unwrap();
        assert_eq!(guard.identity().purpose, "recover-malformed");
    }

    #[test]
    fn empty_contents_do_not_block_acquisition_when_lock_free() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        std::fs::write(&path, "").unwrap();
        let guard = try_acquire(&path, "recover-empty").unwrap();
        assert_eq!(guard.identity().purpose, "recover-empty");
    }

    #[test]
    fn wait_acquire_succeeds_after_release() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        let first = try_acquire(&path, "first").unwrap();
        let path_clone = path.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(first);
        });
        let guard = wait_acquire(&path_clone, "wait", Duration::from_secs(5)).unwrap();
        assert_eq!(guard.identity().purpose, "wait");
        handle.join().unwrap();
    }

    #[test]
    fn wait_acquire_times_out_at_configured_deadline() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        let _first = try_acquire(&path, "holder").unwrap();
        let start = std::time::Instant::now();
        let result = wait_acquire(&path, "timeout", Duration::from_millis(200));
        let elapsed = start.elapsed();
        assert!(matches!(result, Err(ProcessFileLockError::Timeout { .. })));
        assert!(elapsed >= Duration::from_millis(150));
    }

    #[test]
    fn owner_metadata_contains_no_sensitive_values() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        let _g = try_acquire(&path, "diag-payload").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let lower = raw.to_lowercase();
        let value_only = lower
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

    #[cfg(unix)]
    #[test]
    fn lock_file_has_private_permissions() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        let _g = try_acquire(&path, "perm-test").unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn busy_owner_reports_diagnostic_identity() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        let _first = try_acquire(&path, "diagnostic").unwrap();
        let err = try_acquire(&path, "second").unwrap_err();
        match err {
            ProcessFileLockError::Busy { owner } => {
                let owner = owner.expect("owner metadata must be readable");
                assert_eq!(owner.purpose, "diagnostic");
            }
            other => panic!("expected Busy, got {other:?}"),
        }
    }

    #[test]
    fn nonce_uniqueness() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b);
    }

    #[test]
    fn identity_roundtrip() {
        let id = LockIdentity {
            schema_version: LOCK_IDENTITY_SCHEMA_VERSION,
            purpose: "roundtrip".to_string(),
            pid: 12345,
            start_token: Some("tok-1".to_string()),
            nonce: "nonce-1".to_string(),
            acquired_at_unix_ms: 9_999_999,
        };
        let s = toml::to_string_pretty(&id).unwrap();
        let back: LockIdentity = toml::from_str(&s).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn busy_error_display_includes_owner() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        let _first = try_acquire(&path, "show-owner").unwrap();
        let err = try_acquire(&path, "show-owner").unwrap_err();
        let s = err.to_string();
        assert!(
            s.contains("show-owner"),
            "display must include purpose: {s}"
        );
    }

    #[test]
    fn timeout_error_display_includes_owner() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        let _first = try_acquire(&path, "timeout-owner").unwrap();
        let err = wait_acquire(&path, "wait", Duration::from_millis(100)).unwrap_err();
        let s = err.to_string();
        assert!(
            s.contains("timeout-owner"),
            "display must include purpose: {s}"
        );
    }

    #[test]
    fn read_owner_returns_metadata_after_acquisition() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        let guard = try_acquire(&path, "inspect").unwrap();
        let observed = read_owner(&path).expect("owner must be readable");
        assert_eq!(observed.nonce, guard.nonce());
        assert_eq!(observed.purpose, "inspect");
    }

    #[test]
    fn read_owner_returns_none_for_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        std::fs::write(&path, "").unwrap();
        assert!(read_owner(&path).is_none());
    }

    #[test]
    fn read_owner_returns_none_for_malformed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k.lock");
        std::fs::write(&path, "not valid toml").unwrap();
        assert!(read_owner(&path).is_none());
    }
}
