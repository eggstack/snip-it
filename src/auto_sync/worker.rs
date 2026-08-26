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
        if start.elapsed() >= policy.max_lifetime {
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
                policy.max_lifetime,
                &clock,
            ),
            start,
            policy.max_lifetime,
            policy.debounce,
            &clock,
        ) {
            DebounceResult::Ready(state) => state,
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

/// Interpret a pending observation whose generation is lower than the one
/// being debounced.
///
/// A lower generation normally indicates corrupt rollback (fail closed), but
/// it is benign when an explicit sync cleared the marker and a new mutation
/// immediately re-recorded one: `record_pending_mutation` restarts at
/// generation 1 with a fresh creation timestamp. Such a reset is adopted as
/// the new debounce target; a regression that keeps an equal-or-older
/// creation timestamp remains corrupt state.
fn adopt_generation_reset(current: &PendingState, latest: PendingState) -> Option<PendingState> {
    // A generation of one is only produced when no pending marker existed.
    // Accept it even when the wall clock moved backwards between the clear and
    // recreate; otherwise a legitimate reset can be mistaken for corruption.
    if (latest.generation == 1 && latest.created_at_unix_ms != current.created_at_unix_ms)
        || latest.created_at_unix_ms > current.created_at_unix_ms
    {
        tracing::info!(
            observed_generation = current.generation,
            recreated_generation = latest.generation,
            "pending marker was cleared and recreated during debounce; adopting new generation"
        );
        Some(latest)
    } else {
        None
    }
}

pub fn debounce(
    state_dir: &Path,
    observed: PendingState,
    initial_deadline: Instant,
    start: Instant,
    max_lifetime: Duration,
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
                    match adopt_generation_reset(&current, latest) {
                        Some(recreated) => {
                            current = recreated;
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
                        None => {
                            return DebounceResult::Failed(
                                "pending generation rollback during debounce".into(),
                            );
                        }
                    }
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
                match adopt_generation_reset(&current, latest) {
                    // Track the freshly recreated marker through a new full
                    // debounce window instead of failing the cycle.
                    Some(recreated) => {
                        current = recreated;
                        deadline = compute_deadline(
                            current.created_at_unix_ms,
                            debounce_duration,
                            start,
                            max_lifetime,
                            clock,
                        )
                        .min(max_target);
                    }
                    None => {
                        return DebounceResult::Failed(
                            "pending generation rollback during debounce".into(),
                        );
                    }
                }
            }
            Ok(_) => {}
            Err(pending::PendingError::NotFound) => return DebounceResult::CancelledMarkerRemoved,
            Err(error) => return DebounceResult::Failed(error.to_string()),
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
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
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
                FailureClass::LocalFailure,
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
        return Err(pending::PendingError::Corrupted(format!(
            "startup recovery scheduling failed: {error}"
        )));
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
    use super::{DebounceResult, WorkerOutcome, follow_up_allowed};
    use crate::auto_sync::pending::{self, PendingSnapshot};
    use crate::auto_sync::policy::MutationKind;
    use std::path::Path;
    use std::time::{Duration, Instant};

    #[test]
    fn failed_or_empty_sync_cannot_enter_new_generation_follow_up() {
        assert!(!follow_up_allowed(WorkerOutcome::Failed));
        assert!(!follow_up_allowed(WorkerOutcome::NothingToDo));
        assert!(follow_up_allowed(WorkerOutcome::Success));
    }

    fn write_marker(state_dir: &Path, generation: u64, created_at_ms: u64) {
        pending::set_local_generation_with_timestamp(state_dir, generation, created_at_ms).unwrap();
    }

    fn mutation_snapshot() -> PendingSnapshot {
        PendingSnapshot::Mutation {
            kind: MutationKind::SnippetCreate,
        }
    }

    /// Explicit-sync clear + immediate re-record (generation restarts at 1
    /// with a newer creation timestamp) is benign: debounce must adopt it
    /// instead of classifying it as corrupt rollback.
    #[test]
    fn debounce_adopts_cleared_and_recreated_marker() {
        let dir = tempfile::TempDir::new().unwrap();
        let base_ms: u64 = 1_700_000_000_000;

        // The marker being debounced.
        write_marker(dir.path(), 3, base_ms);

        // Mid-debounce, an explicit sync clears the marker and a new
        // mutation re-records one at generation 1.
        pending::clear(dir.path()).unwrap();
        write_marker(dir.path(), 1, base_ms + 5_000);

        // What the worker observed before the clear/re-record race.
        let observed = super::PendingState {
            generation: 3,
            snapshot: mutation_snapshot(),
            created_at_unix_ms: base_ms,
        };

        let start = Instant::now();
        let result = super::debounce(
            dir.path(),
            observed,
            start - Duration::from_secs(1),
            start,
            Duration::from_secs(300),
            Duration::ZERO,
            &super::SystemClock,
        );

        match result {
            DebounceResult::Ready(state) => {
                assert_eq!(state.generation, 1);
                assert_eq!(state.created_at_unix_ms, base_ms + 5_000);
            }
            other => panic!("expected Ready(gen=1) after recreation, got {other:?}"),
        }
    }

    /// A lower generation with an equal-or-older creation timestamp is
    /// genuine rollback and must still fail closed.
    #[test]
    fn debounce_fails_closed_on_genuine_generation_rollback() {
        let dir = tempfile::TempDir::new().unwrap();
        let base_ms: u64 = 1_700_000_000_000;

        write_marker(dir.path(), 3, base_ms);
        // Corrupt rewind: same creation timestamp, lower generation.
        write_marker(dir.path(), 1, base_ms);

        let start = Instant::now();
        let observed = super::PendingState {
            generation: 3,
            snapshot: mutation_snapshot(),
            created_at_unix_ms: base_ms,
        };
        let result = super::debounce(
            dir.path(),
            observed,
            start - Duration::from_secs(1),
            start,
            Duration::from_secs(300),
            Duration::ZERO,
            &super::SystemClock,
        );

        assert!(
            matches!(result, DebounceResult::Failed(ref msg) if msg.contains("rollback")),
            "expected Failed(rollback), got {result:?}"
        );
        // The corrupt marker must be preserved for inspection/repair.
        assert_eq!(
            pending::read_state_from_dir(dir.path()).unwrap().generation,
            1
        );
    }

    #[test]
    fn debounce_adopts_generation_reset_after_clock_moves_backwards() {
        let dir = tempfile::TempDir::new().unwrap();
        let base_ms: u64 = 1_700_000_000_000;
        write_marker(dir.path(), 3, base_ms);
        write_marker(dir.path(), 1, base_ms - 5_000);

        let start = Instant::now();
        let observed = super::PendingState {
            generation: 3,
            snapshot: mutation_snapshot(),
            created_at_unix_ms: base_ms,
        };
        let result = super::debounce(
            dir.path(),
            observed,
            start - Duration::from_secs(1),
            start,
            Duration::from_secs(300),
            Duration::ZERO,
            &super::SystemClock,
        );

        assert!(matches!(result, DebounceResult::Ready(state) if state.generation == 1));
    }
}
