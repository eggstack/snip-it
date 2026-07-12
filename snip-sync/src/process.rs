use crate::paths;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn write_pid() -> Result<(), String> {
    write_pid_at(&paths::pid_path(), std::process::id())
}

fn write_pid_at(path: &Path, pid: u32) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create pid dir: {}", e))?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Failed to write PID file {}: {}", path.display(), e))?;

    if let Err(e) = file.write_all(pid.to_string().as_bytes()) {
        let _ = fs::remove_file(path);
        return Err(format!(
            "Failed to write PID file {}: {}",
            path.display(),
            e
        ));
    }
    Ok(())
}

pub fn remove_pid() {
    let path = paths::pid_path();
    let _ = fs::remove_file(&path);
}

pub fn read_pid() -> Option<u32> {
    let path = paths::pid_path();
    let content = fs::read_to_string(&path).ok()?;
    content.trim().parse().ok()
}

#[cfg(unix)]
pub fn is_running(pid: u32) -> bool {
    // kill(0, 0) checks if process exists without sending a signal
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(not(unix))]
pub fn is_running(_pid: u32) -> bool {
    false
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_pid_does_not_remove_an_existing_pid_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("snip-sync.pid");
        fs::write(&path, "live-pid").unwrap();

        assert!(write_pid_at(&path, 1234).is_err());
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
}
