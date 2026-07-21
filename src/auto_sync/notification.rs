//! Parent-side mutation notification API.
//!
//! Production flow:
//!
//! 1. Local mutation is committed (atomic).
//! 2. `notify_local_mutation` calls `pending::record_pending_mutation` —
//!    the only API that increments the pending generation.
//! 3. `schedule::schedule_and_spawn` spawns a detached worker.
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
    /// Pending intent recorded but no worker scheduled (auto-sync disabled,
    /// sync account exists).
    PendingRecorded {
        generation: u64,
    },
    Scheduled {
        generation: u64,
    },
    SchedulingFailed {
        generation: Option<u64>,
    },
}

pub fn notify_mutation(kind: MutationKind, origin: MutationOrigin) -> AutoSyncNotificationResult {
    let settings = get_sync_settings();
    let policy = AutoSyncPolicy::resolve(&settings);
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
/// `schedule::schedule_and_spawn`). On spawn failure the pending
/// generation is preserved for recovery.
pub fn notify_local_mutation(
    policy: &AutoSyncPolicy,
    context: MutationContext,
) -> AutoSyncNotificationResult {
    notify_local_mutation_with_dir(policy, context, &derive_state_dir())
}

fn notify_local_mutation_with_dir(
    policy: &AutoSyncPolicy,
    context: MutationContext,
    state_dir: &std::path::Path,
) -> AutoSyncNotificationResult {
    if !policy.sync_configured {
        // Even if policy says not configured, check if config file exists.
        // If it does, the config is broken — still record pending so the
        // mutation is not lost.
        let sync_path = state_dir.join("sync.toml");
        if !sync_path.exists() {
            return AutoSyncNotificationResult::Disabled;
        }
        // Config exists but is broken — fall through to record pending
    }

    if context.origin.should_suppress() {
        tracing::debug!(
            origin = ?context.origin,
            kind = ?context.kind,
            "auto-sync notification suppressed: sync-origin mutation"
        );
        return AutoSyncNotificationResult::Suppressed;
    }

    let snapshot = PendingSnapshot::Mutation { kind: context.kind };

    match pending::record_pending_mutation(state_dir, snapshot) {
        Ok(marked) => {
            if !policy.should_trigger() {
                return AutoSyncNotificationResult::PendingRecorded {
                    generation: marked.generation,
                };
            }
            match schedule_after_record(state_dir, &marked) {
                SpawnResult::Spawned => AutoSyncNotificationResult::Scheduled {
                    generation: marked.generation,
                },
                SpawnResult::Suppressed => AutoSyncNotificationResult::Suppressed,
                SpawnResult::SpawnFailed => AutoSyncNotificationResult::SchedulingFailed {
                    generation: Some(marked.generation),
                },
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to record auto-sync pending generation");
            apply_scheduling_failure_policy(policy);
            AutoSyncNotificationResult::SchedulingFailed { generation: None }
        }
    }
}

fn schedule_after_record(state_dir: &std::path::Path, _marked: &PendingState) -> SpawnResult {
    let settings = crate::config::get_sync_settings();
    let policy = AutoSyncPolicy::resolve(&settings);
    match crate::auto_sync::schedule::schedule_and_spawn(
        state_dir,
        &policy,
        crate::auto_sync::schedule::Caller::Mutation,
    ) {
        crate::auto_sync::schedule::ScheduleDecision::SpawnNow => SpawnResult::Spawned,
        _ => SpawnResult::Suppressed,
    }
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

/// Simplified tag for the active subcommand, used for startup recovery
/// classification. The binary crate maps its `Commands` enum to this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubcommandTag {
    /// Default (no subcommand) or mutation-producing commands — allow recovery.
    Mutation,
    /// `sync` — about to sync itself.
    Sync,
    /// `cron` — scheduled sync path.
    Cron,
    /// `register` — account setup.
    Register,
    /// Internal worker subprocess.
    AutoSyncWorker,
    /// Internal executor subprocess.
    AutoSyncExecute,
}

/// Returns true if the given command should attempt auto-sync recovery
/// at startup. Commands that are themselves about to sync, modify sync
/// policy, or are internal worker/executor subcommands should NOT
/// trigger recovery.
pub fn should_attempt_auto_sync_recovery(tag: Option<SubcommandTag>) -> bool {
    matches!(tag, None | Some(SubcommandTag::Mutation))
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
        let dir = tempfile::TempDir::new().unwrap();
        let policy = AutoSyncPolicy::default();
        let result = notify_local_mutation_with_dir(
            &policy,
            MutationContext {
                kind: MutationKind::SnippetCreate,
                origin: MutationOrigin::User,
                library_id: None,
            },
            dir.path(),
        );
        assert_eq!(result, AutoSyncNotificationResult::Disabled);
    }

    #[test]
    fn test_sync_merge_origin_returns_suppressed() {
        let policy = AutoSyncPolicy {
            sync_configured: true,
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

    #[test]
    fn test_recovery_allowed_for_none_command() {
        assert!(should_attempt_auto_sync_recovery(None));
    }

    #[test]
    fn test_recovery_allowed_for_mutation_commands() {
        assert!(should_attempt_auto_sync_recovery(Some(
            SubcommandTag::Mutation
        )));
    }

    #[test]
    fn test_recovery_blocked_for_sync() {
        assert!(!should_attempt_auto_sync_recovery(Some(
            SubcommandTag::Sync
        )));
    }

    #[test]
    fn test_recovery_blocked_for_cron() {
        assert!(!should_attempt_auto_sync_recovery(Some(
            SubcommandTag::Cron
        )));
    }

    #[test]
    fn test_recovery_blocked_for_register() {
        assert!(!should_attempt_auto_sync_recovery(Some(
            SubcommandTag::Register
        )));
    }

    #[test]
    fn test_recovery_blocked_for_auto_sync_worker() {
        assert!(!should_attempt_auto_sync_recovery(Some(
            SubcommandTag::AutoSyncWorker
        )));
    }

    #[test]
    fn test_recovery_blocked_for_auto_sync_execute() {
        assert!(!should_attempt_auto_sync_recovery(Some(
            SubcommandTag::AutoSyncExecute
        )));
    }

    #[test]
    fn test_subcommand_tag_equality() {
        assert_eq!(SubcommandTag::Mutation, SubcommandTag::Mutation);
        assert_ne!(SubcommandTag::Sync, SubcommandTag::Cron);
    }

    #[test]
    fn test_subcommand_tag_debug() {
        let debug = format!("{:?}", SubcommandTag::AutoSyncWorker);
        assert_eq!(debug, "AutoSyncWorker");
    }

    #[test]
    fn test_sync_configured_but_auto_sync_disabled_records_pending() {
        let dir = tempfile::TempDir::new().unwrap();
        let policy = AutoSyncPolicy {
            sync_configured: true,
            enabled: false,
            ..AutoSyncPolicy::default()
        };
        let result = notify_local_mutation_with_dir(
            &policy,
            MutationContext {
                kind: MutationKind::SnippetCreate,
                origin: MutationOrigin::User,
                library_id: None,
            },
            dir.path(),
        );
        match &result {
            AutoSyncNotificationResult::PendingRecorded { generation } => {
                assert!(*generation >= 1, "generation must be at least 1");
            }
            other => panic!("expected PendingRecorded, got {other:?}"),
        }
        // The pending marker must exist on disk.
        let pending_path = dir.path().join("auto-sync-pending.toml");
        assert!(
            pending_path.exists(),
            "pending marker must exist on disk when sync is configured but auto_sync is disabled"
        );
    }
}
