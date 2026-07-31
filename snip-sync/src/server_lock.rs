//! Kernel-backed singleton lock for the `snip-sync` server.
//!
//! Authority for mutual exclusion is the operating system kernel; the
//! persistent lock file is diagnostic metadata only. The Unix path uses
//! `flock`; Windows uses `LockFileEx`. Unsupported platforms return an
//! explicit error rather than silently weakening exclusion.
//!
//! The server acquires this lock nonblocking at startup. While the lock
//! is held, the PID file is published and the listeners run. Dropping
//! the guard releases the kernel lock; the persistent lock file remains
//! on disk.
//!
//! A new server can start after a crashed owner exits because the kernel
//! releases the lock when the previous owner closes its file descriptor
//! at process teardown. The leftover metadata is harmless.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Seek, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Diagnostic identity record written into the server lock file after
/// the kernel lock is acquired. The kernel owns mutual exclusion; this
/// record is for diagnostics only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerLockIdentity {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
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

const SERVER_LOCK_NAME: &str = "snip-sync.server.lock";
const SERVER_LOCK_SCHEMA_VERSION: u32 = 1;

/// Errors from [`ServerLock::try_acquire`].
#[derive(Debug)]
pub enum ServerLockError {
    /// Another server holds the kernel lock.
    Busy {
        /// Best-effort parsed metadata of the current owner, when readable.
        owner: Option<ServerLockIdentity>,
    },
    /// Underlying I/O failure.
    Io(std::io::Error),
    /// The current platform has no kernel-backed lock implementation.
    UnsupportedPlatform,
}

impl std::fmt::Display for ServerLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy { owner } => match owner {
                Some(id) => write!(
                    f,
                    "snip-sync server already running (pid={}, nonce={})",
                    id.pid, id.nonce
                ),
                None => write!(f, "snip-sync server already running"),
            },
            Self::Io(e) => write!(f, "server lock io error: {e}"),
            Self::UnsupportedPlatform => write!(
                f,
                "server lock is not supported on this platform; refusing to weaken exclusion"
            ),
        }
    }
}

impl std::error::Error for ServerLockError {}

impl From<std::io::Error> for ServerLockError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// RAII guard for the kernel-backed server singleton lock.
#[derive(Debug)]
pub struct ServerLock {
    file: Option<File>,
    path: PathBuf,
    identity: ServerLockIdentity,
}

impl ServerLock {
    /// Path of the persistent lock file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Identity record published after acquisition.
    pub fn identity(&self) -> &ServerLockIdentity {
        &self.identity
    }

    /// Nonce generated at acquisition time.
    pub fn nonce(&self) -> &str {
        &self.identity.nonce
    }
}

impl Drop for ServerLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        release_unix_lock(self.file.as_ref());
        #[cfg(windows)]
        release_windows_lock(self.file.as_ref());
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

/// Path of the persistent server lock file.
pub fn server_lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join(SERVER_LOCK_NAME)
}

impl ServerLock {
    /// Try to acquire the singleton server lock without waiting.
    pub fn try_acquire(state_dir: &Path) -> Result<Self, ServerLockError> {
        let path = server_lock_path(state_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc != 0 {
                let err = std::io::Error::last_os_error();
                if matches!(err.raw_os_error(), Some(libc::EWOULDBLOCK)) {
                    let owner = read_owner(&path);
                    return Err(ServerLockError::Busy { owner });
                }
                return Err(ServerLockError::Io(err));
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
                if matches!(err.raw_os_error(), Some(33) | Some(32)) {
                    let owner = read_owner(&path);
                    return Err(ServerLockError::Busy { owner });
                }
                return Err(ServerLockError::Io(err));
            }
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = file;
            return Err(ServerLockError::UnsupportedPlatform);
        }

        let identity = ServerLockIdentity {
            schema_version: SERVER_LOCK_SCHEMA_VERSION,
            pid: std::process::id(),
            start_token: current_start_token(),
            nonce: generate_nonce(),
            acquired_at_unix_ms: unix_now_ms(),
        };

        #[cfg(unix)]
        {
            if let Ok(meta) = std::fs::metadata(&path) {
                let mut perms = meta.permissions();
                perms.set_mode(0o600);
                let _ = std::fs::set_permissions(&path, perms);
            }
        }

        if let Err(e) = publish_identity(&mut file, &identity) {
            #[cfg(unix)]
            release_unix_lock(Some(&file));
            #[cfg(windows)]
            release_windows_lock(Some(&file));
            return Err(ServerLockError::Io(e));
        }

        Ok(ServerLock {
            file: Some(file),
            path,
            identity,
        })
    }
}

fn publish_identity(file: &mut File, identity: &ServerLockIdentity) -> std::io::Result<()> {
    file.set_len(0)?;
    file.seek(std::io::SeekFrom::Start(0))?;
    let serialized = toml::to_string_pretty(identity)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    file.write_all(serialized.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

/// Best-effort read of the on-disk server lock metadata.
pub fn read_owner(path: &Path) -> Option<ServerLockIdentity> {
    let raw = std::fs::read_to_string(path).ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    toml::from_str(&raw).ok()
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_start_token_parser_reads_field_22() {
        let stat = "42 (name with ) parens) S f3 f4 f5 f6 f7 f8 f9 f10 f11 f12 f13 f14 f15 f16 f17 f18 f19 f20 FIELD21 START22";
        assert_eq!(parse_linux_proc_start_token(stat), Some("START22".into()));
        assert_ne!(parse_linux_proc_start_token(stat), Some("FIELD21".into()));
        assert_eq!(parse_linux_proc_start_token("1 (short) S f3"), None);
    }

    #[test]
    fn first_acquisition_succeeds() {
        let dir = tempfile::TempDir::new().unwrap();
        let guard = ServerLock::try_acquire(dir.path()).unwrap();
        assert_eq!(guard.identity().pid, std::process::id());
        assert!(!guard.identity().nonce.is_empty());
    }

    #[test]
    fn second_acquisition_in_same_process_returns_busy() {
        let dir = tempfile::TempDir::new().unwrap();
        let _first = ServerLock::try_acquire(dir.path()).unwrap();
        let second = ServerLock::try_acquire(dir.path());
        match second {
            Err(ServerLockError::Busy { owner }) => {
                assert!(owner.is_some());
            }
            other => panic!("expected Busy, got {other:?}"),
        }
    }

    #[test]
    fn drop_allows_later_acquisition() {
        let dir = tempfile::TempDir::new().unwrap();
        let first = ServerLock::try_acquire(dir.path()).unwrap();
        let nonce1 = first.nonce().to_string();
        drop(first);
        let second = ServerLock::try_acquire(dir.path()).unwrap();
        assert_ne!(second.nonce(), nonce1);
    }

    #[test]
    fn canonical_file_persists_after_release() {
        let dir = tempfile::TempDir::new().unwrap();
        {
            let _g = ServerLock::try_acquire(dir.path()).unwrap();
        }
        assert!(server_lock_path(dir.path()).exists());
    }

    #[test]
    fn empty_metadata_does_not_block_acquisition_when_lock_free() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = server_lock_path(dir.path());
        std::fs::write(&path, "").unwrap();
        let guard = ServerLock::try_acquire(dir.path()).unwrap();
        let observed = read_owner(&path).unwrap();
        assert_eq!(observed.nonce, guard.nonce());
    }

    #[test]
    fn malformed_metadata_does_not_block_acquisition_when_lock_free() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = server_lock_path(dir.path());
        std::fs::write(&path, "not valid toml").unwrap();
        let guard = ServerLock::try_acquire(dir.path()).unwrap();
        let observed = read_owner(&path).unwrap();
        assert_eq!(observed.nonce, guard.nonce());
    }

    #[test]
    fn identity_roundtrip() {
        let id = ServerLockIdentity {
            schema_version: 1,
            pid: 12345,
            start_token: Some("tok".to_string()),
            nonce: "nonce".to_string(),
            acquired_at_unix_ms: 999,
        };
        let s = toml::to_string_pretty(&id).unwrap();
        let back: ServerLockIdentity = toml::from_str(&s).unwrap();
        assert_eq!(id, back);
    }
}
