use crate::paths;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
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

/// Typed result of [`parse_pid_file`]. Distinguishes the on-disk formats
/// the server may legitimately encounter:
///
/// - `Structured` — a current structured `PidRecord`.
/// - `LegacyPid(pid)` — a numeric-only PID file from older versions.
/// - `Empty` — an empty or whitespace-only file. Treated as an explicit
///   state, not as `None`, so callers can replace it cleanly.
/// - `Malformed` — content that is neither structured nor a plain
///   integer. Reported explicitly so callers do not silently treat it
///   as absence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedPidFile {
    Structured(PidRecord),
    LegacyPid(u32),
    Empty,
    Malformed(String),
}

impl PartialEq for PidRecord {
    fn eq(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.pid == other.pid
            && self.start_token == other.start_token
            && self.nonce == other.nonce
    }
}

impl Eq for PidRecord {}

/// Parse the on-disk PID file at `path`.
///
/// Parsing order:
/// 1. Trim contents.
/// 2. Empty → `Empty`.
/// 3. All-decimal numeric value → `LegacyPid`.
/// 4. Structured TOML → `Structured`.
/// 5. Otherwise → `Malformed` with the raw text.
pub fn parse_pid_file(path: &Path) -> ParsedPidFile {
    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
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
        Ok(rec) => ParsedPidFile::Structured(rec),
        Err(e) => ParsedPidFile::Malformed(e.to_string()),
    }
}

/// Reconcile the existing PID file under the protection of the server
/// singleton lock. Behavior:
///
/// - Structured record for the current owner: leave it (overwrite
///   happens later in [`write_pid`]).
/// - Stale structured record (dead PID or token mismatch): overwrite
///   later via [`write_pid`]. No deletion here — the kernel lock
///   guarantees we are the only writer.
/// - Legacy numeric PID that is alive AND process name verifies as
///   `snip-sync`: refuse startup. An older server that does not hold
///   the new kernel lock may still be running and we must not race it.
/// - Legacy numeric PID that is dead: OK to overwrite.
/// - Legacy numeric PID alive but process name cannot be verified:
///   refuse startup as a conservative safety measure.
/// - Empty or malformed record: log a warning, replace later via
///   [`write_pid`].
pub fn reconcile_pid_under_lock(state_dir: &Path) -> Result<(), String> {
    let path = state_dir.join("snip-sync.pid");
    if !path.exists() {
        return Ok(());
    }
    let parsed = parse_pid_file(&path);
    match parsed {
        ParsedPidFile::Structured(rec) => {
            if record_still_matches(&rec) {
                // A live structured record belongs to another running
                // server that does not hold the new kernel lock.
                // Refuse to start.
                return Err(format!(
                    "snip-sync server already running with PID {}. Use 'snip-sync stop' first.",
                    rec.pid
                ));
            }
            tracing::warn!(
                "Found stale structured PID file for process {}. It will be replaced.",
                rec.pid
            );
            Ok(())
        }
        ParsedPidFile::LegacyPid(pid) => {
            if is_running(pid) {
                #[cfg(unix)]
                {
                    if !validate_process_name(pid) {
                        return Err(format!(
                            "Legacy PID file references PID {} but process name cannot be verified as snip-sync. Refusing to start; investigate manually or remove the file.",
                            pid
                        ));
                    }
                    return Err(format!(
                        "Legacy snip-sync server appears to be running with PID {}. Stop it before starting a new server.",
                        pid
                    ));
                }
                #[cfg(not(unix))]
                {
                    return Err(format!(
                        "Legacy PID file references PID {} but Windows process verification cannot confirm snip-sync. Refusing to start; investigate manually or remove the file.",
                        pid
                    ));
                }
            }
            tracing::warn!(
                "Found legacy PID file for dead process {}. It will be replaced.",
                pid
            );
            Ok(())
        }
        ParsedPidFile::Empty => {
            tracing::warn!("PID file is empty; it will be replaced.");
            Ok(())
        }
        ParsedPidFile::Malformed(msg) => {
            tracing::warn!("PID file is malformed ({}); it will be replaced.", msg);
            Ok(())
        }
    }
}

/// RAII guard that removes the PID file only if its on-disk identity
/// still matches the PID/start_token/nonce recorded at startup. Uses
/// identity-checked unlink (no quarantine) so a concurrent replacement
/// record cannot be lost.
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
            let _ = remove_pid_atomic(&self.path);
        }
    }
}

/// Write the PID file at startup and return an RAII guard that will only
/// remove it if the on-disk identity still matches.
///
/// Publication is atomic: a unique temp file is written, synced, and
/// renamed over the canonical path. A crash before the rename leaves no
/// partial record at the canonical path.
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
    write_pid_atomic(&path, &record)?;
    Ok(PidGuard { path, record })
}

fn write_pid_atomic(path: &Path, record: &PidRecord) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create pid dir: {}", e))?;
    }
    let serialized = toml::to_string_pretty(record)
        .map_err(|e| format!("Failed to serialize PID record: {e}"))?;

    let temp_path = unique_temp_path(path);
    {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        let mut file = options.open(&temp_path).map_err(|e| {
            format!(
                "Failed to create temp PID file {}: {}",
                temp_path.display(),
                e
            )
        })?;
        #[cfg(unix)]
        restrict_permissions_unix(&temp_path);
        if let Err(e) = file.write_all(serialized.as_bytes()) {
            let _ = fs::remove_file(&temp_path);
            return Err(format!("Failed to write temp PID file: {e}"));
        }
        if let Err(e) = file.sync_all() {
            let _ = fs::remove_file(&temp_path);
            return Err(format!("Failed to sync temp PID file: {e}"));
        }
    }

    // Atomic rename. On Unix `rename` overwrites atomically; on Windows
    // we use `MoveFileExW` with replace-existing + write-through.
    if let Err(e) = replace_existing(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "Failed to atomically install PID file {}: {}",
            path.display(),
            e
        ));
    }
    let _ = fs::remove_file(&temp_path);
    fsync_parent_dir(path);
    Ok(())
}

fn unique_temp_path(final_path: &Path) -> PathBuf {
    let parent = final_path.parent().unwrap_or_else(|| Path::new("."));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let stem = final_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("snip-sync");
    let ext = final_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("pid");
    parent.join(format!(".{stem}-{}-{nanos}.{ext}.tmp", std::process::id()))
}

#[cfg(unix)]
fn replace_existing(from: &Path, to: &Path) -> Result<(), std::io::Error> {
    fs::rename(from, to)
}

#[cfg(windows)]
fn replace_existing(from: &Path, to: &Path) -> Result<(), std::io::Error> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let from_wide: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to_wide: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        MoveFileExW(
            from_wide.as_ptr(),
            to_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(unix)]
fn restrict_permissions_unix(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o600);
        let _ = fs::set_permissions(path, perms);
    }
}

fn fsync_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        #[cfg(unix)]
        {
            if let Ok(f) = fs::OpenOptions::new().read(true).open(parent) {
                let _ = f.sync_all();
            }
        }
        #[cfg(not(unix))]
        {
            let _ = parent;
        }
    }
}

/// Read the PID file and return its structured contents.
pub fn read_pid_record() -> Option<PidRecord> {
    read_pid_at(&paths::pid_path())
}

/// CLI support helper: remove the PID file only if it still matches `expected`.
///
/// The caller must hold the server singleton lock. This is public only because
/// the package binary and library are separate Rust crates; it is not a general
/// lock-ownership or reclamation API. Legacy numeric records are compared by
/// PID alone; structured records use all of their identity fields. Other parsed
/// states are never removable.
#[doc(hidden)]
pub fn remove_pid_if_unchanged(expected: &ParsedPidFile) {
    remove_pid_if_unchanged_at(&paths::pid_path(), expected);
}

fn remove_pid_if_unchanged_at(path: &Path, expected: &ParsedPidFile) {
    let matches = match (expected, parse_pid_file(path)) {
        (ParsedPidFile::LegacyPid(expected_pid), ParsedPidFile::LegacyPid(current_pid)) => {
            expected_pid == &current_pid
        }
        (ParsedPidFile::Structured(expected_record), ParsedPidFile::Structured(current_record)) => {
            expected_record == &current_record
        }
        _ => false,
    };
    if matches {
        let _ = remove_pid_atomic(path);
    }
}

fn read_pid_at(path: &Path) -> Option<PidRecord> {
    match parse_pid_file(path) {
        ParsedPidFile::Structured(rec) => Some(rec),
        _ => None,
    }
}

/// Read just the numeric PID for legacy callers.
pub fn read_pid() -> Option<u32> {
    match parse_pid_file(&paths::pid_path()) {
        ParsedPidFile::Structured(rec) => Some(rec.pid),
        ParsedPidFile::LegacyPid(pid) => Some(pid),
        _ => None,
    }
}

/// Identity-checked unlink. The caller must have already verified that
/// the on-disk record still identifies the current owner. Missing file
/// is treated as success — a concurrent process may have already
/// removed it.
fn remove_pid_atomic(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Remove the PID file unconditionally. Used by stale-reclaim paths
/// after the caller has confirmed the on-disk record does not identify a
/// live owner.
pub fn remove_pid() {
    let path = paths::pid_path();
    let _ = remove_pid_atomic(&path);
}

/// Verify that the on-disk PID file identifies a live process and matches
/// the recorded identity (PID + start_token). `start_token` defends
/// against PID-reuse: a recycled PID with a different start token is not
/// the server.
#[cfg(unix)]
pub fn is_running(pid: u32) -> bool {
    // kill(0, 0) checks if process exists without sending a signal
    unsafe { libc::kill(pid as i32, 0) == 0 }
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
        let mut exit_code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        if ok == 0 {
            return false;
        }
        exit_code == STILL_ACTIVE as u32
    }
}

#[cfg(not(any(unix, windows)))]
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

pub(crate) fn generate_nonce() -> String {
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
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
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
    fn test_parse_pid_file_structured_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        let record = PidRecord {
            schema_version: 1,
            pid: 9999,
            start_token: Some("token-abc".to_string()),
            nonce: Some("nonce-xyz".to_string()),
        };
        let serialized = toml::to_string_pretty(&record).unwrap();
        std::fs::write(&path, serialized).unwrap();
        match parse_pid_file(&path) {
            ParsedPidFile::Structured(rec) => {
                assert_eq!(rec.pid, 9999);
                assert_eq!(rec.start_token.as_deref(), Some("token-abc"));
            }
            other => panic!("expected Structured, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_pid_file_legacy_numeric() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        std::fs::write(&path, "1234\n").unwrap();
        match parse_pid_file(&path) {
            ParsedPidFile::LegacyPid(pid) => assert_eq!(pid, 1234),
            other => panic!("expected LegacyPid, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_pid_file_empty() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        std::fs::write(&path, "   \n").unwrap();
        assert_eq!(parse_pid_file(&path), ParsedPidFile::Empty);
    }

    #[test]
    fn test_parse_pid_file_missing_is_empty() {
        let path = PathBuf::from("/nonexistent/snip-sync.pid");
        assert_eq!(parse_pid_file(&path), ParsedPidFile::Empty);
    }

    #[test]
    fn test_parse_pid_file_malformed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        std::fs::write(&path, "this is not valid toml nor numeric").unwrap();
        match parse_pid_file(&path) {
            ParsedPidFile::Malformed(_) => {}
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn test_remove_legacy_pid_only_when_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        std::fs::write(&path, "1234\n").unwrap();

        remove_pid_if_unchanged_at(&path, &ParsedPidFile::LegacyPid(9999));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "1234\n");

        remove_pid_if_unchanged_at(&path, &ParsedPidFile::LegacyPid(1234));
        assert!(!path.exists());
    }

    #[test]
    fn test_write_pid_does_not_remove_an_existing_pid_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        fs::write(&path, "live-pid\n").unwrap();

        // Pre-existing legacy file is NOT replaced by atomic install if
        // the kernel lock is held and reconcile passes (it would). The
        // reconcile logic checks liveness; we are not running a server,
        // so the legacy PID is dead and replace is allowed.
        let record = PidRecord {
            schema_version: 1,
            pid: 1234,
            start_token: None,
            nonce: Some("n".to_string()),
        };
        write_pid_atomic(&path, &record).unwrap();
        let parsed = parse_pid_file(&path);
        assert!(matches!(parsed, ParsedPidFile::Structured(_)));
    }

    #[test]
    fn test_write_pid_atomic_installs_record() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        let record = PidRecord {
            schema_version: 1,
            pid: 7777,
            start_token: Some("t".to_string()),
            nonce: Some("n".to_string()),
        };
        write_pid_atomic(&path, &record).unwrap();
        let parsed = parse_pid_file(&path);
        match parsed {
            ParsedPidFile::Structured(rec) => {
                assert_eq!(rec.pid, 7777);
            }
            other => panic!("expected Structured, got {other:?}"),
        }
    }

    #[test]
    fn test_write_pid_atomic_overwrites_existing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        let record1 = PidRecord {
            schema_version: 1,
            pid: 1000,
            start_token: Some("t1".to_string()),
            nonce: Some("n1".to_string()),
        };
        write_pid_atomic(&path, &record1).unwrap();
        let record2 = PidRecord {
            schema_version: 1,
            pid: 2000,
            start_token: Some("t2".to_string()),
            nonce: Some("n2".to_string()),
        };
        write_pid_atomic(&path, &record2).unwrap();
        match parse_pid_file(&path) {
            ParsedPidFile::Structured(rec) => {
                assert_eq!(rec.pid, 2000);
            }
            other => panic!("expected Structured, got {other:?}"),
        }
    }

    #[test]
    fn test_remove_pid_atomic_removes_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        fs::write(&path, "test").unwrap();
        assert!(remove_pid_atomic(&path).is_ok());
        assert!(!path.exists());
    }

    #[test]
    fn test_remove_pid_atomic_missing_is_ok() {
        let path = PathBuf::from("/nonexistent/snip-sync.pid");
        assert!(remove_pid_atomic(&path).is_ok());
    }

    #[test]
    fn test_pid_guard_does_not_remove_replacement() {
        // Simulate the race: a guard is dropped, but a replacement
        // server has already published a new PID record. The guard
        // must NOT unlink the replacement record.
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        let record = PidRecord {
            schema_version: 1,
            pid: std::process::id(),
            start_token: Some("t".to_string()),
            nonce: Some("n".to_string()),
        };
        write_pid_atomic(&path, &record).unwrap();
        let guard = PidGuard {
            path: path.clone(),
            record,
        };
        // Replace with a different record (e.g. another server took over).
        let replacement = PidRecord {
            schema_version: 1,
            pid: 1234,
            start_token: Some("replacement-token".to_string()),
            nonce: Some("replacement-nonce".to_string()),
        };
        write_pid_atomic(&path, &replacement).unwrap();
        drop(guard);
        // Replacement record must still be present.
        match parse_pid_file(&path) {
            ParsedPidFile::Structured(rec) => assert_eq!(rec.pid, 1234),
            other => panic!("expected Structured replacement, got {other:?}"),
        }
    }

    #[test]
    fn test_record_still_matches_dead_pid() {
        let record = PidRecord {
            schema_version: 1,
            pid: 99999999,
            start_token: Some("anything".to_string()),
            nonce: Some("nonce".to_string()),
        };
        assert!(!record_still_matches(&record));
    }

    #[test]
    fn test_record_still_matches_no_start_token() {
        // Legacy record with no start token: conservative policy treats
        // live PID as a match.
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

    #[test]
    #[cfg(unix)]
    fn test_reconcile_pid_under_lock_dead_legacy_is_ok() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("snip-sync.pid"), "99999999").unwrap();
        assert!(reconcile_pid_under_lock(temp.path()).is_ok());
    }

    #[test]
    #[cfg(unix)]
    fn test_reconcile_pid_under_lock_empty_is_ok() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("snip-sync.pid"), "").unwrap();
        assert!(reconcile_pid_under_lock(temp.path()).is_ok());
    }

    #[test]
    fn test_reconcile_pid_under_lock_missing_is_ok() {
        let temp = tempfile::tempdir().unwrap();
        assert!(reconcile_pid_under_lock(temp.path()).is_ok());
    }

    #[test]
    fn test_reconcile_pid_under_lock_malformed_is_ok() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("snip-sync.pid"),
            "this is not valid toml nor numeric",
        )
        .unwrap();
        assert!(reconcile_pid_under_lock(temp.path()).is_ok());
    }
}
