//! Detached one-shot auto-sync helper.
//!
//! The helper owns the shared execution lock for its whole bounded cycle,
//! debounces durable pending work, runs the canonical sync operation directly,
//! and conditionally clears only the generation it observed.

use crate::auto_sync::execution_lock::{self, ExecutionLockError, SyncExecutionLock};
use crate::auto_sync::pending::{self, PendingState};
use crate::auto_sync::policy::{AutoSyncPolicy, FailureClass, transient_backoff};
use crate::auto_sync::status;
use crate::config::get_sync_settings;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
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

/// Run one detached, bounded helper cycle.
pub fn run(state_dir: &Path) -> WorkerOutcome {
    let policy = AutoSyncPolicy::resolve(&get_sync_settings());
    crate::auto_sync::test_events::emit("worker", "started", std::process::id(), None, None);
    let lock = match execution_lock::try_acquire(state_dir) {
        Ok(lock) => lock,
        Err(ExecutionLockError::AlreadyHeld { .. }) => return WorkerOutcome::NothingToDo,
        Err(error) => {
            tracing::error!(%error, "auto-sync helper failed to acquire execution lock");
            return WorkerOutcome::Failed;
        }
    };
    run_locked(state_dir, lock, &policy)
}

fn run_locked(state_dir: &Path, lock: SyncExecutionLock, policy: &AutoSyncPolicy) -> WorkerOutcome {
    let _lock = lock;
    if !policy.enabled {
        return WorkerOutcome::NothingToDo;
    }
    let clock = SystemClock;
    let start = clock.now_instant();

    loop {
        if start.elapsed() >= policy.worker_lifetime {
            return WorkerOutcome::NothingToDo;
        }
        let pending_state = match pending::read_state_from_dir(state_dir) {
            Ok(state) => state,
            Err(pending::PendingError::NotFound) => return WorkerOutcome::NothingToDo,
            Err(error) => {
                tracing::error!(%error, "auto-sync helper could not read pending state");
                return WorkerOutcome::Failed;
            }
        };
        let observed = match debounce(
            state_dir,
            pending_state.clone(),
            compute_deadline(
                pending_state.created_at_unix_ms,
                policy.debounce,
                start,
                policy.worker_lifetime,
                &clock,
            ),
            start,
            policy.worker_lifetime,
            policy.max_delay,
            policy.debounce,
            &clock,
        ) {
            DebounceResult::Ready(state) | DebounceResult::DeferredMaximumLifetime(state) => state,
            DebounceResult::CancelledMarkerRemoved => return WorkerOutcome::NothingToDo,
            DebounceResult::Failed(error) => {
                return record_sync_failure(
                    state_dir,
                    pending_state.generation,
                    FailureClass::Internal,
                    &error,
                );
            }
        };
        let observed = match preflight_check(state_dir, observed.generation) {
            Ok(state) => state,
            Err(error) => {
                tracing::debug!(%error, "auto-sync helper preflight found no executable work");
                return WorkerOutcome::NothingToDo;
            }
        };
        let outcome = execute_sync(state_dir, policy, observed.generation);
        if !follow_up_allowed(outcome) {
            return outcome;
        }
        match pending::read_state_from_dir(state_dir) {
            Ok(current) if current.generation > observed.generation => continue,
            Ok(current) if current.generation < observed.generation => {
                return record_sync_failure(
                    state_dir,
                    observed.generation,
                    FailureClass::Internal,
                    "pending generation rollback after sync",
                );
            }
            Ok(_) | Err(pending::PendingError::NotFound) => return outcome,
            Err(error) => {
                return record_sync_failure(
                    state_dir,
                    observed.generation,
                    FailureClass::Internal,
                    &format!("pending reload failed after sync: {error}"),
                );
            }
        }
    }
}

fn follow_up_allowed(outcome: WorkerOutcome) -> bool {
    matches!(outcome, WorkerOutcome::Success)
}

fn compute_deadline(
    observed_timestamp_ms: u64,
    debounce: Duration,
    start: Instant,
    lifetime: Duration,
    clock: &dyn Clock,
) -> Instant {
    let target = unix_ms_to_instant(
        observed_timestamp_ms.saturating_add(debounce.as_millis() as u64),
        clock,
    );
    target.min(start.checked_add(lifetime).unwrap_or(target))
}

pub fn debounce(
    state_dir: &Path,
    observed: PendingState,
    initial_deadline: Instant,
    start: Instant,
    max_lifetime: Duration,
    max_delay: Duration,
    debounce_duration: Duration,
    clock: &dyn Clock,
) -> DebounceResult {
    let mut current = observed;
    let mut deadline = initial_deadline;
    let max_target = start.checked_add(max_lifetime).unwrap_or(deadline);
    loop {
        if clock.now_instant() >= deadline || clock.now_instant() >= max_target {
            match pending::read_state_from_dir(state_dir) {
                Ok(latest) if latest.generation > current.generation => {
                    current = latest;
                    deadline = compute_deadline(
                        current.created_at_unix_ms,
                        debounce_duration,
                        start,
                        max_lifetime,
                        clock,
                    )
                    .min(max_target);
                    continue;
                }
                Ok(latest) if latest.generation < current.generation => {
                    return DebounceResult::Failed(
                        "pending generation rollback during debounce".into(),
                    );
                }
                Ok(_) => return DebounceResult::Ready(current),
                Err(pending::PendingError::NotFound) => {
                    return DebounceResult::CancelledMarkerRemoved;
                }
                Err(error) => return DebounceResult::Failed(error.to_string()),
            }
        }
        let sleep_for = deadline
            .saturating_duration_since(clock.now_instant())
            .min(Duration::from_millis(250));
        clock.sleep(sleep_for);
        match pending::read_state_from_dir(state_dir) {
            Ok(latest) if latest.generation > current.generation => {
                current = latest;
                deadline = compute_deadline(
                    current.created_at_unix_ms,
                    debounce_duration,
                    start,
                    max_lifetime,
                    clock,
                )
                .min(max_target);
            }
            Ok(latest) if latest.generation < current.generation => {
                return DebounceResult::Failed(
                    "pending generation rollback during debounce".into(),
                );
            }
            Ok(_) => {}
            Err(pending::PendingError::NotFound) => return DebounceResult::CancelledMarkerRemoved,
            Err(error) => return DebounceResult::Failed(error.to_string()),
        }
        if clock.now_instant().duration_since(start) >= max_delay {
            return DebounceResult::DeferredMaximumLifetime(current);
        }
    }
}

pub fn preflight_check(state_dir: &Path, observed_generation: u64) -> Result<PendingState, String> {
    match pending::read_state_from_dir(state_dir) {
        Ok(state) if state.generation < observed_generation => Err(format!(
            "pending generation rollback: observed {} after {}",
            state.generation, observed_generation
        )),
        Ok(state) => Ok(state),
        Err(pending::PendingError::NotFound) => Err("pending marker removed, nothing to do".into()),
        Err(error) => Err(format!("corrupt pending state: {error}")),
    }
}

fn unix_ms_to_instant(target_unix_ms: u64, clock: &dyn Clock) -> Instant {
    let now = clock.now_unix_ms();
    if target_unix_ms <= now {
        clock.now_instant()
    } else {
        clock
            .now_instant()
            .checked_add(Duration::from_millis(target_unix_ms - now))
            .unwrap_or_else(|| clock.now_instant())
    }
}

fn execute_sync(state_dir: &Path, policy: &AutoSyncPolicy, generation: u64) -> WorkerOutcome {
    if !policy.enabled {
        return WorkerOutcome::NothingToDo;
    }
    crate::auto_sync::test_events::emit(
        "worker",
        "sync_started",
        std::process::id(),
        Some(generation),
        None,
    );
    let settings = get_sync_settings();
    let (push_only, pull_only) = match settings.sync_direction {
        crate::config::SyncDirection::Push => (true, false),
        crate::config::SyncDirection::Pull => (false, true),
        crate::config::SyncDirection::Bidirectional => (false, false),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("snp-auto-sync")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return record_sync_failure(
                state_dir,
                generation,
                FailureClass::Internal,
                &format!("runtime creation failed: {error}"),
            );
        }
    };
    let deadline = Instant::now()
        .checked_add(policy.sync_timeout)
        .unwrap_or_else(Instant::now);
    let limits = crate::sync::SyncRunLimits {
        deadline,
        request_timeout: policy.sync_timeout,
    };
    match crate::sync_commands::run_sync_with_limits(
        &settings,
        None,
        push_only,
        pull_only,
        &runtime,
        Some(limits),
    ) {
        Ok(()) => match pending::clear_if_generation_matches(state_dir, generation) {
            Ok(pending::ConditionalClearResult::Cleared) => {
                let _ = status::record_success(
                    state_dir,
                    generation,
                    "canonical sync acknowledged; pending cleared",
                );
                emit_sync_completed(generation, true, "pending_cleared");
                WorkerOutcome::Success
            }
            Ok(pending::ConditionalClearResult::GenerationChanged { current }) => {
                let _ = status::record_success(
                    state_dir,
                    generation,
                    "canonical sync acknowledged; newer generation preserved",
                );
                tracing::info!(
                    generation,
                    current,
                    "auto-sync preserved newer pending generation"
                );
                emit_sync_completed(generation, true, "newer_generation_preserved");
                WorkerOutcome::Success
            }
            Ok(pending::ConditionalClearResult::Missing) => {
                let _ = status::record_success(
                    state_dir,
                    generation,
                    "canonical sync completed; pending was already cleared",
                );
                emit_sync_completed(generation, true, "pending_already_missing");
                WorkerOutcome::Success
            }
            Err(error) => record_sync_failure(
                state_dir,
                generation,
                FailureClass::LocalPersistence,
                &format!("pending clear failed after successful sync: {error}"),
            ),
        },
        Err(error) => record_sync_failure(
            state_dir,
            generation,
            FailureClass::from_error(&error),
            &error.to_string(),
        ),
    }
}

fn record_sync_failure(
    state_dir: &Path,
    generation: u64,
    class: FailureClass,
    message: &str,
) -> WorkerOutcome {
    let consecutive = next_consecutive_failures(state_dir);
    let backoff = transient_backoff(consecutive);
    let _ = status::record_failure(
        state_dir,
        generation,
        class,
        -1,
        consecutive,
        unix_now_ms().saturating_add(backoff.as_millis() as u64),
        message,
        current_config_fingerprint(),
    );
    emit_sync_completed(generation, false, class.as_code());
    WorkerOutcome::Failed
}

fn emit_sync_completed(generation: u64, success: bool, reason: &str) {
    crate::auto_sync::test_events::emit(
        "worker",
        "sync_completed",
        std::process::id(),
        Some(generation),
        Some(serde_json::json!({"success": success, "reason": reason}).to_string()),
    );
}

fn next_consecutive_failures(state_dir: &Path) -> u32 {
    match status::read_status_typed(state_dir) {
        status::StatusRead::Valid(status) => status.consecutive_failures.saturating_add(1),
        _ => 1,
    }
}

fn current_config_fingerprint() -> u64 {
    status::compute_config_fingerprint(&get_sync_settings())
}
fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn startup_recover(state_dir: &Path) -> Result<Option<PendingState>, pending::PendingError> {
    if !pending::pending_path(state_dir).exists() {
        return Ok(None);
    }
    let current = pending::read_state_from_dir(state_dir)?;
    let policy = AutoSyncPolicy::resolve(&get_sync_settings());
    if let Err(error) = crate::auto_sync::schedule::schedule_and_spawn(
        state_dir,
        &policy,
        crate::auto_sync::schedule::Caller::StartupRecovery,
    ) {
        tracing::warn!(%error, "startup recovery scheduling failed; pending work preserved");
    }
    Ok(Some(current))
}

pub fn clear_after_explicit_sync(
    state_dir: &Path,
    observed_generation: u64,
    sync_succeeded: bool,
) -> Result<bool, pending::PendingError> {
    if !sync_succeeded {
        return Ok(false);
    }
    Ok(matches!(
        pending::clear_if_generation_matches(state_dir, observed_generation)?,
        pending::ConditionalClearResult::Cleared
    ))
}

pub fn observed_pending_generation(state_dir: &Path) -> Result<Option<u64>, pending::PendingError> {
    match pending::read_state_from_dir(state_dir) {
        Ok(state) => Ok(Some(state.generation)),
        Err(pending::PendingError::NotFound) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkerOutcome, follow_up_allowed};

    #[test]
    fn failed_or_empty_sync_cannot_enter_new_generation_follow_up() {
        assert!(!follow_up_allowed(WorkerOutcome::Failed));
        assert!(!follow_up_allowed(WorkerOutcome::NothingToDo));
        assert!(follow_up_allowed(WorkerOutcome::Success));
    }
}
