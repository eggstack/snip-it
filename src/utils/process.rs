//! Shared process-liveness and owned-lock-file helpers.
//!
//! Used by the transaction, local-data, and execution locks so liveness
//! semantics cannot diverge between implementations.

use std::fs;
use std::path::Path;

/// Check whether a process with the given PID is alive.
///
/// PID 0 is never a valid lock owner: `kill(0, 0)` targets the caller's
/// process group and would always succeed, so it is treated as dead.
#[cfg(unix)]
pub(crate) fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let rc = unsafe { libc::kill(pid as i32, 0) };
    rc == 0 || classify_kill_zero_error(std::io::Error::last_os_error().raw_os_error())
}

#[cfg(unix)]
pub(crate) fn classify_kill_zero_error(errno: Option<i32>) -> bool {
    !matches!(errno, Some(libc::ESRCH))
}

/// Check whether a process with the given PID is alive.
///
/// PID 0 is never a valid lock owner and is treated as dead. A live process
/// reports `STILL_ACTIVE` as its exit code; any other code means exited.
#[cfg(windows)]
pub(crate) fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        ok != 0 && exit_code == STILL_ACTIVE as u32
    }
}

/// Remove a lock file only if it still holds our ownership record.
///
/// The record is verified by reading through an opened file handle (nonce,
/// pid, start-token must all match). On Unix the handle's `(dev, ino)` is
/// then compared against a fresh stat of `path`, closing the TOCTOU window
/// where a concurrent quarantine + re-acquire could swap a different
/// owner's lock into place between verification and unlink. Returns `true`
/// only when this call removed the file.
pub(crate) fn remove_owned_lock_file(
    path: &Path,
    owner_nonce: &str,
    owner_pid: u32,
    owner_start_token: Option<&str>,
) -> bool {
    use std::io::Read as _;

    // Read through the opened handle so verification and the metadata
    // identity check below refer to the same inode.
    let Ok(mut handle) = fs::File::open(path) else {
        return false;
    };
    let mut content = String::new();
    if handle.read_to_string(&mut content).is_err() {
        return false;
    }

    // Lock records are TOML documents (top-level tables), so parse into
    // `toml::Table` rather than `toml::Value`, which only accepts single
    // standalone values.
    let Ok(record) = toml::from_str::<toml::Table>(&content) else {
        return false;
    };

    let nonce_matches = record.get("nonce").and_then(|v| v.as_str()) == Some(owner_nonce);
    let pid_matches = record.get("pid").and_then(|v| v.as_integer()) == Some(i64::from(owner_pid));
    let recorded_start_token = match record.get("start_token") {
        Some(toml::Value::String(s)) => Some(s.as_str()),
        _ => None,
    };
    if !nonce_matches || !pid_matches || recorded_start_token != owner_start_token {
        return false;
    }

    // Identity verified by content; confirm the file at `path` is still the
    // same file we just verified before unlinking.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (handle.metadata(), fs::metadata(path)) {
            (Ok(h), Ok(p)) if h.dev() == p.dev() && h.ino() == p.ino() => {}
            _ => return false,
        }
    }
    #[cfg(windows)]
    {
        // No stable file-id API in std; the handle-based content check above
        // is the strongest available verification.
        let _ = &handle;
    }

    fs::remove_file(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record(start_token: Option<&str>) -> String {
        let token_line = match start_token {
            Some(t) => format!("start_token = \"{t}\"\n"),
            None => String::new(),
        };
        format!("schema_version = 1\npid = 4242\nnonce = \"abc\"\n{token_line}")
    }

    #[test]
    fn removes_matching_record_with_handle_metadata_check() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("x.lock");
        std::fs::write(&path, sample_record(Some("tok-1"))).unwrap();

        let removed = remove_owned_lock_file(&path, "abc", 4242, Some("tok-1"));
        assert!(removed, "expected removal");
        assert!(!path.exists());
    }

    #[test]
    fn refuses_nonce_mismatch() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("x.lock");
        std::fs::write(&path, sample_record(Some("tok-1"))).unwrap();
        assert!(!remove_owned_lock_file(&path, "other", 4242, Some("tok-1")));
        assert!(path.exists());
    }

    #[test]
    fn refuses_pid_mismatch() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("x.lock");
        std::fs::write(&path, sample_record(Some("tok-1"))).unwrap();
        assert!(!remove_owned_lock_file(&path, "abc", 7, Some("tok-1")));
        assert!(path.exists());
    }

    #[test]
    fn refuses_start_token_mismatch() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("x.lock");
        std::fs::write(&path, sample_record(Some("tok-1"))).unwrap();
        assert!(!remove_owned_lock_file(&path, "abc", 4242, Some("tok-2")));
        assert!(path.exists());
        assert!(!remove_owned_lock_file(&path, "abc", 4242, None));
        assert!(path.exists());
    }
}
