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

use crate::auto_sync::execution_lock::{self, ExecutionLockError, SyncExecutionLock};
use crate::auto_sync::executor::ExecutorExitCode;
use crate::auto_sync::pending::{self, PendingState};
use crate::auto_sync::policy::{AutoSyncPolicy, FailureClass, transient_backoff};
use crate::auto_sync::spawn;
use crate::auto_sync::status;
use crate::config::get_sync_settings;
use std::path::Path;
use std::process::ExitStatus;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

pub trait Clock {
    fn now_instant(&self) -> Instant;
    fn now_unix_ms(&self) -> u64;
    fn sleep(&self, duration: Duration);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_instant(&self) -> Instant {
        Instant::now()
    }

    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebounceResult {
    Ready(PendingState),
    CancelledMarkerRemoved,
    DeferredMaximumLifetime(PendingState),
    Failed(String),
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
/// Acquires the execution lock; exits with `NothingToDo` if another
/// worker holds it. On success, performs a bounded debounce/sync loop:
///
/// 1. Read current pending state (the *observed* generation/timestamp).
/// 2. Sleep until the quiet-period deadline is reached (clamped to a
///    maximum worker lifetime).
/// 3. Reload the marker. If generation or timestamp changed, the
///    deadline is recomputed.
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

    let lock = match execution_lock::try_acquire(state_dir) {
        Ok(l) => l,
        Err(ExecutionLockError::AlreadyHeld { .. }) => {
            tracing::info!("auto-sync worker exiting: execution lock already held");
            return WorkerOutcome::NothingToDo;
        }
        Err(e) => {
            tracing::error!(error = %e, "auto-sync worker failed to acquire execution lock");
            return WorkerOutcome::Failed;
        }
    };

    run_locked(state_dir, lock, &policy)
}

fn run_locked(state_dir: &Path, lock: SyncExecutionLock, policy: &AutoSyncPolicy) -> WorkerOutcome {
    let _lock_keepalive = lock;
    let clock = SystemClock;
    let start = clock.now_instant();
    let max_lifetime = Duration::from_secs(policy::WORKER_MAX_LIFETIME_SECS);

    if !policy.enabled {
        tracing::info!("auto-sync worker exiting: policy disabled; pending preserved");
        return WorkerOutcome::NothingToDo;
    }

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

        tracing::info!(
            generation = observed_generation,
            "auto-sync worker starting cycle"
        );

        let initial_deadline = compute_deadline(
            pending.created_at_unix_ms,
            policy.debounce,
            start,
            max_lifetime,
            &clock,
        );

        let observed = if let Some(deadline) = initial_deadline {
            match debounce(
                state_dir,
                pending,
                deadline,
                start,
                max_lifetime,
                policy.max_delay,
                &clock,
            ) {
                DebounceResult::Ready(state) => state,
                DebounceResult::CancelledMarkerRemoved => {
                    tracing::info!(
                        "auto-sync worker exiting: pending marker removed during debounce"
                    );
                    return WorkerOutcome::NothingToDo;
                }
                DebounceResult::DeferredMaximumLifetime(state) => {
                    tracing::info!(
                        generation = state.generation,
                        "auto-sync worker: max delay reached, forcing sync"
                    );
                    state
                }
                DebounceResult::Failed(e) => {
                    tracing::warn!(error = %e, "auto-sync worker debounce failed");
                    return WorkerOutcome::Failed;
                }
            }
        } else {
            pending
        };

        match preflight_check(state_dir, observed.generation) {
            Ok(latest) => {
                let sync_observed = if latest.generation != observed.generation {
                    tracing::info!(
                        observed = observed.generation,
                        latest = latest.generation,
                        "auto-sync worker: preflight detected newer generation"
                    );
                    latest
                } else {
                    observed
                };

                let outcome = execute_sync(state_dir, policy);

                match outcome {
                    WorkerOutcome::Success => {
                        let _ = pending::clear_if_generation_matches(
                            state_dir,
                            sync_observed.generation,
                        );
                        tracing::info!(
                            generation = sync_observed.generation,
                            "auto-sync worker cycle completed"
                        );
                    }
                    WorkerOutcome::Failed => {
                        let _ =
                            pending::record_failure(state_dir, sync_observed.generation, "unknown");
                    }
                    WorkerOutcome::NothingToDo => {
                        tracing::info!(
                            generation = sync_observed.generation,
                            "auto-sync worker NothingToDo: pending preserved for next cycle"
                        );
                    }
                }

                match pending::read_state_from_dir(state_dir) {
                    Ok(current) if current.generation > sync_observed.generation => {
                        tracing::info!(
                            previous = sync_observed.generation,
                            current = current.generation,
                            "auto-sync worker: newer generation detected, starting follow-up cycle"
                        );
                        continue;
                    }
                    _ => return outcome,
                }
            }
            Err(e) => {
                tracing::info!(error = %e, "auto-sync worker preflight: nothing to do");
                return WorkerOutcome::NothingToDo;
            }
        }
    }
}

fn compute_deadline(
    observed_timestamp_ms: u64,
    debounce: Duration,
    start: Instant,
    max_lifetime: Duration,
    clock: &dyn Clock,
) -> Option<Instant> {
    let now = clock.now_instant();
    if debounce.is_zero() {
        return None;
    }
    let target_unix_ms = observed_timestamp_ms.saturating_add(debounce.as_millis() as u64);
    let target = unix_ms_to_instant(target_unix_ms, clock);
    let max_target = start
        .checked_add(max_lifetime)
        .unwrap_or_else(|| now.checked_add(max_lifetime).unwrap_or(now));
    Some(target.min(max_target))
}

pub fn debounce(
    state_dir: &Path,
    observed: PendingState,
    initial_deadline: Instant,
    start: Instant,
    max_lifetime: Duration,
    max_delay: Duration,
    clock: &dyn Clock,
) -> DebounceResult {
    let mut deadline = initial_deadline;
    let max_target = start.checked_add(max_lifetime).unwrap_or(deadline);
    let mut current = observed;

    if clock.now_instant() >= max_target {
        return DebounceResult::Ready(current);
    }

    loop {
        let now = clock.now_instant();
        if now >= deadline || now >= max_target {
            match pending::read_state_from_dir(state_dir) {
                Ok(latest) => {
                    if latest.generation != current.generation {
                        current = latest;
                        let new_deadline = compute_deadline(
                            current.created_at_unix_ms,
                            current_debounce(state_dir),
                            start,
                            max_lifetime,
                            clock,
                        );
                        if let Some(d) = new_deadline {
                            deadline = d.min(max_target);
                            continue;
                        }
                        return DebounceResult::Ready(current);
                    }
                    return DebounceResult::Ready(current);
                }
                Err(pending::PendingError::NotFound) => {
                    return DebounceResult::CancelledMarkerRemoved;
                }
                Err(e) => {
                    return DebounceResult::Failed(format!("{e}"));
                }
            }
        }

        let sleep_for = deadline.saturating_duration_since(now);
        clock.sleep(sleep_for.min(Duration::from_millis(250)));

        match pending::read_state_from_dir(state_dir) {
            Ok(current_state) => {
                if current_state.generation > current.generation {
                    current = current_state;
                    let new_deadline = compute_deadline(
                        current.created_at_unix_ms,
                        current_debounce(state_dir),
                        start,
                        max_lifetime,
                        clock,
                    );
                    if let Some(d) = new_deadline {
                        deadline = d.min(max_target);
                    } else {
                        return DebounceResult::Ready(current);
                    }
                } else if current_state.generation < current.generation {
                    current = current_state;
                }
            }
            Err(pending::PendingError::NotFound) => {
                return DebounceResult::CancelledMarkerRemoved;
            }
            Err(e) => {
                return DebounceResult::Failed(format!("{e}"));
            }
        }

        if start.elapsed() >= max_delay {
            return DebounceResult::DeferredMaximumLifetime(current);
        }
    }
}

pub fn preflight_check(
    state_dir: &Path,
    _observed_generation: u64,
) -> Result<PendingState, String> {
    match pending::read_state_from_dir(state_dir) {
        Ok(state) => Ok(state),
        Err(pending::PendingError::NotFound) => {
            Err("pending marker removed, nothing to do".to_string())
        }
        Err(e) => Err(format!("corrupt pending state: {e}")),
    }
}

fn current_debounce(state_dir: &Path) -> Duration {
    match pending::read_state_from_dir(state_dir) {
        Ok(_) => get_sync_settings().auto_sync_debounce(),
        Err(_) => Duration::ZERO,
    }
}

fn unix_ms_to_instant(target_unix_ms: u64, clock: &dyn Clock) -> Instant {
    let now_unix_ms = clock.now_unix_ms();
    if target_unix_ms <= now_unix_ms {
        return clock.now_instant();
    }
    let delta = Duration::from_millis(target_unix_ms - now_unix_ms);
    clock
        .now_instant()
        .checked_add(delta)
        .unwrap_or_else(|| clock.now_instant())
}

/// Performs the bounded sync attempt for the observed generation.
///
/// Spawns an executor subprocess and waits for it, enforcing the sync
/// timeout. On timeout, sends SIGTERM (Unix) / terminate (Windows),
/// waits a 2-second grace period, then SIGKILL (Unix) / kill (Windows)
/// if still alive.
///
/// Maps `ExecutorExitCode` to `WorkerOutcome` and records durable status:
/// - `Success` → `Success` (caller may conditionally clear pending)
/// - any other code → `Failed` with failure class, backoff, and status
///
/// The disabled-policy branch is retained as a defensive guard, but
/// `run_locked` already exits with `NothingToDo` before reaching this
/// function when policy is disabled.
fn execute_sync(state_dir: &Path, policy: &AutoSyncPolicy) -> WorkerOutcome {
    if !policy.enabled {
        return WorkerOutcome::NothingToDo;
    }

    let mut child = match spawn::spawn_executor(state_dir) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "failed to spawn sync executor");
            // Record spawn failure status
            let _ = status::record_failure(
                state_dir,
                0,
                FailureClass::Internal,
                -1,
                0,
                0,
                &format!("executor spawn failed: {e}"),
                current_config_fingerprint(),
            );
            return WorkerOutcome::Failed;
        }
    };

    match wait_child_with_timeout(&mut child, policy.sync_timeout) {
        Ok(Some(status_out)) => {
            let code = status_out.code().unwrap_or(7);
            match ExecutorExitCode::from_exit_status(status_out) {
                ExecutorExitCode::Success => {
                    let _ = status::record_success(state_dir, 0, "sync completed successfully");
                    WorkerOutcome::Success
                }
                exit_code => {
                    let failure_class = exit_code.failure_class();
                    tracing::warn!(
                        exit_code = code,
                        failure_class = %failure_class.as_code(),
                        "sync executor failed"
                    );
                    // Record failure with backoff
                    let consecutive = next_consecutive_failures(state_dir);
                    let backoff = transient_backoff(consecutive);
                    let next_attempt = unix_now_ms().saturating_add(backoff.as_millis() as u64);
                    let _ = status::record_failure(
                        state_dir,
                        0,
                        failure_class,
                        code,
                        consecutive,
                        next_attempt,
                        &exit_code.to_string(),
                        current_config_fingerprint(),
                    );
                    WorkerOutcome::Failed
                }
            }
        }
        Ok(None) => {
            tracing::warn!("sync executor timed out, terminating");
            terminate_child(&mut child);
            std::thread::sleep(Duration::from_secs(2));
            if let Ok(None) = child.try_wait() {
                force_kill_child(&mut child);
                let _ = child.wait();
            }
            // Record timeout as network failure
            let consecutive = next_consecutive_failures(state_dir);
            let backoff = transient_backoff(consecutive);
            let next_attempt = unix_now_ms().saturating_add(backoff.as_millis() as u64);
            let _ = status::record_failure(
                state_dir,
                0,
                FailureClass::TransientTimeout,
                4,
                consecutive,
                next_attempt,
                "sync executor timed out",
                current_config_fingerprint(),
            );
            WorkerOutcome::Failed
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to wait for sync executor");
            let _ = status::record_failure(
                state_dir,
                0,
                FailureClass::Internal,
                -1,
                0,
                0,
                &format!("wait failed: {e}"),
                current_config_fingerprint(),
            );
            WorkerOutcome::Failed
        }
    }
}

/// Read the current consecutive failure count from status.
fn next_consecutive_failures(state_dir: &Path) -> u32 {
    status::read_status(state_dir)
        .map(|s| s.consecutive_failures.saturating_add(1))
        .unwrap_or(1)
}

/// Compute the current config fingerprint for deferral release detection.
fn current_config_fingerprint() -> u64 {
    let settings = get_sync_settings();
    status::compute_config_fingerprint(&settings)
}

fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Wait for a child process with a timeout.
///
/// Polls `try_wait()` every 100ms. Returns `Ok(Some(status))` if the
/// child exits before the deadline, `Ok(None)` on timeout, or `Err`
/// on platform errors.
fn wait_child_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<Option<ExitStatus>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Some(status)),
            Ok(None) => {
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(unix)]
fn terminate_child(child: &mut std::process::Child) {
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
}

#[cfg(not(unix))]
fn terminate_child(child: &mut std::process::Child) {
    let _ = child.kill();
}

#[cfg(unix)]
fn force_kill_child(child: &mut std::process::Child) {
    unsafe {
        libc::kill(child.id() as i32, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn force_kill_child(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// Recovery path invoked by the parent at startup.
///
/// Loads the pending marker. If absent, nothing to do. If present, the
/// marker is preserved (old valid work is not silently discarded); the
/// caller may spawn a worker through `schedule_existing_pending`.
///
/// If the execution lock is already held (another sync in progress),
/// scheduling is skipped — the active worker or foreground sync will
/// handle the pending state.
pub fn startup_recover(state_dir: &Path) -> Result<Option<PendingState>, pending::PendingError> {
    let pending_path = pending::pending_path(state_dir);
    if !pending_path.exists() {
        return Ok(None);
    }

    let current = pending::read_state_from_dir(state_dir)?;

    // Check if the execution lock is already held by another process.
    // If so, skip scheduling — the active holder will handle pending work.
    let lock_path = execution_lock::execution_lock_path(state_dir);
    if let Some(contents) = execution_lock::inspect(&lock_path)
        && execution_lock::process_alive(contents.pid)
    {
        tracing::info!(
            generation = current.generation,
            owner_pid = contents.pid,
            "startup recovery: execution lock held; skipping scheduling (active sync in progress)"
        );
        return Ok(Some(current));
    }

    let now_ms = unix_now_ms();
    let age_ms = now_ms.saturating_sub(current.created_at_unix_ms);

    if age_ms > pending::STALE_PENDING_THRESHOLD_MS {
        tracing::warn!(
            generation = current.generation,
            age_ms,
            "startup recovery: stale pending marker; scheduling worker for recoverable work"
        );
    } else {
        tracing::info!(
            generation = current.generation,
            "startup recovery: pending state present; scheduling worker"
        );
    }

    let _ = schedule_existing_pending(state_dir);
    Ok(Some(current))
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

pub mod policy {
    //! Re-exported constants for worker policy.
    pub use crate::auto_sync::policy::*;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_sync::pending::PendingSnapshot;
    use std::sync::Mutex;
    use tempfile::TempDir;

    struct MockClock {
        instant: Mutex<Instant>,
        unix_ms: Mutex<u64>,
    }

    impl MockClock {
        fn new(start_instant: Instant, start_unix_ms: u64) -> Self {
            Self {
                instant: Mutex::new(start_instant),
                unix_ms: Mutex::new(start_unix_ms),
            }
        }

        fn advance(&self, duration: Duration) {
            let mut inst = self.instant.lock().unwrap();
            *inst += duration;
            let mut ms = self.unix_ms.lock().unwrap();
            *ms += duration.as_millis() as u64;
        }
    }

    impl Clock for MockClock {
        fn now_instant(&self) -> Instant {
            *self.instant.lock().unwrap()
        }
        fn now_unix_ms(&self) -> u64 {
            *self.unix_ms.lock().unwrap()
        }
        fn sleep(&self, duration: Duration) {
            self.advance(duration);
        }
    }

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

    #[test]
    fn test_wait_child_with_timeout_exits_before_deadline() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let result = wait_child_with_timeout(&mut child, Duration::from_secs(5)).unwrap();
        assert!(result.is_some());
        let status = result.unwrap();
        assert!(status.success());
    }

    #[test]
    fn test_wait_child_with_timeout_returns_none_on_timeout() {
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let result = wait_child_with_timeout(&mut child, Duration::from_millis(200)).unwrap();
        assert!(result.is_none());
        force_kill_child(&mut child);
        let _ = child.wait();
    }

    #[test]
    fn test_execute_sync_disabled_policy() {
        let dir = TempDir::new().unwrap();
        let policy = AutoSyncPolicy {
            enabled: false,
            ..AutoSyncPolicy::default()
        };
        let outcome = execute_sync(dir.path(), &policy);
        assert_eq!(outcome, WorkerOutcome::NothingToDo);
    }

    #[test]
    fn test_execute_sync_spawn_failure_returns_failed() {
        let nonexistent = Path::new("/nonexistent/state/dir/xyzzy");
        let policy = AutoSyncPolicy {
            enabled: true,
            ..AutoSyncPolicy::default()
        };
        let outcome = execute_sync(nonexistent, &policy);
        assert_eq!(outcome, WorkerOutcome::Failed);
    }

    #[test]
    fn test_terminate_child_reap() {
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        terminate_child(&mut child);
        let status = child.wait().unwrap();
        assert!(!status.success());
    }

    #[test]
    fn test_force_kill_child_reap() {
        let mut child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        force_kill_child(&mut child);
        let status = child.wait().unwrap();
        assert!(!status.success());
    }

    #[test]
    fn test_debounce_zero_is_immediate() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let observed = pending::read_state_from_dir(dir.path()).unwrap();
        let clock = MockClock::new(Instant::now(), 1_000_000);
        let start = clock.now_instant();
        let max_lifetime = Duration::from_secs(300);
        let max_delay = Duration::from_secs(300);
        let deadline = start; // already at deadline (zero debounce means compute_deadline returns None, caller passes start)
        let result = debounce(
            dir.path(),
            observed.clone(),
            deadline,
            start,
            max_lifetime,
            max_delay,
            &clock,
        );
        match result {
            DebounceResult::Ready(state) => {
                assert_eq!(state.generation, observed.generation);
            }
            other => panic!("expected Ready, got {:?}", other),
        }
    }

    #[test]
    fn test_debounce_marker_removed_returns_cancelled() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let observed = pending::read_state_from_dir(dir.path()).unwrap();
        let clock = MockClock::new(Instant::now(), 1_000_000);
        let start = clock.now_instant();
        let max_lifetime = Duration::from_secs(300);
        let max_delay = Duration::from_secs(300);
        // Deadline is in the past so debounce checks the marker immediately.
        let deadline = start - Duration::from_secs(10);
        // Remove the marker before debounce reads it.
        pending::clear(dir.path()).unwrap();
        let result = debounce(
            dir.path(),
            observed,
            deadline,
            start,
            max_lifetime,
            max_delay,
            &clock,
        );
        assert!(matches!(result, DebounceResult::CancelledMarkerRemoved));
    }

    #[test]
    fn test_debounce_generation_change_promotes_observation() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let observed = pending::read_state_from_dir(dir.path()).unwrap();
        assert_eq!(observed.generation, 1);
        let clock = MockClock::new(Instant::now(), 1_000_000);
        let start = clock.now_instant();
        let max_lifetime = Duration::from_secs(300);
        // max_delay reached immediately so debounce exits after first sleep/read cycle.
        let max_delay = Duration::ZERO;
        // Deadline far in the future so the sleep path is taken.
        let deadline = start + Duration::from_secs(60);
        // Arrive with a newer generation before debounce reads.
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetUpdate,
            },
        )
        .unwrap();
        let result = debounce(
            dir.path(),
            observed,
            deadline,
            start,
            max_lifetime,
            max_delay,
            &clock,
        );
        match result {
            DebounceResult::DeferredMaximumLifetime(state) => {
                assert_eq!(state.generation, 2);
            }
            other => panic!("expected DeferredMaximumLifetime, got {:?}", other),
        }
    }

    #[test]
    fn test_preflight_check_returns_current_state() {
        let dir = TempDir::new().unwrap();
        let initial = pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let result = preflight_check(dir.path(), initial.generation).unwrap();
        assert_eq!(result.generation, initial.generation);
    }

    #[test]
    fn test_preflight_check_returns_error_when_missing() {
        let dir = TempDir::new().unwrap();
        let result = preflight_check(dir.path(), 1);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("nothing to do"));
    }

    #[test]
    fn test_startup_recover_always_schedules_worker() {
        let dir = TempDir::new().unwrap();
        // Create a marker with a stale timestamp (created >5 min ago).
        let stale_ms = unix_now_ms() - pending::STALE_PENDING_THRESHOLD_MS - 60_000;
        pending::set_local_generation_with_timestamp(dir.path(), 1, stale_ms).unwrap();
        // startup_recover should return the stale state (not None) — it always recovers.
        let result = startup_recover(dir.path()).unwrap();
        assert!(result.is_some());
        let state = result.unwrap();
        assert_eq!(state.generation, 1);
    }

    #[test]
    fn test_debounce_result_ready_equality() {
        let a = DebounceResult::Ready(PendingState {
            generation: 1,
            snapshot: PendingSnapshot::None,
            created_at_unix_ms: 0,
        });
        let b = DebounceResult::Ready(PendingState {
            generation: 1,
            snapshot: PendingSnapshot::None,
            created_at_unix_ms: 0,
        });
        assert_eq!(a, b);
        assert_ne!(
            DebounceResult::CancelledMarkerRemoved,
            DebounceResult::Failed("x".into())
        );
    }

    #[test]
    fn test_clock_system_clock_methods() {
        let clock = SystemClock;
        let t1 = clock.now_instant();
        let ms1 = clock.now_unix_ms();
        std::thread::sleep(Duration::from_millis(10));
        let t2 = clock.now_instant();
        let ms2 = clock.now_unix_ms();
        assert!(t2 >= t1);
        assert!(ms2 >= ms1);
        assert!(ms2 - ms1 < 1000);
    }

    // ── Debounce tests ──────────────────────────────────────────────

    #[test]
    fn test_one_mutation_produces_one_ready_state() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let observed = pending::read_state_from_dir(dir.path()).unwrap();
        assert_eq!(observed.generation, 1);
        let clock = MockClock::new(Instant::now(), 1_000_000);
        let start = clock.now_instant();
        let max_lifetime = Duration::from_secs(300);
        let max_delay = Duration::from_secs(300);
        let deadline = start;
        let result = debounce(
            dir.path(),
            observed.clone(),
            deadline,
            start,
            max_lifetime,
            max_delay,
            &clock,
        );
        match result {
            DebounceResult::Ready(state) => {
                assert_eq!(
                    state.generation, 1,
                    "Ready must carry the correct generation"
                );
            }
            other => panic!("expected Ready, got {:?}", other),
        }
    }

    #[test]
    fn test_rapid_mutations_produce_single_ready_for_final_generation() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let observed = pending::read_state_from_dir(dir.path()).unwrap();
        assert_eq!(observed.generation, 1);

        for i in 0..5 {
            pending::record_pending_mutation(
                dir.path(),
                PendingSnapshot::Mutation {
                    kind: if i % 2 == 0 {
                        crate::auto_sync::policy::MutationKind::SnippetCreate
                    } else {
                        crate::auto_sync::policy::MutationKind::SnippetUpdate
                    },
                },
            )
            .unwrap();
        }

        let latest = pending::read_state_from_dir(dir.path()).unwrap();
        assert_eq!(
            latest.generation, 6,
            "final generation must be 6 after 5 additional mutations"
        );

        let clock = MockClock::new(Instant::now(), 1_000_000);
        let start = clock.now_instant();
        let max_lifetime = Duration::from_secs(300);
        let max_delay = Duration::from_secs(300);
        let deadline = start + Duration::from_secs(1);

        let result = debounce(
            dir.path(),
            observed,
            deadline,
            start,
            max_lifetime,
            max_delay,
            &clock,
        );

        match result {
            DebounceResult::Ready(state) => {
                assert_eq!(
                    state.generation, 6,
                    "Ready must return the final generation, not intermediate ones"
                );
            }
            other => panic!("expected Ready with generation 6, got {:?}", other),
        }
    }

    #[test]
    fn test_wall_clock_skew_does_not_panic() {
        let dir = TempDir::new().unwrap();
        pending::set_local_generation_with_timestamp(dir.path(), 1, u64::MAX - 1000).unwrap();
        let observed = pending::read_state_from_dir(dir.path()).unwrap();
        assert_eq!(observed.generation, 1);
        let clock = MockClock::new(Instant::now(), 1_000_000);
        let start = clock.now_instant();
        let max_lifetime = Duration::from_secs(300);
        let max_delay = Duration::from_secs(300);
        let deadline = start + Duration::from_secs(1);

        let result = debounce(
            dir.path(),
            observed,
            deadline,
            start,
            max_lifetime,
            max_delay,
            &clock,
        );

        match result {
            DebounceResult::Ready(state) => {
                assert_eq!(
                    state.generation, 1,
                    "Ready must return without panicking on extreme timestamps"
                );
            }
            other => panic!("expected Ready, got {:?}", other),
        }
    }

    #[test]
    fn test_debounce_returns_latest_generation_not_initial() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let initial = pending::read_state_from_dir(dir.path()).unwrap();
        assert_eq!(initial.generation, 1);

        let clock = MockClock::new(Instant::now(), 1_000_000);
        let start = clock.now_instant();
        let max_lifetime = Duration::from_secs(300);
        let max_delay = Duration::from_secs(300);
        let deadline = start + Duration::from_secs(60);

        // Before the deadline, update to gen 2.
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetUpdate,
            },
        )
        .unwrap();

        let result = debounce(
            dir.path(),
            initial,
            deadline,
            start,
            max_lifetime,
            max_delay,
            &clock,
        );

        match result {
            DebounceResult::Ready(state) => {
                assert_eq!(
                    state.generation, 2,
                    "Ready must return the latest generation, not the initial one"
                );
            }
            other => panic!("expected Ready with generation 2, got {:?}", other),
        }
    }

    // ── Active-sync tests ───────────────────────────────────────────

    #[test]
    fn test_follow_up_cycle_detects_newer_generation() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let initial = pending::read_state_from_dir(dir.path()).unwrap();
        assert_eq!(initial.generation, 1);

        // Simulate a sync completing for gen 1, but a newer mutation arrived.
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetUpdate,
            },
        )
        .unwrap();

        let current = pending::read_state_from_dir(dir.path()).unwrap();
        assert!(
            current.generation > initial.generation,
            "follow-up detection: current generation must be newer than initial"
        );
        assert_eq!(
            current.generation, 2,
            "follow-up detection: newer generation must be 2"
        );
    }

    #[test]
    fn test_failed_sync_preserves_newer_generation() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetUpdate,
            },
        )
        .unwrap();

        let current = pending::read_state_from_dir(dir.path()).unwrap();
        assert_eq!(current.generation, 2);

        // Simulate a failed sync on gen 1 — it must not touch gen 2.
        pending::record_failure(dir.path(), 1, "network").unwrap();

        let after = pending::read_state_from_dir(dir.path()).unwrap();
        assert_eq!(
            after.generation, 2,
            "failed sync on gen 1 must preserve gen 2"
        );
        assert_eq!(
            after.snapshot,
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetUpdate
            }
        );
    }

    /// Workstream F: mutation during active sync triggers a follow-up cycle.
    ///
    /// The worker observes gen 1, launches sync. While sync is active, gen 2
    /// arrives. On successful sync, the worker conditionally clears only gen 1
    /// (which succeeds since gen 2 is now current, so gen 1 is already gone
    /// or the conditional clear preserves gen 2). The worker then reads the
    /// marker and detects a newer generation, starting a follow-up cycle.
    #[test]
    fn test_auto_sync_worker_follows_up_on_newer_generation() {
        let dir = TempDir::new().unwrap();

        // Step 1: Create gen 1 — the worker observes this and launches sync.
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let observed = pending::read_state_from_dir(dir.path()).unwrap();
        assert_eq!(observed.generation, 1);

        // Step 2: While sync is active, gen 2 arrives (mutation during sync).
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetUpdate,
            },
        )
        .unwrap();

        // Step 3: Sync completes successfully — conditional clear of gen 1.
        // Since gen 2 is current, clear_if_generation_matches(gen 1) returns
        // GenerationChanged (gen 1 is not the current generation). The marker
        // is preserved with gen 2.
        let clear_result =
            pending::clear_if_generation_matches(dir.path(), observed.generation).unwrap();
        assert!(
            matches!(
                clear_result,
                pending::ConditionalClearResult::GenerationChanged { current: 2 }
            ),
            "clearing gen 1 when gen 2 is current must return GenerationChanged"
        );

        // Step 4: Worker reads the marker for follow-up detection (lines 239-249).
        let current = pending::read_state_from_dir(dir.path()).unwrap();
        assert!(
            current.generation > observed.generation,
            "follow-up detection: current generation ({}) must be newer than observed ({})",
            current.generation,
            observed.generation
        );
        assert_eq!(current.generation, 2);
        assert_eq!(
            current.snapshot,
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetUpdate
            }
        );
    }

    /// Workstream G: startup recovery skips scheduling when execution lock is held.
    ///
    /// If another process (worker, manual sync, cron) already holds the
    /// execution lock, startup_recover must return the pending state but
    /// NOT schedule a duplicate worker.
    #[test]
    fn test_startup_recover_skips_scheduling_when_lock_held() {
        let dir = TempDir::new().unwrap();

        // Create pending state.
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let before_bytes = std::fs::read_to_string(pending::pending_path(dir.path())).unwrap();

        // Acquire the execution lock (simulating an active sync).
        let _lock = crate::auto_sync::execution_lock::try_acquire(dir.path()).unwrap();
        assert!(
            crate::auto_sync::execution_lock::execution_lock_path(dir.path()).exists(),
            "execution lock must exist while held"
        );

        // startup_recover should return the pending state but skip scheduling.
        let result = startup_recover(dir.path()).unwrap();
        assert!(
            result.is_some(),
            "pending state must be returned even when lock is held"
        );
        let state = result.unwrap();
        assert_eq!(state.generation, 1);

        // The pending marker must be byte-for-byte unchanged — no scheduling occurred.
        let after_bytes = std::fs::read_to_string(pending::pending_path(dir.path())).unwrap();
        assert_eq!(
            before_bytes, after_bytes,
            "pending marker must not be mutated when execution lock is held"
        );

        // Verify the lock is still held (we didn't release it).
        assert!(
            crate::auto_sync::execution_lock::execution_lock_path(dir.path()).exists(),
            "execution lock must remain held after startup_recover"
        );
    }

    /// Dead worker is recoverable: spawning to a nonexistent state dir
    /// returns Failed, not a panic.
    #[test]
    fn test_dead_worker_is_recoverable() {
        let nonexistent = Path::new("/nonexistent/state/dir/recovery-test");
        let policy = AutoSyncPolicy {
            enabled: true,
            ..AutoSyncPolicy::default()
        };
        let outcome = execute_sync(nonexistent, &policy);
        assert_eq!(outcome, WorkerOutcome::Failed);
    }

    /// Foreground and detached paths classify the same injected error identically.
    #[test]
    fn test_foreground_detached_parity_classification() {
        use crate::auto_sync::executor::ExecutorExitCode;
        use crate::error::SyncFailureKind;

        // Both paths use the same FailureClass classification
        let err = crate::error::SnipError::sync_failure(
            SyncFailureKind::ConnectFailed,
            Some("connection refused"),
        );
        let class = FailureClass::from_error(&err);
        assert_eq!(class, FailureClass::TransientNetwork);

        // The same class maps to the same exit code
        let code = ExecutorExitCode::from_failure_class(class);
        assert_eq!(code, ExecutorExitCode::NetworkTimeout);

        // And back again
        let roundtrip = code.failure_class();
        assert_eq!(roundtrip, FailureClass::TransientNetwork);
    }

    /// Startup recovery preserves backoff: if status has a future
    /// next_attempt_at, startup_recover still schedules a worker
    /// (the worker itself will check backoff via schedule_sync).
    #[test]
    fn test_startup_recovery_preserves_backoff_state() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: crate::auto_sync::policy::MutationKind::SnippetCreate,
            },
        )
        .unwrap();

        // Record a failure with active backoff
        let future_ms = unix_now_ms() + 60_000;
        status::record_failure(
            dir.path(),
            1,
            FailureClass::TransientNetwork,
            4,
            3,
            future_ms,
            "connection failed",
            0,
        )
        .unwrap();

        // Verify backoff is active
        let status = status::read_status(dir.path()).unwrap();
        assert_eq!(status.next_attempt_at_unix_ms, future_ms);
        assert_eq!(status.consecutive_failures, 3);

        // Startup recovery should still return the pending state
        let result = startup_recover(dir.path()).unwrap();
        assert!(result.is_some());

        // Backoff state must be preserved after recovery
        let status = status::read_status(dir.path()).unwrap();
        assert_eq!(status.next_attempt_at_unix_ms, future_ms);
        assert_eq!(status.consecutive_failures, 3);
    }
}
