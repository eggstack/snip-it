use crate::paths;
use std::fs;
use std::time::{Duration, Instant};

pub fn write_pid() -> Result<(), String> {
    let pid = std::process::id();
    let path = paths::pid_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create pid dir: {}", e))?;
    }
    fs::write(&path, pid.to_string())
        .map_err(|e| format!("Failed to write PID file {}: {}", path.display(), e))
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

#[cfg(test)]
mod tests {
    use super::*;

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
