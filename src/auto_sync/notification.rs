//! Parent-side mutation notification API.
//!
//! Production flow:
//!
//! 1. Local mutation is committed (atomic).
//! 2. `notify_local_mutation` calls `pending::record_pending_mutation` —
//!    the only API that increments the pending generation.
//! 3. `worker::schedule_existing_pending` spawns a detached worker.
//!    This never mutates the pending marker.
//! 4. Parent returns immediately.

use crate::auto_sync::pending::{self, PendingSnapshot, PendingState};
use crate::auto_sync::policy::{AutoSyncPolicy, MutationKind, MutationOrigin};
use crate::auto_sync::worker::{self, SpawnResult};
use crate::config::{AutoSyncFailureMode, get_sync_settings};

pub struct MutationContext {
    pub kind: MutationKind,
    pub origin: MutationOrigin,
    pub library_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoSyncNotificationResult {
    Disabled,
    Suppressed,
    Scheduled { generation: u64 },
    SchedulingFailed { generation: Option<u64> },
}

pub fn notify_mutation(kind: MutationKind, origin: MutationOrigin) -> AutoSyncNotificationResult {
    eprintln!("NOTIFY-MUTATION-DIAG: entered kind={kind:?} origin={origin:?}");
    let settings = get_sync_settings();
    let policy = AutoSyncPolicy::resolve(&settings);
    eprintln!(
        "NOTIFY-MUTATION-DIAG: policy.enabled={} should_trigger={}",
        policy.enabled,
        policy.should_trigger()
    );
    notify_local_mutation(
        &policy,
        MutationContext {
            kind,
            origin,
            library_id: None,
        },
    )
}

/// Notify the auto-sync subsystem of a successful local mutation.
///
/// Performs one generation increment (via
/// `pending::record_pending_mutation`) and one worker spawn (via
/// `worker::schedule_existing_pending`). On spawn failure the pending
/// generation is preserved for recovery.
pub fn notify_local_mutation(
    policy: &AutoSyncPolicy,
    context: MutationContext,
) -> AutoSyncNotificationResult {
    if !policy.should_trigger() {
        return AutoSyncNotificationResult::Disabled;
    }

    if context.origin.should_suppress() {
        tracing::debug!(
            origin = ?context.origin,
            kind = ?context.kind,
            "auto-sync notification suppressed: sync-origin mutation"
        );
        return AutoSyncNotificationResult::Suppressed;
    }

    let state_dir = derive_state_dir();
    let snapshot = PendingSnapshot::Mutation { kind: context.kind };

    match pending::record_pending_mutation(&state_dir, snapshot) {
        Ok(marked) => match schedule_after_record(&state_dir, &marked) {
            SpawnResult::Spawned => AutoSyncNotificationResult::Scheduled {
                generation: marked.generation,
            },
            SpawnResult::Suppressed => AutoSyncNotificationResult::Suppressed,
            SpawnResult::SpawnFailed => AutoSyncNotificationResult::SchedulingFailed {
                generation: Some(marked.generation),
            },
        },
        Err(e) => {
            tracing::warn!(error = %e, "failed to record auto-sync pending generation");
            apply_scheduling_failure_policy(policy);
            AutoSyncNotificationResult::SchedulingFailed { generation: None }
        }
    }
}

fn schedule_after_record(state_dir: &std::path::Path, _marked: &PendingState) -> SpawnResult {
    worker::schedule_existing_pending(state_dir)
}

/// Clear pending intent after a successful explicit sync.
///
/// Generation-safe: callers must capture the observed generation **before**
/// running explicit sync via `observe_pending_generation`, then pass it
/// here. A mutation arriving during the sync is preserved.
pub fn clear_pending_after_explicit_sync(observed_generation: Option<u64>, sync_succeeded: bool) {
    let state_dir = derive_state_dir();
    let Some(generation) = observed_generation else {
        return;
    };
    let _ = worker::clear_after_explicit_sync(&state_dir, generation, sync_succeeded);
}

/// Reads the current pending generation, if any. Callers should capture
/// this **before** running an explicit sync, then pass the result to
/// `clear_pending_after_explicit_sync` along with whether sync succeeded.
pub fn observe_pending_generation() -> Option<u64> {
    let state_dir = derive_state_dir();
    worker::observed_pending_generation(&state_dir)
        .ok()
        .flatten()
}

pub fn startup_recover_pending() {
    let state_dir = derive_state_dir();
    let _ = worker::startup_recover(&state_dir);
}

pub fn derive_state_dir() -> std::path::PathBuf {
    crate::config::get_sync_config_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf()
}

fn apply_scheduling_failure_policy(policy: &AutoSyncPolicy) {
    match policy.failure_mode {
        AutoSyncFailureMode::Ignore => {
            tracing::debug!("auto-sync scheduling failed (ignored per policy)");
        }
        AutoSyncFailureMode::Warn => {
            eprintln!("warning: auto-sync scheduling failed; pending work preserved for recovery");
        }
        AutoSyncFailureMode::Error => {
            eprintln!("error: auto-sync scheduling failed; pending work preserved for recovery");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_policy_returns_disabled() {
        let policy = AutoSyncPolicy::default();
        let result = notify_local_mutation(
            &policy,
            MutationContext {
                kind: MutationKind::SnippetCreate,
                origin: MutationOrigin::User,
                library_id: None,
            },
        );
        assert_eq!(result, AutoSyncNotificationResult::Disabled);
    }

    #[test]
    fn test_sync_merge_origin_returns_suppressed() {
        let policy = AutoSyncPolicy {
            enabled: true,
            ..AutoSyncPolicy::default()
        };
        let result = notify_local_mutation(
            &policy,
            MutationContext {
                kind: MutationKind::SnippetCreate,
                origin: MutationOrigin::SyncMerge,
                library_id: None,
            },
        );
        assert_eq!(result, AutoSyncNotificationResult::Suppressed);
    }

    #[test]
    fn test_mutation_context_construction() {
        let ctx = MutationContext {
            kind: MutationKind::SnippetDelete,
            origin: MutationOrigin::User,
            library_id: Some("lib-1".to_string()),
        };
        assert_eq!(ctx.kind, MutationKind::SnippetDelete);
        assert_eq!(ctx.origin, MutationOrigin::User);
        assert_eq!(ctx.library_id.as_deref(), Some("lib-1"));
    }

    #[test]
    fn test_mutation_context_no_library() {
        let ctx = MutationContext {
            kind: MutationKind::Import,
            origin: MutationOrigin::Import,
            library_id: None,
        };
        assert!(ctx.library_id.is_none());
    }

    #[test]
    fn test_notification_result_equality() {
        assert_eq!(
            AutoSyncNotificationResult::Disabled,
            AutoSyncNotificationResult::Disabled
        );
        assert_eq!(
            AutoSyncNotificationResult::Suppressed,
            AutoSyncNotificationResult::Suppressed
        );
        assert_ne!(
            AutoSyncNotificationResult::Disabled,
            AutoSyncNotificationResult::Suppressed
        );
    }

    #[test]
    fn test_notification_result_debug() {
        let debug = format!("{:?}", AutoSyncNotificationResult::Disabled);
        assert_eq!(debug, "Disabled");
        let debug = format!("{:?}", AutoSyncNotificationResult::Suppressed);
        assert_eq!(debug, "Suppressed");
        let debug = format!(
            "{:?}",
            AutoSyncNotificationResult::Scheduled { generation: 42 }
        );
        assert!(debug.contains("Scheduled"));
        assert!(debug.contains("42"));
    }
}
