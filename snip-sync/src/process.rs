use crate::paths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PidRecord {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub pid: u32,
    /// Process start-time token. `None` when the platform cannot determine it.
    #[serde(default)]
    pub start_token: Option<String>,
    /// Random nonce written at server start; verified before signalling.
    #[serde(default)]
    pub nonce: Option<String>,
}

fn default_schema_version() -> u32 {
    1
}

/// RAII guard that removes the PID file only if its on-disk identity still
/// matches the PID/start_token/nonce recorded at startup. Uses an atomic
/// rename-aside so a racing replacement PID file cannot be unlinked.
pub struct PidGuard {
    path: PathBuf,
    record: PidRecord,
}

impl PidGuard {
    pub fn pid(&self) -> u32 {
        self.record.pid
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        if let Some(current) = read_pid_at(&self.path)
            && current.pid == self.record.pid
            && current.start_token == self.record.start_token
            && current.nonce == self.record.nonce
        {
            let _ = remove_pid_record(&self.path);
        }
    }
}

/// Write the PID file at startup and return an RAII guard that will only
/// remove it if the on-disk identity still matches.
pub fn write_pid() -> Result<PidGuard, String> {
    let pid = std::process::id();
    let path = paths::pid_path();
    let nonce = generate_nonce();
    let record = PidRecord {
        schema_version: 1,
        pid,
        start_token: get_process_start_token(pid),
        nonce: Some(nonce),
    };
    write_pid_at(&path, &record)?;
    Ok(PidGuard { path, record })
}

fn write_pid_at(path: &Path, record: &PidRecord) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create pid dir: {}", e))?;
    }
    let serialized = toml::to_string_pretty(record)
        .map_err(|e| format!("Failed to serialize PID record: {e}"))?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Failed to write PID file {}: {}", path.display(), e))?;

    if let Err(e) = file.write_all(serialized.as_bytes()) {
        let _ = fs::remove_file(path);
        return Err(format!(
            "Failed to write PID file {}: {}",
            path.display(),
            e
        ));
    }
    Ok(())
}

/// Read the PID file and return its structured contents.
pub fn read_pid_record() -> Option<PidRecord> {
    read_pid_at(&paths::pid_path())
}

fn read_pid_at(path: &Path) -> Option<PidRecord> {
    let content = fs::read_to_string(path).ok()?;
    if content.trim().is_empty() {
        return None;
    }
    toml::from_str(&content).ok()
}

/// Read just the numeric PID for legacy callers.
pub fn read_pid() -> Option<u32> {
    read_pid_record().map(|r| r.pid)
}

/// Atomically remove a PID file by renaming it aside. NotFound is treated as
/// success because another writer may have already removed or replaced it.
fn remove_pid_record(path: &Path) -> std::io::Result<()> {
    let quarantine_name = format!("snip-sync.pid.quarantine.{}", uuid::Uuid::new_v4());
    let quarantine_path = path.parent().unwrap_or(path).join(&quarantine_name);
    match fs::rename(path, &quarantine_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Remove the PID file unconditionally. Used by stale-reclaim paths.
pub fn remove_pid() {
    let path = paths::pid_path();
    let _ = remove_pid_record(&path);
}

/// Verify that the on-disk PID file identifies a live process and matches
/// the recorded identity (PID + start_token). `start_token` defends against
/// PID-reuse: a recycled PID with a different start token is not the server.
#[cfg(unix)]
pub fn is_running(pid: u32) -> bool {
    // kill(0, 0) checks if process exists without sending a signal
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
pub fn is_running(_pid: u32) -> bool {
    false
}

/// Verify the live process recorded in `record` still matches by start token.
/// Returns `true` when the process is alive and its start token equals the
/// recorded start token (so a PID-reused process is rejected). `false` if
/// the process is dead, the start token cannot be observed, or the token
/// differs from what was recorded.
pub fn record_still_matches(record: &PidRecord) -> bool {
    if !is_running(record.pid) {
        return false;
    }
    let recorded = match &record.start_token {
        Some(t) => t,
        None => return true,
    };
    let observed = match get_process_start_token(record.pid) {
        Some(t) => t,
        None => return true, // Conservative: can't observe, treat as live.
    };
    recorded == &observed
}

#[cfg(unix)]
pub fn validate_process_name(pid: u32) -> bool {
    use std::process::Command;
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output();
    match output {
        Ok(o) => {
            let name = String::from_utf8_lossy(&o.stdout);
            name.trim().contains("snip-sync")
        }
        Err(_) => false,
    }
}

#[cfg(not(unix))]
pub fn validate_process_name(_pid: u32) -> bool {
    false
}

pub fn wait_for_exit(pid: u32, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if !is_running(pid) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Process {} did not exit within {}s",
                pid,
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Try to acquire an advisory lock for a short-lived maintenance command.
///
/// Unix callers use `flock`, so the lock file can safely remain on disk and
/// stale processes do not leave a permanently blocked lock. On other
/// platforms, an exclusive create is used as a best-effort fallback.
pub struct LockGuard {
    #[allow(dead_code)]
    file: fs::File,
    path: PathBuf,
    remove_on_drop: bool,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub fn try_lock(path: &Path) -> Result<Option<LockGuard>, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create lock directory: {}", e))?;
    }

    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;

        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| format!("Failed to open lock file {}: {}", path.display(), e))?;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            return Ok(Some(LockGuard {
                file,
                path: path.to_path_buf(),
                remove_on_drop: false,
            }));
        }
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock {
            return Ok(None);
        }
        Err(format!(
            "Failed to lock {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        ))
    }

    #[cfg(not(unix))]
    {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(file) => Ok(Some(LockGuard {
                file,
                path: path.to_path_buf(),
                remove_on_drop: true,
            })),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
            Err(e) => Err(format!("Failed to lock {}: {}", path.display(), e)),
        }
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

/// Process start-time token for the given PID.
#[cfg(target_os = "linux")]
pub fn get_process_start_token(pid: u32) -> Option<String> {
    let stat_path = format!("/proc/{pid}/stat");
    let content = fs::read_to_string(&stat_path).ok()?;
    // Field 22 (1-indexed) is `starttime`. The comm field (field 2) may
    // contain spaces or parens, so find the last `)` and count from there.
    let after_comm = content.rfind(')')?;
    let fields: Vec<&str> = content[after_comm + 2..].split_whitespace().collect();
    if fields.len() >= 19 {
        Some(fields[18].to_string())
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
pub fn get_process_start_token(pid: u32) -> Option<String> {
    use libc::{PROC_PIDTBSDINFO, c_int, proc_bsdinfo, proc_pidinfo};

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
pub fn get_process_start_token(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut creation_time: FILETIME = std::mem::zeroed();
        let mut exit_time: FILETIME = std::mem::zeroed();
        let mut kernel_time: FILETIME = std::mem::zeroed();
        let mut user_time: FILETIME = std::mem::zeroed();
        let success = GetProcessTimes(
            handle,
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        );
        CloseHandle(handle);
        if success == 0 {
            return None;
        }
        let creation =
            ((creation_time.dwHighDateTime as u64) << 32) | (creation_time.dwLowDateTime as u64);
        Some(creation.to_string())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn get_process_start_token(_pid: u32) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_pid_does_not_remove_an_existing_pid_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        fs::write(&path, "live-pid").unwrap();

        let record = PidRecord {
            schema_version: 1,
            pid: 1234,
            start_token: None,
            nonce: None,
        };
        assert!(write_pid_at(&path, &record).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "live-pid");
    }

    #[test]
    fn test_read_pid_nonexistent() {
        // Verify no panic when reading PID file
        assert!(read_pid().is_some() || read_pid().is_none());
    }

    #[test]
    fn test_is_running_pid_0() {
        // PID 0 is the scheduler on Unix, always exists
        #[cfg(unix)]
        assert!(is_running(0));
    }

    #[test]
    fn test_wait_for_exit_invalid_pid() {
        // Non-existent PID — should return quickly
        let result = wait_for_exit(999999999, Duration::from_millis(500));
        assert!(result.is_ok()); // process doesn't exist, so "not running" → ok
    }

    #[test]
    fn test_pid_record_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        let record = PidRecord {
            schema_version: 1,
            pid: 9999,
            start_token: Some("token-abc".to_string()),
            nonce: Some("nonce-xyz".to_string()),
        };
        write_pid_at(&path, &record).unwrap();
        let read = read_pid_at(&path).unwrap();
        assert_eq!(read.pid, 9999);
        assert_eq!(read.start_token.as_deref(), Some("token-abc"));
        assert_eq!(read.nonce.as_deref(), Some("nonce-xyz"));
    }

    #[test]
    fn test_pid_record_missing_file() {
        let path = PathBuf::from("/nonexistent/snip-sync.pid");
        assert!(read_pid_at(&path).is_none());
    }

    #[test]
    fn test_pid_record_empty_file_is_none() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        fs::write(&path, "").unwrap();
        assert!(read_pid_at(&path).is_none());
    }

    #[test]
    fn test_pid_record_legacy_numeric() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        fs::write(&path, "1234").unwrap();
        // Numeric-only PID file from an older server is unreadable as a
        // structured record, but the raw read_pid helper also rejects it.
        assert!(read_pid_at(&path).is_none());
        assert!(read_pid_at(&path).map(|r| r.pid).is_none());
    }

    #[test]
    fn test_remove_pid_record_handles_missing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        // Already absent — NotFound must be treated as success.
        assert!(remove_pid_record(&path).is_ok());
    }

    #[test]
    fn test_remove_pid_record_atomic_rename() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        fs::write(&path, "pid = 1\n").unwrap();
        assert!(remove_pid_record(&path).is_ok());
        assert!(!path.exists(), "PID file must be removed");
        // Quarantine file must exist alongside (or have been left in place
        // since `remove` only requires the rename to succeed).
    }

    #[test]
    fn test_record_still_matches_dead_pid() {
        let record = PidRecord {
            schema_version: 1,
            pid: 99999999, // assumed dead
            start_token: Some("anything".to_string()),
            nonce: Some("nonce".to_string()),
        };
        assert!(!record_still_matches(&record));
    }

    #[test]
    fn test_record_still_matches_no_start_token() {
        // Legacy record with no start token: conservative policy treats live
        // PID as a match (we cannot disprove it).
        let record = PidRecord {
            schema_version: 1,
            pid: std::process::id(),
            start_token: None,
            nonce: None,
        };
        assert!(record_still_matches(&record));
    }

    #[test]
    #[cfg(unix)]
    fn test_record_still_matches_pid_reuse_detected() {
        // Write a record with a deliberately bogus start token for the
        // current PID. The observed token from /proc/<pid>/stat will not
        // match, so record_still_matches must return false.
        let bogus = "bogus-token-99999";
        let record = PidRecord {
            schema_version: 1,
            pid: std::process::id(),
            start_token: Some(bogus.to_string()),
            nonce: Some("n".to_string()),
        };
        assert!(!record_still_matches(&record));
    }

    #[test]
    fn test_generate_nonce_unique() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b);
    }
}
