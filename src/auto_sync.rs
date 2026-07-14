//! Auto-sync policy model and mutation classification.
//!
//! Defines the effective policy resolved once per command invocation
//! and the mutation classification contract that determines which
//! operations may trigger post-mutation synchronization.

use crate::config::{AutoSyncFailureMode, SyncSettings};
use std::time::Duration;

/// Effective auto-sync policy resolved from configuration.
///
/// This is computed once per command invocation and carries validated,
/// clamped values. A disabled policy produces no scheduling request.
#[derive(Debug, Clone)]
pub struct AutoSyncPolicy {
    /// Whether auto-sync is enabled.
    pub enabled: bool,
    /// Debounce delay before firing after a mutation.
    pub debounce: Duration,
    /// Failure behavior when auto-sync cannot complete.
    pub failure_mode: AutoSyncFailureMode,
}

impl AutoSyncPolicy {
    /// Resolve the effective policy from persisted settings.
    ///
    /// Disabled (`auto_sync: false`) produces a safe no-op policy.
    /// Invalid configuration values are clamped to valid ranges.
    pub fn resolve(settings: &SyncSettings) -> Self {
        Self {
            enabled: settings.auto_sync && settings.enabled,
            debounce: settings.auto_sync_debounce(),
            failure_mode: settings.auto_sync_failure.clone(),
        }
    }

    /// Returns `true` if auto-sync should be triggered for a mutation.
    pub fn should_trigger(&self) -> bool {
        self.enabled
    }
}

impl Default for AutoSyncPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            debounce: Duration::from_secs(2),
            failure_mode: AutoSyncFailureMode::Warn,
        }
    }
}

/// Classification of library-mutating operations.
///
/// Each variant identifies one logical class of mutation that may
/// trigger post-mutation auto-sync. This enum does NOT gate the
/// trigger — that is the policy's job — but it records the reason
/// for the sync request.
///
/// ## Trigger matrix
///
/// | Kind              | Mutates syncable content? | Triggers auto-sync? |
/// |-------------------|--------------------------|---------------------|
/// | SnippetCreate     | Yes                      | Yes (when enabled)  |
/// | SnippetUpdate     | Yes                      | Yes (when enabled)  |
/// | SnippetDelete     | Yes (tombstone)          | Yes (when enabled)  |
/// | Import            | Yes (bulk)               | Yes (once)          |
/// | LibraryChange     | Depends on scope         | Only if remote mapped |
/// | PremadeInstall    | Yes (bulk)               | Yes (once)          |
/// | SyncConflictWrite | Yes                      | Yes (once)          |
/// | AccountConfig     | No                       | Never               |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationKind {
    /// A new snippet was created.
    SnippetCreate,
    /// An existing snippet's command, description, tags, or output was modified.
    SnippetUpdate,
    /// A snippet was soft-deleted (tombstone).
    SnippetDelete,
    /// Bulk import (create/merge/replace) was performed.
    Import,
    /// A library was created, renamed, or deleted.
    LibraryChange,
    /// A premade library was downloaded or installed.
    PremadeInstall,
    /// A sync conflict resolution wrote local state.
    SyncConflictWrite,
    /// Account or configuration changes — never triggers sync.
    AccountConfig,
}

impl MutationKind {
    /// Returns `true` if this mutation kind mutates syncable library content.
    pub fn is_syncable_mutation(&self) -> bool {
        !matches!(self, Self::AccountConfig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SyncSettings;

    #[test]
    fn test_policy_disabled_by_default() {
        let settings = SyncSettings::default();
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(!policy.enabled);
        assert!(!policy.should_trigger());
    }

    #[test]
    fn test_policy_enabled_requires_sync_enabled() {
        let mut settings = SyncSettings::default();
        settings.enabled = false;
        settings.auto_sync = true;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(!policy.enabled);

        settings.enabled = true;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(policy.enabled);
        assert!(policy.should_trigger());
    }

    #[test]
    fn test_policy_debounce_clamped() {
        let mut settings = SyncSettings::default();
        settings.enabled = true;
        settings.auto_sync = true;

        settings.auto_sync_debounce_seconds = 0;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.debounce, Duration::from_secs(0));

        settings.auto_sync_debounce_seconds = 2;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.debounce, Duration::from_secs(2));

        settings.auto_sync_debounce_seconds = 300;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.debounce, Duration::from_secs(300));

        // Overflow clamped to max
        settings.auto_sync_debounce_seconds = u64::MAX;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.debounce, Duration::from_secs(300));
    }

    #[test]
    fn test_policy_failure_mode() {
        let mut settings = SyncSettings::default();
        settings.enabled = true;
        settings.auto_sync = true;

        settings.auto_sync_failure = AutoSyncFailureMode::Ignore;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.failure_mode, AutoSyncFailureMode::Ignore);

        settings.auto_sync_failure = AutoSyncFailureMode::Error;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.failure_mode, AutoSyncFailureMode::Error);
    }

    #[test]
    fn test_mutation_kind_syncable() {
        assert!(MutationKind::SnippetCreate.is_syncable_mutation());
        assert!(MutationKind::SnippetUpdate.is_syncable_mutation());
        assert!(MutationKind::SnippetDelete.is_syncable_mutation());
        assert!(MutationKind::Import.is_syncable_mutation());
        assert!(MutationKind::LibraryChange.is_syncable_mutation());
        assert!(MutationKind::PremadeInstall.is_syncable_mutation());
        assert!(MutationKind::SyncConflictWrite.is_syncable_mutation());
        assert!(!MutationKind::AccountConfig.is_syncable_mutation());
    }

    #[test]
    fn test_default_policy_is_disabled() {
        let policy = AutoSyncPolicy::default();
        assert!(!policy.enabled);
        assert_eq!(policy.debounce, Duration::from_secs(2));
        assert_eq!(policy.failure_mode, AutoSyncFailureMode::Warn);
    }
}
