//! Worker lock — re-exported from `execution_lock` for backward compatibility.

pub use crate::auto_sync::execution_lock::{
    WORKER_LOCK_NAME, WORKER_LOCK_PURPOSE, WorkerLock, WorkerLockError,
    try_acquire_worker as try_acquire, wait_acquire_worker as wait_acquire,
    worker_inspect as inspect, worker_is_stale as is_stale, worker_lock_path as lock_path,
};

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_acquire_and_drop_releases() {
        let dir = TempDir::new().unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        assert!(lock_path(dir.path()).exists());
        let nonce1 = lock.nonce().to_string();
        drop(lock);
        assert!(lock_path(dir.path()).exists());
        let lock2 = try_acquire(dir.path()).unwrap();
        assert_ne!(lock2.nonce(), nonce1);
    }

    #[test]
    fn test_double_acquire_fails() {
        let dir = TempDir::new().unwrap();
        let _first = try_acquire(dir.path()).unwrap();
        let result = try_acquire(dir.path());
        assert!(matches!(result, Err(WorkerLockError::AlreadyHeld { .. })));
    }

    #[test]
    fn test_lock_path_is_in_state_dir() {
        let dir = TempDir::new().unwrap();
        assert_eq!(lock_path(dir.path()), dir.path().join(WORKER_LOCK_NAME));
    }

    #[test]
    fn test_no_secrets_in_lock_file() {
        let dir = TempDir::new().unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        let id = lock
            .read_owner_via_handle()
            .expect("identity must be readable via handle");
        let serialized = toml::to_string_pretty(&id).unwrap();
        let raw_lower = serialized.to_lowercase();
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
            "credential",
        ] {
            assert!(
                !value_only.contains(forbidden),
                "lock file must not contain {forbidden} in a value"
            );
        }
    }

    #[test]
    fn test_inspect_returns_contents() {
        let dir = TempDir::new().unwrap();
        let lock = try_acquire(dir.path()).unwrap();
        let contents = lock.read_owner_via_handle().unwrap();
        assert_eq!(contents.pid, std::process::id());
        assert_eq!(contents.nonce, lock.nonce());
        assert_eq!(contents.purpose, WORKER_LOCK_PURPOSE);
    }

    #[test]
    fn test_lock_permissions() {
        let dir = TempDir::new().unwrap();
        let _lock = try_acquire(dir.path()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(lock_path(dir.path())).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_process_alive_zero_pid() {
        assert!(crate::auto_sync::execution_lock::process_alive(0));
    }

    #[test]
    fn test_process_alive_current_pid() {
        assert!(crate::auto_sync::execution_lock::process_alive(
            std::process::id()
        ));
    }
}
