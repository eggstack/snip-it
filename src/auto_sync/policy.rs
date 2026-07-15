//! Effective auto-sync policy resolved from persisted configuration.

use crate::config::{AutoSyncFailureMode, SyncSettings};
use std::time::Duration;

pub const MAX_DEBOUNCE_SECS: u64 = 300;
pub const DEFAULT_SYNC_TIMEOUT_SECS: u64 = 30;
pub const MAX_SYNC_TIMEOUT_SECS: u64 = 120;
pub const DEFAULT_MAX_RETRIES: u32 = 1;
pub const WORKER_MAX_LIFETIME_SECS: u64 = 300;

#[derive(Debug, Clone)]
pub struct AutoSyncPolicy {
    pub enabled: bool,
    pub debounce: Duration,
    pub failure_mode: AutoSyncFailureMode,
    pub max_retries: u32,
    pub sync_timeout: Duration,
}

impl AutoSyncPolicy {
    pub fn resolve(settings: &SyncSettings) -> Self {
        Self {
            enabled: settings.auto_sync && settings.enabled,
            debounce: settings.auto_sync_debounce(),
            failure_mode: settings.auto_sync_failure.clone(),
            max_retries: DEFAULT_MAX_RETRIES,
            sync_timeout: Duration::from_secs(
                settings
                    .auto_sync_debounce_seconds
                    .clamp(1, MAX_SYNC_TIMEOUT_SECS),
            ),
        }
    }

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
            max_retries: DEFAULT_MAX_RETRIES,
            sync_timeout: Duration::from_secs(DEFAULT_SYNC_TIMEOUT_SECS),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationKind {
    SnippetCreate,
    SnippetUpdate,
    SnippetDelete,
    Import,
    LibraryChange,
    PremadeInstall,
    SyncConflictWrite,
    AccountConfig,
}

impl MutationKind {
    pub fn is_syncable_mutation(&self) -> bool {
        !matches!(self, Self::AccountConfig)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationOrigin {
    User,
    Import,
    SyncMerge,
    Recovery,
}

impl MutationOrigin {
    pub fn should_suppress(self) -> bool {
        matches!(self, Self::SyncMerge)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureClass {
    Network,
    Auth,
    Conflict,
    Unknown,
}

impl FailureClass {
    pub fn from_error(err: &crate::error::SnipError) -> Self {
        use crate::error::SnipError;
        match err {
            SnipError::Runtime { message, detail } => {
                let combined = format!("{message} {}", detail.as_deref().unwrap_or(""));
                let lower = combined.to_lowercase();
                if lower.contains("network")
                    || lower.contains("timeout")
                    || lower.contains("dns")
                    || lower.contains("connection refused")
                    || lower.contains("connect")
                    || lower.contains("unavailable")
                {
                    FailureClass::Network
                } else if lower.contains("auth")
                    || lower.contains("unauthorized")
                    || lower.contains("forbidden")
                    || lower.contains("api key")
                    || lower.contains("permission denied")
                {
                    FailureClass::Auth
                } else if lower.contains("conflict") || lower.contains("merge") {
                    FailureClass::Conflict
                } else {
                    FailureClass::Unknown
                }
            }
            SnipError::Io { operation, .. } => {
                let lower = operation.to_lowercase();
                if lower.contains("connection")
                    || lower.contains("connect")
                    || lower.contains("network")
                {
                    FailureClass::Network
                } else {
                    FailureClass::Unknown
                }
            }
            _ => FailureClass::Unknown,
        }
    }

    pub fn as_code(&self) -> &'static str {
        match self {
            Self::Network => "network",
            Self::Auth => "auth",
            Self::Conflict => "conflict",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_code(code: &str) -> Self {
        match code {
            "network" => Self::Network,
            "auth" => Self::Auth,
            "conflict" => Self::Conflict,
            _ => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        settings.auto_sync_debounce_seconds = 300;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(policy.debounce, Duration::from_secs(300));

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
    fn test_default_policy_is_disabled() {
        let policy = AutoSyncPolicy::default();
        assert!(!policy.enabled);
        assert_eq!(policy.debounce, Duration::from_secs(2));
        assert_eq!(policy.failure_mode, AutoSyncFailureMode::Warn);
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
    fn test_origin_suppression() {
        assert!(MutationOrigin::SyncMerge.should_suppress());
        assert!(!MutationOrigin::User.should_suppress());
        assert!(!MutationOrigin::Import.should_suppress());
        assert!(!MutationOrigin::Recovery.should_suppress());
    }

    #[test]
    fn test_failure_class_network() {
        let err = crate::error::SnipError::runtime_error("connection timeout", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Network);
    }

    #[test]
    fn test_failure_class_auth() {
        let err = crate::error::SnipError::runtime_error("unauthorized access", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Auth);
    }

    #[test]
    fn test_failure_class_conflict() {
        let err = crate::error::SnipError::runtime_error("merge conflict", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Conflict);
    }

    #[test]
    fn test_failure_class_unknown() {
        let err = crate::error::SnipError::runtime_error("something broke", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Unknown);
    }

    #[test]
    fn test_failure_class_code_roundtrip() {
        for class in [
            FailureClass::Network,
            FailureClass::Auth,
            FailureClass::Conflict,
            FailureClass::Unknown,
        ] {
            assert_eq!(FailureClass::from_code(class.as_code()), class);
        }
    }
}
