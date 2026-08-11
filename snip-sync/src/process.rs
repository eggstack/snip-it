//! Process helpers used by stop/restart and the croncheck maintenance lock.
//!
//! The kernel-backed server lock is the current owner record. This module
//! only parses old PID files as a bounded upgrade fallback and provides
//! process-identity checks before a signal is sent.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct PidRecord {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub pid: u32,
    #[serde(default)]
    pub start_token: Option<String>,
    #[serde(default)]
    pub nonce: Option<String>,
}

fn default_schema_version() -> u32 {
    1
}

/// PID-file formats retained only for upgrades from older releases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedPidFile {
    Structured(PidRecord),
    LegacyPid(u32),
    Empty,
    Malformed(String),
}

pub fn parse_pid_file(path: &Path) -> ParsedPidFile {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return ParsedPidFile::Empty;
        }
        Err(_) => return ParsedPidFile::Empty,
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ParsedPidFile::Empty;
    }
    if let Ok(pid) = trimmed.parse::<u32>() {
        return ParsedPidFile::LegacyPid(pid);
    }
    match toml::from_str::<PidRecord>(trimmed) {
        Ok(record) => ParsedPidFile::Structured(record),
        Err(error) => ParsedPidFile::Malformed(error.to_string()),
    }
}

/// Remove an old PID file only when it still contains the record the caller
/// inspected. Current servers never create this file.
#[doc(hidden)]
pub fn remove_pid_if_unchanged(expected: &ParsedPidFile) {
    let path = crate::paths::pid_path();
    let matches = match (expected, parse_pid_file(&path)) {
        (ParsedPidFile::LegacyPid(expected), ParsedPidFile::LegacyPid(current)) => {
            expected == &current
        }
        (ParsedPidFile::Structured(expected), ParsedPidFile::Structured(current)) => {
            expected == &current
        }
        _ => false,
    };
    if matches {
        let _ = fs::remove_file(path);
    }
}

pub fn read_pid() -> Option<u32> {
    match parse_pid_file(&crate::paths::pid_path()) {
        ParsedPidFile::Structured(record) => Some(record.pid),
        ParsedPidFile::LegacyPid(pid) => Some(pid),
        _ => None,
    }
}

#[cfg(unix)]
pub fn is_running(pid: u32) -> bool {
    if pid == 0 {
        return true;
    }
    let result = unsafe { libc::kill(pid as i32, 0) };
    result == 0
        || !matches!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        )
}

#[cfg(windows)]
pub fn is_running(pid: u32) -> bool {
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
        let mut exit_code = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        ok != 0 && exit_code == STILL_ACTIVE as u32
    }
}

#[cfg(not(any(unix, windows)))]
pub fn is_running(_pid: u32) -> bool {
    false
}

pub fn record_still_matches(record: &PidRecord) -> bool {
    if !is_running(record.pid) {
        return false;
    }
    let Some(expected) = &record.start_token else {
        return true;
    };
    get_process_start_token(record.pid).is_none_or(|observed| observed == *expected)
}

#[cfg(unix)]
pub fn validate_process_name(pid: u32) -> bool {
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output();
    match output {
        Ok(output) => String::from_utf8_lossy(&output.stdout)
            .trim()
            .contains("snip-sync"),
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
            .map_err(|error| format!("Failed to create lock directory: {error}"))?;
    }

    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|error| format!("Failed to open lock file {}: {error}", path.display()))?;
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
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
            Err(error) => Err(format!("Failed to lock {}: {error}", path.display())),
        }
    }
}

#[cfg(target_os = "linux")]
pub fn get_process_start_token(pid: u32) -> Option<String> {
    let content = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_linux_proc_start_token(&content)
}

#[cfg(target_os = "linux")]
fn parse_linux_proc_start_token(stat: &str) -> Option<String> {
    let after_comm = stat.rfind(')')?;
    stat.get(after_comm + 2..)?
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}

#[cfg(target_os = "macos")]
pub fn get_process_start_token(pid: u32) -> Option<String> {
    use libc::{PROC_PIDTBSDINFO, c_int, proc_bsdinfo, proc_pidinfo};
    let mut info: proc_bsdinfo = unsafe { std::mem::zeroed() };
    let result = unsafe {
        proc_pidinfo(
            pid as c_int,
            PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut _,
            std::mem::size_of::<proc_bsdinfo>() as i32,
        )
    };
    (result > 0).then(|| format!("{}.{:06}", info.pbi_start_tvsec, info.pbi_start_tvusec))
}

#[cfg(windows)]
pub fn get_process_start_token(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut creation: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user);
        CloseHandle(handle);
        (ok != 0).then(|| {
            (((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64).to_string()
        })
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn get_process_start_token(_pid: u32) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_start_token_parser_reads_field_22() {
        let stat = "42 (name with ) parens) S f4 f5 f6 f7 f8 f9 f10 f11 f12 f13 f14 f15 f16 f17 f18 f19 f20 FIELD21 START22";
        assert_eq!(parse_linux_proc_start_token(stat), Some("START22".into()));
        assert_eq!(parse_linux_proc_start_token("1 (short) S f3"), None);
    }

    #[test]
    fn test_parse_pid_file_legacy_numeric() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        fs::write(&path, "1234\n").unwrap();
        assert_eq!(parse_pid_file(&path), ParsedPidFile::LegacyPid(1234));
    }

    #[test]
    fn test_remove_legacy_pid_only_when_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        fs::write(&path, "1234\n").unwrap();
        remove_pid_if_unchanged_at(&path, &ParsedPidFile::LegacyPid(9999));
        assert!(path.exists());
        remove_pid_if_unchanged_at(&path, &ParsedPidFile::LegacyPid(1234));
        assert!(!path.exists());
    }

    fn remove_pid_if_unchanged_at(path: &Path, expected: &ParsedPidFile) {
        let matches = match (expected, parse_pid_file(path)) {
            (ParsedPidFile::LegacyPid(expected), ParsedPidFile::LegacyPid(current)) => {
                expected == &current
            }
            _ => false,
        };
        if matches {
            let _ = fs::remove_file(path);
        }
    }
}
