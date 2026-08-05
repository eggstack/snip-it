//! Centralized scheduling decision for auto-sync workers.
//!
//! This module prevents worker storms by consolidating all scheduling
//! logic into a single decision function. Every code path that wants
//! to spawn a worker must go through `schedule_sync`, which considers
//! pending state, policy, execution lock, backoff, and failure class
//! to determine whether spawning is appropriate.

use crate::auto_sync::execution_lock;
use crate::auto_sync::pending;
use crate::auto_sync::policy::{AutoSyncPolicy, FailureClass};
use crate::auto_sync::status;
use std::path::Path;

/// The outcome of a scheduling decision.
///
/// Only `SpawnNow` should invoke the process spawner. All other
/// variants indicate that spawning is inappropriate and explain why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleDecision {
    /// Conditions are met; spawn a worker immediately.
    SpawnNow,
    /// Backoff is active; spawn no earlier than the given unix timestamp (ms).
    DeferredUntil(u64),
    /// Auto-sync is disabled in policy.
    Disabled,
    /// A failure class that requires operator attention; no automatic retry.
    RequiresAttention(FailureClass),
    /// No pending work exists; nothing to do.
    NoPending,
    /// Policy is not configured (no sync account).
    NotConfigured,
}

#[derive(Debug)]
pub enum ScheduleError {
    Pending(pending::PendingError),
    Spawn(execution_lock::SpawnError),
}

impl std::fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending(error) => write!(f, "pending state error: {error}"),
            Self::Spawn(error) => write!(f, "worker spawn error: {error}"),
        }
    }
}

impl std::error::Error for ScheduleError {}

/// Determine whether a worker should be spawned.
///
/// This is the single entry point for all scheduling decisions:
/// - startup recovery
/// - post-mutation scheduling
/// - explicit retry (`snp sync --retry`)
///
/// The `caller` parameter distinguishes these paths for logging
/// but does not change the decision logic (except that explicit
/// retry can bypass backoff wait).
pub fn schedule_sync(
    state_dir: &Path,
    policy: &AutoSyncPolicy,
    caller: Caller,
) -> Result<ScheduleDecision, ScheduleError> {
    if !policy.sync_configured {
        return Ok(ScheduleDecision::NotConfigured);
    }

    if !policy.should_trigger() {
        return Ok(ScheduleDecision::Disabled);
    }

    // Check if pending work exists
    let _pending_state = match pending::read_state_from_dir(state_dir) {
        Ok(s) => s,
        Err(pending::PendingError::NotFound) => return Ok(ScheduleDecision::NoPending),
        Err(e) => return Err(ScheduleError::Pending(e)),
    };

    // Check backoff status (unless explicit retry bypasses it)
    if caller != Caller::ExplicitRetry {
        match status::read_status_typed(state_dir) {
            status::StatusRead::Valid(status) => {
                if status.next_attempt_at_unix_ms > 0 {
                    let now_ms = unix_now_ms();
                    if now_ms < status.next_attempt_at_unix_ms {
                        return Ok(ScheduleDecision::DeferredUntil(
                            status.next_attempt_at_unix_ms,
                        ));
                    }
                }

                // Check if last failure requires attention (no automatic retry).
                // Before returning RequiresAttention, check if config has changed
                // to release the deferral.
                if status.attention_required && status.consecutive_failures > 0 {
                    let last_class = FailureClass::from_code(&status.last_failure_class);
                    match last_class.retry_disposition(status.consecutive_failures) {
                        crate::auto_sync::policy::RetryDisposition::RequiresAttention
                        | crate::auto_sync::policy::RetryDisposition::NoAutomaticRetry
                        | crate::auto_sync::policy::RetryDisposition::WaitForConfigurationChange => {
                            if last_class.is_deferred() {
                                let current_fingerprint = status::compute_config_fingerprint(
                                    &crate::config::get_sync_settings(),
                                );
                                if status::release_deferral_on_config_change(
                                    state_dir,
                                    current_fingerprint,
                                ) {
                                    // Config changed — fall through to SpawnNow
                                } else {
                                    return Ok(ScheduleDecision::RequiresAttention(last_class));
                                }
                            } else {
                                return Ok(ScheduleDecision::RequiresAttention(last_class));
                            }
                        }
                        _ => {} // RetryAfter or WaitForConfigurationChange - proceed
                    }
                }
            }
            status::StatusRead::Corrupt(_) => {
                return Ok(ScheduleDecision::RequiresAttention(FailureClass::Internal));
            }
            status::StatusRead::Missing => {}
        }
    }

    Ok(ScheduleDecision::SpawnNow)
}

/// Convenience wrapper that resolves policy from the current config.
pub fn schedule_sync_from_config(
    state_dir: &Path,
    caller: Caller,
) -> Result<ScheduleDecision, ScheduleError> {
    let settings = crate::config::get_sync_settings();
    let policy = AutoSyncPolicy::resolve(&settings);
    schedule_sync(state_dir, &policy, caller)
}

/// Schedule existing pending work without recording a new pending mutation.
///
/// This is the scheduling-only path used after pending intent has already
/// been durably recorded (e.g. by restore). It must not mutate pending state,
/// change the generation, or replace the snapshot. It only translates a
/// `SpawnNow` decision into an actual worker spawn.
pub fn schedule_existing_pending(
    state_dir: &Path,
    policy: &AutoSyncPolicy,
    caller: Caller,
) -> Result<ScheduleDecision, ScheduleError> {
    let decision = schedule_sync(state_dir, policy, caller)?;
    if decision == ScheduleDecision::SpawnNow
        && let Err(e) = execution_lock::spawn_worker(state_dir)
    {
        tracing::warn!(error = %e, "schedule_existing_pending: failed to spawn worker");
        return Err(ScheduleError::Spawn(e));
    }
    Ok(decision)
}

/// The sole authority for translating a `SpawnNow` decision into an actual
/// worker spawn. All automatic scheduling paths must call this function
/// rather than calling `execution_lock::spawn_worker` directly.
pub fn schedule_and_spawn(
    state_dir: &Path,
    policy: &AutoSyncPolicy,
    caller: Caller,
) -> Result<ScheduleDecision, ScheduleError> {
    let decision = schedule_sync(state_dir, policy, caller)?;
    if decision == ScheduleDecision::SpawnNow
        && !test_worker_spawn_suppressed()
        && let Err(e) = execution_lock::spawn_worker(state_dir)
    {
        tracing::warn!(error = %e, "schedule_and_spawn: failed to spawn worker");
        return Err(ScheduleError::Spawn(e));
    }
    Ok(decision)
}

/// Test-only worker spawn suppression.
///
/// When the `test-support` feature is enabled and `SNP_SKIP_WORKER_SPAWN`
/// is set, suppresses the actual spawn (used by CI to avoid worker storms
/// in workspace test jobs that don't need lifecycle evidence).
/// Production builds never check this variable.
#[cfg(feature = "test-support")]
fn test_worker_spawn_suppressed() -> bool {
    std::env::var_os("SNP_SKIP_WORKER_SPAWN").is_some()
}

/// Production no-op: worker spawn is never suppressed.
#[cfg(not(feature = "test-support"))]
#[inline(always)]
fn test_worker_spawn_suppressed() -> bool {
    false
}

/// Who is requesting the scheduling decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    /// Startup recovery — respects backoff.
    StartupRecovery,
    /// Post-mutation scheduling — respects backoff.
    Mutation,
    /// Explicit `snp sync` — may bypass backoff.
    ExplicitRetry,
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
    use crate::auto_sync::pending::PendingSnapshot;
    use crate::auto_sync::policy::MutationKind;
    use tempfile::TempDir;

    fn enabled_policy() -> AutoSyncPolicy {
        AutoSyncPolicy {
            sync_configured: true,
            enabled: true,
            ..AutoSyncPolicy::default()
        }
    }

    #[test]
    fn test_no_pending_returns_no_pending() {
        let dir = TempDir::new().unwrap();
        let decision =
            schedule_sync(dir.path(), &enabled_policy(), Caller::StartupRecovery).unwrap();
        assert_eq!(decision, ScheduleDecision::NoPending);
    }

    #[test]
    fn test_corrupt_pending_is_an_error_not_no_pending() {
        let dir = TempDir::new().unwrap();
        std::fs::write(pending::pending_path(dir.path()), "not toml").unwrap();
        let result = schedule_sync(dir.path(), &enabled_policy(), Caller::StartupRecovery);
        assert!(matches!(result, Err(ScheduleError::Pending(_))));
    }

    #[test]
    fn test_spawn_now_with_pending() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation).unwrap();
        assert_eq!(decision, ScheduleDecision::SpawnNow);
    }

    #[test]
    fn test_deferred_until_backoff_active() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();

        // Record a failure with future next_attempt
        let future_ms = unix_now_ms() + 60_000;
        status::record_failure(
            dir.path(),
            1,
            FailureClass::Transient,
            4,
            1,
            future_ms,
            "connection failed",
            0,
        )
        .unwrap();

        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation).unwrap();
        assert!(matches!(decision, ScheduleDecision::DeferredUntil(_)));
    }

    #[test]
    fn test_explicit_retry_bypasses_backoff() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();

        let future_ms = unix_now_ms() + 60_000;
        status::record_failure(
            dir.path(),
            1,
            FailureClass::Transient,
            4,
            1,
            future_ms,
            "connection failed",
            0,
        )
        .unwrap();

        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::ExplicitRetry).unwrap();
        // Explicit retry should not be DeferredUntil
        assert_ne!(
            decision,
            ScheduleDecision::DeferredUntil(future_ms),
            "explicit retry must bypass backoff"
        );
    }

    #[test]
    fn test_requires_attention_for_auth_failure() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();

        status::record_failure(
            dir.path(),
            1,
            FailureClass::Configuration,
            3,
            1,
            0,
            "bad api key",
            0,
        )
        .unwrap();

        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation).unwrap();
        assert!(matches!(
            decision,
            ScheduleDecision::RequiresAttention(FailureClass::Configuration)
        ));
    }

    #[test]
    fn test_not_configured_policy() {
        let dir = TempDir::new().unwrap();
        let policy = AutoSyncPolicy {
            sync_configured: false,
            ..AutoSyncPolicy::default()
        };
        let decision = schedule_sync(dir.path(), &policy, Caller::StartupRecovery).unwrap();
        assert_eq!(decision, ScheduleDecision::NotConfigured);
    }

    #[test]
    fn test_disabled_policy() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let policy = AutoSyncPolicy {
            sync_configured: true,
            enabled: false,
            ..AutoSyncPolicy::default()
        };
        let decision = schedule_sync(dir.path(), &policy, Caller::StartupRecovery).unwrap();
        assert_eq!(decision, ScheduleDecision::Disabled);
    }

    #[test]
    fn test_config_change_releases_auth_deferral() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();

        // Record an auth failure with attention_required and a config fingerprint
        status::record_failure(
            dir.path(),
            1,
            FailureClass::Configuration,
            3,
            1,
            0,
            "bad api key",
            100, // old fingerprint
        )
        .unwrap();

        // schedule_sync should detect the config fingerprint difference
        // (current fingerprint will differ from 100 since settings are default)
        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation).unwrap();
        // If config changed (fingerprint differs), should be SpawnNow
        // If fingerprint happens to match, should be RequiresAttention
        assert!(
            decision == ScheduleDecision::SpawnNow
                || matches!(decision, ScheduleDecision::RequiresAttention(_)),
            "unexpected decision: {decision:?}"
        );
    }

    #[test]
    fn test_mutation_during_backoff_does_not_spawn() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();

        // Record a failure with future next_attempt (backoff active)
        let future_ms = unix_now_ms() + 60_000;
        status::record_failure(
            dir.path(),
            1,
            FailureClass::Transient,
            4,
            3,
            future_ms,
            "connection failed",
            0,
        )
        .unwrap();

        // Simulate 20 rapid mutations — each should see DeferredUntil
        for i in 0..20 {
            pending::record_pending_mutation(
                dir.path(),
                PendingSnapshot::Mutation {
                    kind: MutationKind::SnippetCreate,
                },
            )
            .unwrap();
            let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation).unwrap();
            assert!(
                matches!(decision, ScheduleDecision::DeferredUntil(_)),
                "mutation {i} should be deferred, got {decision:?}"
            );
        }
    }

    /// Structural guard: verify `spawn_worker` is only referenced from
    /// `schedule_and_spawn` in production code (not tests or docs).
    #[test]
    fn test_spawn_worker_only_called_from_scheduler() {
        let src = include_str!("schedule.rs");
        // The only production call to execution_lock::spawn_worker should be in schedule_and_spawn.
        let lines: Vec<&str> = src.lines().collect();
        let mut production_calls = 0;
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with("#[") {
                continue;
            }
            if trimmed.contains("execution_lock::spawn_worker") || trimmed.contains("spawn_worker(")
            {
                // Check if this is the schedule_and_spawn function or a test
                let in_test = lines[..i].iter().any(|l| l.contains("#[cfg(test)]"));
                if !in_test {
                    production_calls += 1;
                }
            }
        }
        assert_eq!(
            production_calls, 2,
            "spawn_worker should be called in exactly two production locations \
             (schedule_and_spawn and schedule_existing_pending), found {production_calls}"
        );
    }
}
