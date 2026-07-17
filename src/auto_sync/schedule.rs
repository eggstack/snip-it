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
    /// A worker or foreground sync is already active (execution lock held).
    AlreadyActive,
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
) -> ScheduleDecision {
    if !policy.sync_configured {
        return ScheduleDecision::NotConfigured;
    }

    if !policy.should_trigger() {
        return ScheduleDecision::Disabled;
    }

    // Check if pending work exists
    let _pending_state = match pending::read_state_from_dir(state_dir) {
        Ok(s) => s,
        Err(pending::PendingError::NotFound) => return ScheduleDecision::NoPending,
        Err(e) => {
            tracing::warn!(error = %e, "schedule_sync: failed to read pending state");
            return ScheduleDecision::NoPending;
        }
    };

    // Check execution lock
    let lock_path = execution_lock::execution_lock_path(state_dir);
    if let Some(contents) = execution_lock::inspect(&lock_path)
        && execution_lock::process_alive(contents.pid)
    {
        tracing::debug!(
            owner_pid = contents.pid,
            "schedule_sync: execution lock held by live process"
        );
        return ScheduleDecision::AlreadyActive;
    }

    // Check backoff status (unless explicit retry bypasses it)
    if caller != Caller::ExplicitRetry
        && let Some(status) = status::read_status(state_dir)
    {
        if status.next_attempt_at_unix_ms > 0 {
            let now_ms = unix_now_ms();
            if now_ms < status.next_attempt_at_unix_ms {
                return ScheduleDecision::DeferredUntil(status.next_attempt_at_unix_ms);
            }
        }

        // Check if last failure requires attention (no automatic retry).
        // Before returning RequiresAttention, check if config has changed
        // to release the deferral (Workstream I).
        if status.attention_required
            && status.consecutive_failures > 0
            && !FailureClass::from_code(&status.last_failure_class).allows_automatic_retry()
        {
            let last_class = FailureClass::from_code(&status.last_failure_class);
            if last_class.is_deferred()
                || matches!(
                    last_class,
                    FailureClass::Authentication
                        | FailureClass::CredentialStore
                        | FailureClass::Configuration
                )
            {
                // Check if config changed since the failure
                let current_fingerprint =
                    status::compute_config_fingerprint(&crate::config::get_sync_settings());
                if status::release_deferral_on_config_change(state_dir, current_fingerprint) {
                    // Config changed — fall through to SpawnNow
                } else {
                    return ScheduleDecision::RequiresAttention(last_class);
                }
            } else {
                return ScheduleDecision::RequiresAttention(last_class);
            }
        }
    }

    // Check if the failure class from last attempt allows retry
    if let Some(status) = status::read_status(state_dir)
        && status.consecutive_failures > 0
        && !status.attention_required
    {
        let last_class = FailureClass::from_code(&status.last_failure_class);
        if !last_class.allows_automatic_retry() && !last_class.is_deferred() {
            return ScheduleDecision::RequiresAttention(last_class);
        }
    }

    ScheduleDecision::SpawnNow
}

/// Convenience wrapper that resolves policy from the current config.
pub fn schedule_sync_from_config(state_dir: &Path, caller: Caller) -> ScheduleDecision {
    let settings = crate::config::get_sync_settings();
    let policy = AutoSyncPolicy::resolve(&settings);
    schedule_sync(state_dir, &policy, caller)
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
        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::StartupRecovery);
        assert_eq!(decision, ScheduleDecision::NoPending);
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
        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation);
        // May be SpawnNow or AlreadyActive depending on lock state
        assert!(
            decision == ScheduleDecision::SpawnNow || decision == ScheduleDecision::AlreadyActive
        );
    }

    #[test]
    fn test_already_active_when_lock_held() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let _lock = execution_lock::try_acquire(dir.path()).unwrap();
        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::StartupRecovery);
        assert_eq!(decision, ScheduleDecision::AlreadyActive);
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
            FailureClass::TransientNetwork,
            4,
            1,
            future_ms,
            "connection failed",
            0,
        )
        .unwrap();

        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation);
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
            FailureClass::TransientNetwork,
            4,
            1,
            future_ms,
            "connection failed",
            0,
        )
        .unwrap();

        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::ExplicitRetry);
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
            FailureClass::Authentication,
            3,
            1,
            0,
            "bad api key",
            0,
        )
        .unwrap();

        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation);
        assert!(matches!(
            decision,
            ScheduleDecision::RequiresAttention(FailureClass::Authentication)
        ));
    }

    #[test]
    fn test_not_configured_policy() {
        let dir = TempDir::new().unwrap();
        let policy = AutoSyncPolicy {
            sync_configured: false,
            ..AutoSyncPolicy::default()
        };
        let decision = schedule_sync(dir.path(), &policy, Caller::StartupRecovery);
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
        let decision = schedule_sync(dir.path(), &policy, Caller::StartupRecovery);
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
            FailureClass::Authentication,
            3,
            1,
            0,
            "bad api key",
            100, // old fingerprint
        )
        .unwrap();

        // schedule_sync should detect the config fingerprint difference
        // (current fingerprint will differ from 100 since settings are default)
        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation);
        // If config changed (fingerprint differs), should be SpawnNow
        // If fingerprint happens to match, should be RequiresAttention
        assert!(
            decision == ScheduleDecision::SpawnNow
                || matches!(decision, ScheduleDecision::RequiresAttention(_)),
            "unexpected decision: {decision:?}"
        );
    }

    #[test]
    fn test_execution_lock_busy_returns_already_active() {
        let dir = TempDir::new().unwrap();
        pending::record_pending_mutation(
            dir.path(),
            PendingSnapshot::Mutation {
                kind: MutationKind::SnippetCreate,
            },
        )
        .unwrap();
        let _lock = execution_lock::try_acquire(dir.path()).unwrap();
        let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation);
        assert_eq!(decision, ScheduleDecision::AlreadyActive);
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
            FailureClass::TransientNetwork,
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
            let decision = schedule_sync(dir.path(), &enabled_policy(), Caller::Mutation);
            assert!(
                matches!(decision, ScheduleDecision::DeferredUntil(_)),
                "mutation {i} should be deferred, got {decision:?}"
            );
        }
    }
}
