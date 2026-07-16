//! Short-lived transaction lock for pending-marker operations.
//!
//! `PendingTxnGuard` serializes read-modify-write critical sections on the
//! pending marker. It is intentionally distinct from the long-lived worker
//! execution lock (`lock::WorkerLock`): parent mutation commands hold this
//! guard only for the minimum time needed to read, compute, and write.

use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const PENDING_TXN_LOCK_NAME: &str = "auto-sync-pending.lock";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingTxnLockContents {
    pid: u32,
    nonce: String,
    created_at_unix_ms: u64,
}

#[derive(Debug)]
pub enum PendingTxnLockError {
    Io(std::io::Error),
    Busy { timeout_ms: u64 },
    Corrupted(String),
}

impl std::fmt::Display for PendingTxnLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Busy { timeout_ms } => {
                write!(f, "pending transaction lock busy after {timeout_ms}ms")
            }
            Self::Corrupted(msg) => write!(f, "corrupted pending txn lock: {msg}"),
        }
    }
}

impl std::error::Error for PendingTxnLockError {}

pub struct PendingTxnGuard {
    path: PathBuf,
    nonce: String,
}

impl PendingTxnGuard {
    pub fn nonce(&self) -> &str {
        &self.nonce
    }
}

impl Drop for PendingTxnGuard {
    fn drop(&mut self) {
        if let Ok(Some(contents)) = read_lock_contents(&self.path)
            && contents.pid == std::process::id()
            && contents.nonce == self.nonce
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub fn pending_txn_lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join(PENDING_TXN_LOCK_NAME)
}

/// Acquires the pending transaction lock with bounded retry.
///
/// Retries up to `timeout` total, with 1-5ms random jitter between attempts.
/// Live owners are never reclaimed regardless of age. Dead owners (PID not
/// alive) are reclaimed immediately.
pub fn acquire_pending_txn(
    state_dir: &Path,
    timeout: Duration,
) -> Result<PendingTxnGuard, PendingTxnLockError> {
    let path = pending_txn_lock_path(state_dir);
    let deadline = std::time::Instant::now() + timeout;
    let mut attempts = 0u64;

    loop {
        attempts += 1;

        if let Some(contents) = read_lock_contents(&path)? {
            if process_alive(contents.pid) {
                if std::time::Instant::now() >= deadline {
                    return Err(PendingTxnLockError::Busy {
                        timeout_ms: timeout.as_millis() as u64,
                    });
                }
                let jitter_ms = 1 + (attempts % 5);
                std::thread::sleep(Duration::from_millis(jitter_ms));
                continue;
            }
            let _ = std::fs::remove_file(&path);
        }

        let nonce = generate_nonce();
        let contents = PendingTxnLockContents {
            pid: std::process::id(),
            nonce: nonce.clone(),
            created_at_unix_ms: unix_now_ms(),
        };

        let serialized = toml::to_string_pretty(&contents)
            .map_err(|e| PendingTxnLockError::Io(std::io::Error::other(e)))?;
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(PendingTxnLockError::Io)?;
        f.write_all(serialized.as_bytes())
            .map_err(PendingTxnLockError::Io)?;
        f.sync_all().map_err(PendingTxnLockError::Io)?;
        restrict_permissions(&path);

        return Ok(PendingTxnGuard { path, nonce });
    }
}

fn read_lock_contents(path: &Path) -> Result<Option<PendingTxnLockContents>, PendingTxnLockError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let parsed: PendingTxnLockContents = toml::from_str(&contents)
                .map_err(|e| PendingTxnLockError::Corrupted(e.to_string()))?;
            Ok(Some(parsed))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PendingTxnLockError::Io(e)),
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    const SIGNAL_NOOP: i32 = 0;
    unsafe { kill(pid as i32, SIGNAL_NOOP) == 0 }
}

#[cfg(not(unix))]
fn process_alive(pid: u32) -> bool {
    let _ = pid;
    true
}

fn generate_nonce() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{:x}", std::process::id(), nanos)
}

fn restrict_permissions(path: &Path) {
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
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Generates a unique temporary file path in the same directory as `final_path`.
///
/// The name includes the PID and a nanosecond timestamp to prevent conflicts
/// between concurrent writers.
pub fn unique_temp_path(final_path: &Path) -> PathBuf {
    let parent = final_path.parent().unwrap_or_else(|| Path::new("."));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let stem = final_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("auto-sync-pending");
    let ext = final_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("toml");
    parent.join(format!(".{stem}-{}-{nanos}.{ext}.tmp", std::process::id()))
}

/// Atomically writes bytes to `path` via a unique temporary file in the same
/// directory, then renames over the target. Returns the temp path for
/// diagnostics.
pub fn atomic_write_unique(final_path: &Path, bytes: &[u8]) -> Result<PathBuf, std::io::Error> {
    if let Some(parent) = final_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = unique_temp_path(final_path);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, final_path)?;
    Ok(tmp)
}

/// Attempts to fsync the parent directory. Best-effort on platforms that
/// support it; no-op on others.
pub fn fsync_parent_dir(path: &Path) {
    if let Some(parent) = path.parent() {
        #[cfg(unix)]
        {
            if let Ok(f) = OpenOptions::new().read(true).open(parent) {
                let _ = f.sync_all();
            }
        }
        #[cfg(not(unix))]
        {
            let _ = parent;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_acquire_and_drop_releases() {
        let dir = TempDir::new().unwrap();
        let path = pending_txn_lock_path(dir.path());
        {
            let _guard = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
            assert!(path.exists());
        }
        assert!(!path.exists());
    }

    #[test]
    fn test_concurrent_acquire_fails() {
        let dir = TempDir::new().unwrap();
        let _guard1 = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        let result = acquire_pending_txn(dir.path(), Duration::from_millis(50));
        assert!(matches!(result, Err(PendingTxnLockError::Busy { .. })));
    }

    #[test]
    fn test_dead_owner_reclaim() {
        let dir = TempDir::new().unwrap();
        let path = pending_txn_lock_path(dir.path());
        let contents = PendingTxnLockContents {
            pid: 1,
            nonce: "dead-owner".to_string(),
            created_at_unix_ms: unix_now_ms(),
        };
        let serialized = toml::to_string_pretty(&contents).unwrap();
        std::fs::write(&path, serialized).unwrap();

        let guard = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        assert_ne!(guard.nonce, "dead-owner");
    }

    #[test]
    fn test_ownership_checked_drop() {
        let dir = TempDir::new().unwrap();
        let path = pending_txn_lock_path(dir.path());
        let guard1 = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        let nonce1 = guard1.nonce().to_string();
        drop(guard1);
        assert!(!path.exists());

        let guard2 = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        let nonce2 = guard2.nonce().to_string();
        assert_ne!(nonce1, nonce2);
    }

    #[test]
    fn test_lock_permissions() {
        let dir = TempDir::new().unwrap();
        let _guard = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(pending_txn_lock_path(dir.path())).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_no_secrets_in_lock_file() {
        let dir = TempDir::new().unwrap();
        let _guard = acquire_pending_txn(dir.path(), Duration::from_millis(100)).unwrap();
        let raw = std::fs::read_to_string(pending_txn_lock_path(dir.path())).unwrap();
        for forbidden in &[
            "command",
            "description",
            "password",
            "secret",
            "api_key",
            "token",
            "credential",
        ] {
            assert!(
                !raw.to_lowercase().contains(forbidden),
                "pending txn lock must not contain {forbidden}"
            );
        }
    }

    #[test]
    fn test_unique_temp_path_is_distinct() {
        let dir = TempDir::new().unwrap();
        let final_path = dir.path().join("auto-sync-pending.toml");
        let a = unique_temp_path(&final_path);
        let b = unique_temp_path(&final_path);
        assert_ne!(a, b);
        assert!(a.starts_with(dir.path()));
    }

    #[test]
    fn test_atomic_write_unique_creates_file() {
        let dir = TempDir::new().unwrap();
        let final_path = dir.path().join("test.toml");
        let tmp = atomic_write_unique(&final_path, b"hello").unwrap();
        assert!(final_path.exists());
        assert_eq!(std::fs::read_to_string(&final_path).unwrap(), "hello");
        assert!(!tmp.exists());
    }

    #[test]
    fn test_lock_contents_roundtrip() {
        let contents = PendingTxnLockContents {
            pid: 999,
            nonce: "test-nonce".to_string(),
            created_at_unix_ms: 12345,
        };
        let serialized = toml::to_string_pretty(&contents).unwrap();
        let deserialized: PendingTxnLockContents = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.pid, 999);
        assert_eq!(deserialized.nonce, "test-nonce");
    }
}
