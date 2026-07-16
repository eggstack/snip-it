//! One-shot worker entry point and execution helpers.
//!
//! The worker is fully detached from the parent. The parent process never
//! acquires the worker execution lock — that is the worker's job. Each
//! spawned worker races for the lock; the winner performs the debounce,
//! sync, and conditional clear loop. The parent only:
//!
//! 1. Calls `record_pending_mutation` exactly once after a local commit.
//! 2. Calls `schedule_existing_pending` to spawn a detached worker that
//!    arbitrates the lock.
//!
//! The worker performs a bounded quiet-period debounce across separate CLI
//! processes. Newer generations arriving during sync are handled through a
//! bounded follow-up cycle, not by losing work.

use crate::auto_sync::lock::{self, LockError, WorkerLock};
use crate::auto_sync::pending::{self, PendingSnapshot, PendingState};
use crate::auto_sync::policy::AutoSyncPolicy;
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

/// Spawns a detached worker to process any existing pending work.
///
/// **Never mutates pending state, never increments generation, never
/// rewrites the marker.** Acquiring the worker lock is the child's job —
/// every spawned worker races for the lock and exactly one wins.
pub fn schedule_existing_pending(state_dir: &Path) -> SpawnResult {
    match spawn::spawn_worker(state_dir) {
        Ok(_) => SpawnResult::Spawned,
        Err(_) => SpawnResult::SpawnFailed,
    }
}

/// Worker entry point invoked by the detached child.
///
/// Acquires the worker lock; exits with `NothingToDo` if another worker
/// holds it. On success, performs a bounded debounce/sync loop:
///
/// 1. Read current pending state (the *observed* generation/timestamp).
/// 2. Sleep until the quiet-period deadline is reached (clamped to a
///    maximum worker lifetime).
/// 3. Reload the marker. If generation or timestamp changed, recompute
///    the deadline.
/// 4. Once the deadline is reached, run sync bounded by `sync_timeout`.
/// 5. On success, conditionally clear only if the marker still matches
///    the observed generation.
/// 6. Reload. If a newer generation exists, run another bounded cycle.
/// 7. Release the lock and exit.
///
/// The worker never owns a pending marker generation. It observes it,
/// acts on it, and lets a newer generation decide what to do.
pub fn run(state_dir: &Path) -> WorkerOutcome {
    let policy = AutoSyncPolicy::resolve(&get_sync_settings());

    let lock = match lock::try_acquire(state_dir) {
        Ok(l) => l,
        Err(LockError::AlreadyHeld { .. }) => {
            tracing::info!("auto-sync worker exiting: lock already held");
            return WorkerOutcome::NothingToDo;
        }
        Err(e) => {
            tracing::error!(error = %e, "auto-sync worker failed to acquire lock");
            return WorkerOutcome::Failed;
        }
    };

    run_locked(state_dir, lock, &policy)
}

fn run_locked(state_dir: &Path, lock: WorkerLock, policy: &AutoSyncPolicy) -> WorkerOutcome {
    let _lock_keepalive = lock;
    let start = std::time::Instant::now();
    let max_lifetime = Duration::from_secs(policy::WORKER_MAX_LIFETIME_SECS);

    loop {
        if start.elapsed() >= max_lifetime {
            tracing::warn!(
                elapsed_secs = start.elapsed().as_secs(),
                "auto-sync worker exiting: maximum lifetime reached"
            );
            return WorkerOutcome::NothingToDo;
        }

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

        let observed_generation = pending.generation;
        let observed_timestamp = pending.created_at_unix_ms;
        let observed_snapshot = pending.snapshot.clone();

        tracing::info!(
            generation = observed_generation,
            "auto-sync worker starting cycle"
        );

        if let Some(deadline) =
            compute_deadline(observed_timestamp, policy.debounce, start, max_lifetime)
            && let Err(e) = wait_for_quiet(
                state_dir,
                observed_generation,
                deadline,
                start,
                max_lifetime,
            )
        {
            tracing::warn!(error = %e, "auto-sync worker quiet-period wait failed");
        }

        let outcome = execute_sync(state_dir, observed_generation, &observed_snapshot, policy);

        match outcome {
            WorkerOutcome::Success => {
                let _ = pending::clear_if_generation_matches(state_dir, observed_generation);
                tracing::info!(
                    generation = observed_generation,
                    "auto-sync worker cycle completed"
                );
            }
            WorkerOutcome::Failed => {
                let _ = pending::record_failure(state_dir, observed_generation, "unknown");
            }
            WorkerOutcome::NothingToDo => {
                let _ = pending::clear_if_generation_matches(state_dir, observed_generation);
            }
        }

        match pending::read_state_from_dir(state_dir) {
            Ok(current) if current.generation > observed_generation => {
                tracing::info!(
                    previous = observed_generation,
                    current = current.generation,
                    "auto-sync worker: newer generation detected, starting follow-up cycle"
                );
                continue;
            }
            _ => return outcome,
        }
    }
}

/// Computes the wall-clock instant at which the worker should begin sync
/// for the observed generation.
///
/// Returns `None` if there is no need to wait (debounce is zero or
/// already exceeded by the worker's lifetime budget).
fn compute_deadline(
    observed_timestamp_ms: u64,
    debounce: Duration,
    start: std::time::Instant,
    max_lifetime: Duration,
) -> Option<std::time::Instant> {
    let now = std::time::Instant::now();
    if debounce.is_zero() {
        return None;
    }
    let target_unix_ms = observed_timestamp_ms.saturating_add(debounce.as_millis() as u64);
    let target = unix_ms_to_instant(target_unix_ms);
    let max_target = start
        .checked_add(max_lifetime)
        .unwrap_or_else(|| now.checked_add(max_lifetime).unwrap_or(now));
    Some(target.min(max_target))
}

/// Sleeps until the quiet-period deadline, reloading the marker on each
/// wakeup. If the generation or timestamp changes, the deadline is
/// recomputed. Returns an error string only on internal failures; the
/// caller decides how to react.
fn wait_for_quiet(
    state_dir: &Path,
    observed_generation: u64,
    initial_deadline: std::time::Instant,
    start: std::time::Instant,
    max_lifetime: Duration,
) -> Result<(), String> {
    let mut deadline = initial_deadline;
    let max_target = start.checked_add(max_lifetime).unwrap_or(deadline);

    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return Ok(());
        }
        if now >= max_target {
            return Ok(());
        }

        let sleep_for = deadline.saturating_duration_since(now);
        std::thread::sleep(sleep_for.min(Duration::from_millis(250)));

        match pending::read_state_from_dir(state_dir) {
            Ok(current) => {
                if current.generation > observed_generation {
                    let policy = AutoSyncPolicy::resolve(&get_sync_settings());
                    let next = compute_deadline(
                        current.created_at_unix_ms,
                        policy.debounce,
                        start,
                        max_lifetime,
                    );
                    if let Some(next_deadline) = next {
                        deadline = next_deadline.min(max_target);
                    } else {
                        return Ok(());
                    }
                }
            }
            Err(pending::PendingError::NotFound) => return Ok(()),
            Err(e) => return Err(format!("{e}")),
        }
    }
}

fn unix_ms_to_instant(target_unix_ms: u64) -> std::time::Instant {
    let now_unix_ms = unix_now_ms();
    if target_unix_ms <= now_unix_ms {
        return std::time::Instant::now();
    }
    let delta = Duration::from_millis(target_unix_ms - now_unix_ms);
    std::time::Instant::now()
        .checked_add(delta)
        .unwrap_or_else(std::time::Instant::now)
}

/// Performs the bounded sync attempt for the observed generation.
///
/// Refuses to clear pending on success; callers handle that
/// generation-safely. Returns `NothingToDo` only when the policy is
/// disabled.
fn execute_sync(
    _state_dir: &Path,
    _observed_generation: u64,
    _observed_snapshot: &PendingSnapshot,
    policy: &AutoSyncPolicy,
) -> WorkerOutcome {
    if !policy.enabled {
        return WorkerOutcome::NothingToDo;
    }

    let timeout = policy.sync_timeout;

    let result = match run_async_with_timeout(timeout) {
        Ok(()) => Ok(()),
        Err(e) => Err(e),
    };

    match result {
        Ok(()) => WorkerOutcome::Success,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "auto-sync failed"
            );
            WorkerOutcome::Failed
        }
    }
}

/// Runs the default sync in a worker-owned Tokio runtime, bounded by
/// `timeout`. The future is dropped on timeout, cancelling the underlying
/// sync task; no detached thread survives worker completion.
fn run_async_with_timeout(timeout: Duration) -> Result<(), String> {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let rt = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build tokio runtime")
    });

    let _guard = rt.enter();
    let work = crate::sync_commands::run_default_sync_async();
    let timed = tokio::time::timeout(timeout, work);
    let joined = rt.block_on(timed);
    match joined {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("{e}")),
        Err(_elapsed) => Err("sync timeout".to_string()),
    }
}

/// Recovery path invoked by the parent at startup.
///
/// Loads the pending marker. If absent, nothing to do. If present, the
/// marker is preserved (old valid work is not silently discarded); the
/// caller may spawn a worker through `schedule_existing_pending`.
pub fn startup_recover(state_dir: &Path) -> Result<Option<PendingState>, pending::PendingError> {
    let pending_path = pending::pending_path(state_dir);
    if !pending_path.exists() {
        return Ok(None);
    }

    let current = pending::read_state_from_dir(state_dir)?;
    let now_ms = unix_now_ms();
    let age_ms = now_ms.saturating_sub(current.created_at_unix_ms);

    if age_ms > pending::STALE_PENDING_THRESHOLD_MS {
        tracing::warn!(
            generation = current.generation,
            age_ms,
            "startup recovery: stale pending marker; preserving for worker scheduling"
        );
        Ok(Some(current))
    } else {
        tracing::info!(
            generation = current.generation,
            "startup recovery: pending state present; scheduling worker"
        );
        let _ = schedule_existing_pending(state_dir);
        Ok(Some(current))
    }
}

/// Generation-aware explicit-sync clearing.
///
/// Captures the pending generation at sync start, then clears only if the
/// marker still matches after sync succeeds — preserving any mutation
/// that arrived during the explicit sync.
pub fn clear_after_explicit_sync(
    state_dir: &Path,
    observed_generation: u64,
    sync_succeeded: bool,
) -> Result<bool, pending::PendingError> {
    if !sync_succeeded {
        return Ok(false);
    }
    pending::clear_if_generation_matches(state_dir, observed_generation)
        .map(|result| matches!(result, pending::ConditionalClearResult::Cleared))
}

/// Reads the current pending generation, if any. Used by callers of
/// `clear_after_explicit_sync` to capture the observed generation
/// **before** the explicit sync runs.
pub fn observed_pending_generation(state_dir: &Path) -> Result<Option<u64>, pending::PendingError> {
    match pending::read_state_from_dir(state_dir) {
        Ok(s) => Ok(Some(s.generation)),
        Err(pending::PendingError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub mod policy {
    //! Re-exported constants for worker policy.
    pub use crate::auto_sync::policy::*;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_nothing_to_do_without_pending() {
        let dir = TempDir::new().unwrap();
        let outcome = run(dir.path());
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
    fn test_startup_recover_no_pending() {
        let dir = TempDir::new().unwrap();
        let result = startup_recover(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_startup_recover_preserves_pending_without_incrementing() {
        let dir = TempDir::new().unwrap();
        let initial = pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let before_bytes = std::fs::read_to_string(pending::pending_path(dir.path())).unwrap();
        let _ = startup_recover(dir.path()).unwrap();
        let after_bytes = std::fs::read_to_string(pending::pending_path(dir.path())).unwrap();
        assert_eq!(before_bytes, after_bytes);
        let current = pending::read_state_from_dir(dir.path()).unwrap();
        assert_eq!(current.generation, initial.generation);
        assert_eq!(
            current.snapshot,
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate
            }
        );
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
    fn test_clear_after_explicit_sync_skipped_on_failure() {
        let dir = TempDir::new().unwrap();
        let initial = pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let observed = observed_pending_generation(dir.path()).unwrap();
        let cleared = clear_after_explicit_sync(dir.path(), observed.unwrap(), false).unwrap();
        assert!(!cleared);
        let current = pending::read_state_from_dir(dir.path()).unwrap();
        assert_eq!(current.generation, initial.generation);
    }

    #[test]
    fn test_clear_after_explicit_sync_clears_matching_generation() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let observed = observed_pending_generation(dir.path()).unwrap().unwrap();
        let cleared = clear_after_explicit_sync(dir.path(), observed, true).unwrap();
        assert!(cleared);
        assert!(matches!(
            pending::read_state_from_dir(dir.path()),
            Err(pending::PendingError::NotFound)
        ));
    }

    #[test]
    fn test_clear_after_explicit_sync_preserves_newer_generation() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let observed = observed_pending_generation(dir.path()).unwrap().unwrap();
        // Simulate a mutation arriving during the explicit sync.
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetUpdate,
            },
        )
        .unwrap();
        let cleared = clear_after_explicit_sync(dir.path(), observed, true).unwrap();
        assert!(!cleared);
        let current = pending::read_state_from_dir(dir.path()).unwrap();
        assert!(current.generation > observed);
        assert_eq!(
            current.snapshot,
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetUpdate
            }
        );
    }
}
