//! One-shot worker entry point and execution helpers.

use crate::auto_sync::lock::{self, LockError};
use crate::auto_sync::pending::{self, PendingSnapshot, PendingState};
use crate::auto_sync::policy::{AutoSyncPolicy, FailureClass};
use crate::auto_sync::spawn;
use crate::config::get_sync_settings;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerOutcome {
    Success,
    Failed,
    NothingToDo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnResult {
    Spawned,
    Suppressed,
    SpawnFailed,
}

pub fn try_schedule(state_dir: &Path, _snapshot: PendingSnapshot) -> SpawnResult {
    match pending::mark_pending(state_dir, PendingSnapshot::default()) {
        Ok(marked) => match lock::try_acquire(state_dir) {
            Ok(worker_lock) => {
                let nonce = worker_lock.nonce().to_string();
                let path = worker_lock.path().to_path_buf();
                match spawn::spawn_worker(state_dir, &nonce) {
                    Ok(_) => {
                        std::mem::forget(worker_lock);
                        SpawnResult::Spawned
                    }
                    Err(_) => {
                        let _ = pending::clear_if_generation_matches(state_dir, marked.generation);
                        let _ = std::fs::remove_file(&path);
                        SpawnResult::SpawnFailed
                    }
                }
            }
            Err(LockError::AlreadyHeld { .. }) => SpawnResult::Suppressed,
            Err(_) => SpawnResult::SpawnFailed,
        },
        Err(_) => SpawnResult::SpawnFailed,
    }
}

pub fn run(state_dir: &Path, nonce: &str) -> WorkerOutcome {
    let policy = AutoSyncPolicy::resolve(&get_sync_settings());
    let lock = match lock::try_acquire(state_dir) {
        Ok(l) => l,
        Err(LockError::AlreadyHeld { pid, .. }) => {
            tracing::info!(pid, "auto-sync worker exiting: lock already held");
            return WorkerOutcome::NothingToDo;
        }
        Err(e) => {
            tracing::error!(error = %e, "auto-sync worker failed to acquire lock");
            return WorkerOutcome::Failed;
        }
    };

    if nonce_already_used(state_dir, nonce) {
        tracing::debug!(nonce, "auto-sync worker skipping: nonce already consumed");
        return WorkerOutcome::NothingToDo;
    }

    let _lock_keepalive = lock;
    mark_nonce_consumed(state_dir, nonce);

    let pending = match pending::read_state_from_dir(state_dir) {
        Ok(p) => p,
        Err(pending::PendingError::NotFound) => {
            tracing::debug!("auto-sync worker exiting: no pending state");
            return WorkerOutcome::NothingToDo;
        }
        Err(e) => {
            tracing::error!(error = %e, "auto-sync worker failed to read pending state");
            return WorkerOutcome::Failed;
        }
    };

    tracing::info!(generation = pending.generation, "auto-sync worker starting");

    let outcome = execute_sync(state_dir, &pending, &policy);

    match outcome {
        WorkerOutcome::Success => {
            let _ = pending::record_success(state_dir, pending.generation);
            tracing::info!(
                generation = pending.generation,
                "auto-sync worker completed"
            );
        }
        WorkerOutcome::Failed => {
            let _ = pending::record_failure(state_dir, pending.generation, "unknown");
        }
        WorkerOutcome::NothingToDo => {
            let _ = pending::clear_if_generation_matches(state_dir, pending.generation);
        }
    }

    outcome
}

pub fn startup_recover(state_dir: &Path) -> Result<(), pending::PendingError> {
    let pending_path = pending::pending_path(state_dir);
    if !pending_path.exists() {
        return Ok(());
    }

    let current = pending::read_state_from_dir(state_dir).map_err(|e| {
        tracing::warn!(error = %e, "startup recovery: failed to read pending marker");
        e
    })?;

    let now_ms = unix_now_ms();
    let age_ms = now_ms.saturating_sub(current.created_at_unix_ms);
    if age_ms > pending::STALE_PENDING_THRESHOLD_MS {
        tracing::warn!(
            generation = current.generation,
            age_ms,
            "startup recovery: clearing stale pending marker"
        );
        let _ = pending::clear(state_dir);
    } else {
        tracing::info!(
            generation = current.generation,
            "startup recovery: pending state still active, re-scheduling worker"
        );
        let _ = try_schedule(state_dir, PendingSnapshot::None);
    }

    Ok(())
}

pub fn clear_after_explicit_sync(state_dir: &Path) -> Result<(), pending::PendingError> {
    pending::clear_for_explicit_sync(state_dir)
}

pub fn execute_sync(
    state_dir: &Path,
    pending_state: &PendingState,
    policy: &AutoSyncPolicy,
) -> WorkerOutcome {
    if !policy.enabled {
        let _ = pending::clear_if_generation_matches(state_dir, pending_state.generation);
        return WorkerOutcome::NothingToDo;
    }

    let timeout = policy.sync_timeout;

    let result = match run_with_timeout(run_default_sync_blocking, timeout) {
        Ok(()) => Ok(()),
        Err(e) => Err(e),
    };

    match result {
        Ok(()) => WorkerOutcome::Success,
        Err(e) => {
            let classification = FailureClass::from_code(classify_failure(&e));
            tracing::warn!(
                error = %e,
                classification = classification.as_code(),
                "auto-sync failed"
            );
            WorkerOutcome::Failed
        }
    }
}

fn run_default_sync_blocking() -> Result<(), String> {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let rt = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime")
    });

    crate::sync_commands::run_default_sync(rt).map_err(|e| format!("{e}"))
}

fn classify_failure(err: &str) -> &'static str {
    let lower = err.to_lowercase();
    if lower.contains("network")
        || lower.contains("timeout")
        || lower.contains("dns")
        || lower.contains("connection refused")
        || lower.contains("connect")
        || lower.contains("unavailable")
    {
        "network"
    } else if lower.contains("auth")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("api key")
        || lower.contains("permission denied")
    {
        "auth"
    } else if lower.contains("conflict") || lower.contains("merge") {
        "conflict"
    } else {
        "unknown"
    }
}

fn run_with_timeout<F>(work: F, timeout: Duration) -> Result<(), String>
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    use std::sync::mpsc;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(timeout)
        .map_err(|_| "sync timeout".to_string())?
}

fn nonce_already_used(state_dir: &Path, nonce: &str) -> bool {
    let consumed_path = state_dir.join(format!("auto-sync-worker.{nonce}.done"));
    consumed_path.exists()
}

fn mark_nonce_consumed(state_dir: &Path, nonce: &str) {
    let consumed_path = state_dir.join(format!("auto-sync-worker.{nonce}.done"));
    let _ = std::fs::write(&consumed_path, b"");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&consumed_path) {
            let mut perms = meta.permissions();
            perms.set_mode(0o600);
            let _ = std::fs::set_permissions(&consumed_path, perms);
        }
    }
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_nothing_to_do_without_pending() {
        let dir = TempDir::new().unwrap();
        let outcome = run(dir.path(), "test-nonce");
        assert_eq!(outcome, WorkerOutcome::NothingToDo);
    }

    #[test]
    fn test_worker_outcome_equality() {
        assert_eq!(WorkerOutcome::Success, WorkerOutcome::Success);
        assert_eq!(WorkerOutcome::Failed, WorkerOutcome::Failed);
        assert_eq!(WorkerOutcome::NothingToDo, WorkerOutcome::NothingToDo);
        assert_ne!(WorkerOutcome::Success, WorkerOutcome::Failed);
    }

    #[test]
    fn test_worker_outcome_debug() {
        assert_eq!(format!("{:?}", WorkerOutcome::Success), "Success");
        assert_eq!(format!("{:?}", WorkerOutcome::Failed), "Failed");
        assert_eq!(format!("{:?}", WorkerOutcome::NothingToDo), "NothingToDo");
    }

    #[test]
    fn test_classify_failure() {
        assert_eq!(classify_failure("connection refused"), "network");
        assert_eq!(classify_failure("unauthorized"), "auth");
        assert_eq!(classify_failure("merge conflict"), "conflict");
        assert_eq!(classify_failure("unknown"), "unknown");
    }

    #[test]
    fn test_run_with_timeout_success() {
        let result = run_with_timeout(|| Ok(()), Duration::from_secs(1));
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_with_timeout_failure() {
        let result = run_with_timeout(|| Err("boom".to_string()), Duration::from_secs(1));
        assert!(result.is_err());
    }

    #[test]
    fn test_run_with_timeout_exceeds() {
        let result = run_with_timeout(
            || {
                std::thread::sleep(Duration::from_secs(2));
                Ok(())
            },
            Duration::from_millis(50),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_clear_after_explicit_sync() {
        let dir = TempDir::new().unwrap();
        let _lock = pending::mark_pending(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        clear_after_explicit_sync(dir.path()).unwrap();
        assert!(matches!(
            pending::read_state_from_dir(dir.path()),
            Err(pending::PendingError::NotFound)
        ));
    }

    #[test]
    fn test_nonce_already_used() {
        let dir = TempDir::new().unwrap();
        let consumed = dir.path().join("auto-sync-worker.abc.done");
        std::fs::write(&consumed, "").unwrap();
        assert!(nonce_already_used(dir.path(), "abc"));
        assert!(!nonce_already_used(dir.path(), "xyz"));
    }

    #[test]
    fn test_startup_recover_no_pending() {
        let dir = TempDir::new().unwrap();
        assert!(startup_recover(dir.path()).is_ok());
    }

    #[test]
    fn test_execute_sync_disabled_policy_returns_nothing_to_do() {
        let dir = TempDir::new().unwrap();
        let state = pending::PendingState {
            generation: 1,
            snapshot: PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
            created_at_unix_ms: 0,
        };
        let policy = AutoSyncPolicy::default();
        let outcome = execute_sync(dir.path(), &state, &policy);
        assert_eq!(outcome, WorkerOutcome::NothingToDo);
    }

    #[test]
    fn test_spawn_result_equality() {
        assert_eq!(SpawnResult::Spawned, SpawnResult::Spawned);
        assert_eq!(SpawnResult::Suppressed, SpawnResult::Suppressed);
        assert_eq!(SpawnResult::SpawnFailed, SpawnResult::SpawnFailed);
        assert_ne!(SpawnResult::Spawned, SpawnResult::Suppressed);
    }

    #[test]
    fn test_spawn_result_debug() {
        assert_eq!(format!("{:?}", SpawnResult::Spawned), "Spawned");
        assert_eq!(format!("{:?}", SpawnResult::Suppressed), "Suppressed");
        assert_eq!(format!("{:?}", SpawnResult::SpawnFailed), "SpawnFailed");
    }

    #[test]
    fn test_run_skips_duplicate_nonce() {
        let dir = TempDir::new().unwrap();
        let _lock = pending::mark_pending(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        mark_nonce_consumed(dir.path(), "duplicate-nonce");
        let outcome = run(dir.path(), "duplicate-nonce");
        assert_eq!(outcome, WorkerOutcome::NothingToDo);
    }
}
