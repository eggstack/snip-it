//! Effective auto-sync policy resolved from persisted configuration.

use crate::config::{AutoSyncFailureMode, DEFAULT_SYNC_TIMEOUT_SECS, SyncSettings};
use std::time::Duration;

pub const MAX_DEBOUNCE_SECS: u64 = 300;
pub const DEFAULT_WORKER_LIFETIME_SECS: u64 = 300;
pub const DEFAULT_TERMINATION_GRACE_SECS: u64 = 2;

#[derive(Debug, Clone)]
pub struct AutoSyncPolicy {
    /// Whether a sync account is configured (`settings.enabled`).
    pub sync_configured: bool,
    /// Whether auto-sync is actively running (`settings.auto_sync && settings.enabled`).
    pub enabled: bool,
    pub debounce: Duration,
    pub failure_mode: AutoSyncFailureMode,
    pub sync_timeout: Duration,
    pub max_delay: Duration,
    /// Time after SIGTERM before escalating to SIGKILL.
    pub termination_grace: Duration,
    /// Maximum time the worker stays alive before exiting.
    pub worker_lifetime: Duration,
}

impl AutoSyncPolicy {
    pub fn resolve(settings: &SyncSettings) -> Self {
        Self {
            sync_configured: settings.enabled,
            enabled: settings.auto_sync && settings.enabled,
            debounce: settings.auto_sync_debounce(),
            failure_mode: settings.auto_sync_failure.clone(),
            sync_timeout: settings.auto_sync_timeout(),
            max_delay: settings.auto_sync_max_delay(),
            termination_grace: Duration::from_secs(DEFAULT_TERMINATION_GRACE_SECS),
            worker_lifetime: Duration::from_secs(DEFAULT_WORKER_LIFETIME_SECS),
        }
    }

    pub fn should_trigger(&self) -> bool {
        self.enabled
    }
}

impl Default for AutoSyncPolicy {
    fn default() -> Self {
        Self {
            sync_configured: false,
            enabled: false,
            debounce: Duration::from_secs(2),
            failure_mode: AutoSyncFailureMode::Warn,
            sync_timeout: Duration::from_secs(DEFAULT_SYNC_TIMEOUT_SECS),
            max_delay: Duration::from_secs(300),
            termination_grace: Duration::from_secs(DEFAULT_TERMINATION_GRACE_SECS),
            worker_lifetime: Duration::from_secs(DEFAULT_WORKER_LIFETIME_SECS),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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
#[non_exhaustive]
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

/// Typed failure classification for sync operations.
///
/// Each variant represents a distinct operational failure mode with
/// specific retry and operator-attention semantics. The taxonomy is
/// used by the backoff calculator, status persistence, and schedule
/// decision function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FailureClass {
    /// Auto-sync is disabled in configuration.
    DeferredDisabled,
    /// Sync is not configured (no server URL, no API key).
    DeferredNotConfigured,
    /// Transient network connectivity failure (DNS, connection refused, unreachable).
    TransientNetwork,
    /// Transient timeout (server did not respond in time).
    TransientTimeout,
    /// Authentication or credential failure (bad API key, unregistered).
    Authentication,
    /// Configuration error that requires operator intervention.
    Configuration,
    /// Synchronization conflict (merge failure, partial sync).
    Conflict,
    /// Partial sync completion (some libraries succeeded, some failed).
    Partial,
    /// Local persistence failure (could not write library files).
    LocalPersistence,
    /// Credential storage (keychain) failure.
    CredentialStore,
    /// Internal or genuinely unclassified error.
    Internal,
}

impl FailureClass {
    /// Classify a `SnipError` into a failure class.
    ///
    /// For `SyncFailure` variants, uses direct variant matching (no string analysis).
    /// For `Runtime` variants, falls back to heuristic matching on the error message
    /// since `SnipError::Runtime` does not carry typed error categories.
    pub fn from_error(err: &crate::error::SnipError) -> Self {
        use crate::error::{SnipError, SyncFailureKind};
        match err {
            SnipError::SyncFailure { kind, .. } => match kind {
                SyncFailureKind::NotConfigured => FailureClass::DeferredNotConfigured,
                SyncFailureKind::ConnectFailed => FailureClass::TransientNetwork,
                SyncFailureKind::HealthCheckFailed => FailureClass::TransientNetwork,
                SyncFailureKind::AuthenticationFailed => FailureClass::Authentication,
                SyncFailureKind::SyncRequestFailed => FailureClass::TransientNetwork,
                SyncFailureKind::CreateLibraryFailed => FailureClass::Configuration,
                SyncFailureKind::GetPremadeLibraryFailed => FailureClass::TransientNetwork,
                SyncFailureKind::RegistrationFailed => FailureClass::Authentication,
                SyncFailureKind::LibraryManagerInitFailed => FailureClass::LocalPersistence,
                SyncFailureKind::LibraryModeInitFailed => FailureClass::LocalPersistence,
                SyncFailureKind::LibrariesDirReadFailed => FailureClass::LocalPersistence,
                SyncFailureKind::NoLibrariesToSync => FailureClass::Internal,
                SyncFailureKind::SaveMergedLibraryFailed => FailureClass::LocalPersistence,
                SyncFailureKind::PartialSyncFailure => FailureClass::Partial,
                SyncFailureKind::PremadePartialFailure => FailureClass::Partial,
                SyncFailureKind::EncryptionFailed => FailureClass::Internal,
                SyncFailureKind::DecryptionFailed => FailureClass::Internal,
            },
            SnipError::Runtime { message, detail } => {
                let combined = format!("{message} {}", detail.as_deref().unwrap_or(""));
                let lower = combined.to_lowercase();
                if lower.contains("not configured") || lower.contains("sync not enabled") {
                    FailureClass::DeferredNotConfigured
                } else if lower.contains("api key")
                    || lower.contains("auth")
                    || lower.contains("unauthorized")
                    || lower.contains("forbidden")
                    || lower.contains("permission denied")
                {
                    FailureClass::Authentication
                } else if lower.contains("health check")
                    || lower.contains("server")
                    || lower.contains("network")
                    || lower.contains("connection")
                    || lower.contains("dns")
                    || lower.contains("connection refused")
                    || lower.contains("connect")
                    || lower.contains("unavailable")
                    || lower.contains("unreachable")
                {
                    FailureClass::TransientNetwork
                } else if lower.contains("timeout") || lower.contains("timed out") {
                    FailureClass::TransientTimeout
                } else if lower.contains("failed to save")
                    || lower.contains("failed to read")
                    || lower.contains("failed to initialize")
                    || lower.contains("failed to create")
                    || lower.contains("i/o")
                    || lower.contains("permission")
                {
                    FailureClass::LocalPersistence
                } else if lower.contains("conflict") || lower.contains("merge") {
                    FailureClass::Conflict
                } else if lower.contains("failed to sync")
                    || lower.contains("some libraries")
                    || lower.contains("skipped")
                {
                    FailureClass::Partial
                } else if lower.contains("credential") || lower.contains("keychain") {
                    FailureClass::CredentialStore
                } else {
                    FailureClass::Internal
                }
            }
            SnipError::Io { operation, .. } => {
                let lower = operation.to_lowercase();
                if lower.contains("connection")
                    || lower.contains("connect")
                    || lower.contains("network")
                {
                    FailureClass::TransientNetwork
                } else {
                    FailureClass::LocalPersistence
                }
            }
            SnipError::Toml { .. } => FailureClass::LocalPersistence,
            _ => FailureClass::Internal,
        }
    }

    /// Serialize to a stable short code for persistence.
    pub fn as_code(&self) -> &'static str {
        match self {
            Self::DeferredDisabled => "deferred_disabled",
            Self::DeferredNotConfigured => "deferred_not_configured",
            Self::TransientNetwork => "transient_network",
            Self::TransientTimeout => "transient_timeout",
            Self::Authentication => "authentication",
            Self::Configuration => "configuration",
            Self::Conflict => "conflict",
            Self::Partial => "partial",
            Self::LocalPersistence => "local_persistence",
            Self::CredentialStore => "credential_store",
            Self::Internal => "internal",
        }
    }

    /// Deserialize from a stable short code.
    pub fn from_code(code: &str) -> Self {
        match code {
            "deferred_disabled" => Self::DeferredDisabled,
            "deferred_not_configured" => Self::DeferredNotConfigured,
            "transient_network" => Self::TransientNetwork,
            "transient_timeout" => Self::TransientTimeout,
            "authentication" => Self::Authentication,
            "configuration" => Self::Configuration,
            "conflict" => Self::Conflict,
            "partial" => Self::Partial,
            "local_persistence" => Self::LocalPersistence,
            "credential_store" => Self::CredentialStore,
            "internal" => Self::Internal,
            _ => Self::Internal,
        }
    }

    /// Whether this failure class allows automatic retry.
    pub fn allows_automatic_retry(&self) -> bool {
        matches!(
            self,
            Self::TransientNetwork | Self::TransientTimeout | Self::Internal
        )
    }

    /// Whether this failure class is deferred (waiting for config change).
    pub fn is_deferred(&self) -> bool {
        matches!(
            self,
            Self::DeferredDisabled
                | Self::DeferredNotConfigured
                | Self::Configuration
                | Self::Authentication
                | Self::CredentialStore
        )
    }
}

/// Retry disposition derived from a failure class.
///
/// Determines what the scheduling system should do after a failure:
/// retry after a delay, wait for configuration change, require operator
/// attention, or not retry at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetryDisposition {
    /// Retry after the given duration (exponential backoff).
    RetryAfter(Duration),
    /// Do not retry until a relevant configuration change is detected.
    WaitForConfigurationChange,
    /// Requires operator attention; do not retry automatically.
    RequiresAttention,
    /// Do not retry automatically; only explicit `snp sync` can retry.
    NoAutomaticRetry,
}

impl FailureClass {
    /// Map this failure class to a retry disposition.
    ///
    /// The backoff calculator uses this to determine the delay before
    /// the next attempt. `consecutive_failures` is used for exponential
    /// backoff calculation in the `RetryAfter` case.
    pub fn retry_disposition(&self, consecutive_failures: u32) -> RetryDisposition {
        match self {
            Self::DeferredDisabled | Self::DeferredNotConfigured => {
                RetryDisposition::WaitForConfigurationChange
            }
            Self::TransientNetwork | Self::TransientTimeout => {
                RetryDisposition::RetryAfter(transient_backoff(consecutive_failures))
            }
            Self::Authentication | Self::Configuration | Self::CredentialStore => {
                RetryDisposition::RequiresAttention
            }
            Self::Conflict | Self::Partial => RetryDisposition::RequiresAttention,
            Self::LocalPersistence => RetryDisposition::RequiresAttention,
            Self::Internal => {
                if consecutive_failures < 3 {
                    RetryDisposition::RetryAfter(transient_backoff(consecutive_failures))
                } else {
                    RetryDisposition::RequiresAttention
                }
            }
        }
    }
}

/// Compute exponential backoff duration for transient failures.
///
/// Schedule (for `consecutive_failures` count after recording):
/// | Count | Base delay |
/// |-------|------------|
/// | 1     | 5s         |
/// | 2     | 15s        |
/// | 3     | 30s        |
/// | 4     | 60s        |
/// | 5+    | exponential, capped at 15 minutes |
///
/// Includes bounded jitter (0-20% of base delay) to avoid synchronized retries.
pub fn transient_backoff(consecutive_failures: u32) -> Duration {
    let base_secs: u64 = match consecutive_failures {
        0 => 5,
        1 => 5,
        2 => 15,
        3 => 30,
        4 => 60,
        n => {
            let exp = n.saturating_sub(3) as u64;
            60u64
                .saturating_mul(2u64.saturating_pow(exp as u32))
                .min(900)
        }
    };

    // Bounded jitter: 0-20% of base delay
    let jitter_max = base_secs / 5;
    let jitter = if jitter_max > 0 {
        // Use a simple deterministic-ish jitter based on failure count
        (consecutive_failures as u64 * 7 + 13) % (jitter_max + 1)
    } else {
        0
    };

    Duration::from_secs(base_secs.saturating_add(jitter))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_disabled_by_default() {
        let settings = crate::config::SyncSettings::default();
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(!policy.sync_configured);
        assert!(!policy.enabled);
        assert!(!policy.should_trigger());
    }

    #[test]
    fn test_policy_enabled_requires_sync_enabled() {
        let mut settings = crate::config::SyncSettings::default();
        settings.enabled = false;
        settings.auto_sync = true;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(!policy.sync_configured);
        assert!(!policy.enabled);

        settings.enabled = true;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(policy.sync_configured);
        assert!(policy.enabled);
        assert!(policy.should_trigger());
    }

    #[test]
    fn test_sync_configured_without_auto_sync() {
        let mut settings = crate::config::SyncSettings::default();
        settings.enabled = true;
        settings.auto_sync = false;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(policy.sync_configured);
        assert!(!policy.enabled);
        assert!(!policy.should_trigger());
    }

    #[test]
    fn test_policy_debounce_clamped() {
        let mut settings = crate::config::SyncSettings::default();
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
        let mut settings = crate::config::SyncSettings::default();
        settings.enabled = true;
        settings.auto_sync = true;
        settings.auto_sync_failure = crate::config::AutoSyncFailureMode::Ignore;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(
            policy.failure_mode,
            crate::config::AutoSyncFailureMode::Ignore
        );

        settings.auto_sync_failure = crate::config::AutoSyncFailureMode::Error;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert_eq!(
            policy.failure_mode,
            crate::config::AutoSyncFailureMode::Error
        );
    }

    #[test]
    fn test_default_policy_is_disabled() {
        let policy = AutoSyncPolicy::default();
        assert!(!policy.sync_configured);
        assert!(!policy.enabled);
        assert_eq!(policy.debounce, Duration::from_secs(2));
        assert_eq!(
            policy.failure_mode,
            crate::config::AutoSyncFailureMode::Warn
        );
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
    fn test_failure_class_code_roundtrip() {
        for class in [
            FailureClass::DeferredDisabled,
            FailureClass::DeferredNotConfigured,
            FailureClass::TransientNetwork,
            FailureClass::TransientTimeout,
            FailureClass::Authentication,
            FailureClass::Configuration,
            FailureClass::Conflict,
            FailureClass::Partial,
            FailureClass::LocalPersistence,
            FailureClass::CredentialStore,
            FailureClass::Internal,
        ] {
            assert_eq!(FailureClass::from_code(class.as_code()), class);
        }
    }

    #[test]
    fn test_failure_class_allows_automatic_retry() {
        assert!(FailureClass::TransientNetwork.allows_automatic_retry());
        assert!(FailureClass::TransientTimeout.allows_automatic_retry());
        assert!(FailureClass::Internal.allows_automatic_retry());
        assert!(!FailureClass::Authentication.allows_automatic_retry());
        assert!(!FailureClass::Conflict.allows_automatic_retry());
        assert!(!FailureClass::LocalPersistence.allows_automatic_retry());
        assert!(!FailureClass::DeferredDisabled.allows_automatic_retry());
    }

    #[test]
    fn test_failure_class_is_deferred() {
        assert!(FailureClass::DeferredDisabled.is_deferred());
        assert!(FailureClass::DeferredNotConfigured.is_deferred());
        assert!(FailureClass::Configuration.is_deferred());
        assert!(FailureClass::Authentication.is_deferred());
        assert!(FailureClass::CredentialStore.is_deferred());
        assert!(!FailureClass::TransientNetwork.is_deferred());
        assert!(!FailureClass::Conflict.is_deferred());
    }

    // ── Table-driven classification tests (SyncFailure variants) ────

    #[test]
    fn test_classify_sync_failure_not_configured() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::NotConfigured,
            None,
        );
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::DeferredNotConfigured
        );
    }

    #[test]
    fn test_classify_sync_failure_connect_failed() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::ConnectFailed,
            Some("connection refused"),
        );
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::TransientNetwork
        );
    }

    #[test]
    fn test_classify_sync_failure_health_check() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::HealthCheckFailed,
            None,
        );
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::TransientNetwork
        );
    }

    #[test]
    fn test_classify_sync_failure_auth() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::AuthenticationFailed,
            Some("unauthorized"),
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Authentication);
    }

    #[test]
    fn test_classify_sync_failure_sync_request() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::SyncRequestFailed,
            Some("tonic status: cancelled"),
        );
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::TransientNetwork
        );
    }

    #[test]
    fn test_classify_sync_failure_create_library() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::CreateLibraryFailed,
            Some("already exists"),
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Configuration);
    }

    #[test]
    fn test_classify_sync_failure_save_library() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::SaveMergedLibraryFailed,
            Some("disk full"),
        );
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::LocalPersistence
        );
    }

    #[test]
    fn test_classify_sync_failure_partial() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::PartialSyncFailure,
            None,
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Partial);
    }

    #[test]
    fn test_classify_sync_failure_registration() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::RegistrationFailed,
            Some("device limit reached"),
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Authentication);
    }

    #[test]
    fn test_classify_sync_failure_encryption() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::EncryptionFailed,
            None,
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Internal);
    }

    // ── Table-driven classification tests (legacy Runtime variants) ──

    #[test]
    fn test_classify_not_configured() {
        let err = crate::error::SnipError::runtime_error("Sync not configured", None);
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::DeferredNotConfigured
        );
    }

    #[test]
    fn test_classify_sync_disabled() {
        let err = crate::error::SnipError::runtime_error("sync not enabled", None);
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::DeferredNotConfigured
        );
    }

    #[test]
    fn test_classify_api_key() {
        let err = crate::error::SnipError::runtime_error(
            "Sync is enabled but no API key configured",
            None,
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Authentication);
    }

    #[test]
    fn test_classify_health_check() {
        let err = crate::error::SnipError::runtime_error("Server health check failed", None);
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::TransientNetwork
        );
    }

    #[test]
    fn test_classify_server_unreachable() {
        let err =
            crate::error::SnipError::runtime_error("Server is not reachable", Some("timeout"));
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::TransientNetwork
        );
    }

    #[test]
    fn test_classify_network() {
        let err = crate::error::SnipError::runtime_error("network error", None);
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::TransientNetwork
        );
    }

    #[test]
    fn test_classify_timeout() {
        let err = crate::error::SnipError::runtime_error("request timed out", None);
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::TransientTimeout
        );
    }

    #[test]
    fn test_classify_partial_failure() {
        let err = crate::error::SnipError::runtime_error("Some libraries failed to sync", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Partial);
    }

    #[test]
    fn test_classify_conflict() {
        let err = crate::error::SnipError::runtime_error("merge conflict detected", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Conflict);
    }

    #[test]
    fn test_classify_library_manager() {
        let err =
            crate::error::SnipError::runtime_error("Failed to initialize library manager", None);
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::LocalPersistence
        );
    }

    #[test]
    fn test_classify_save() {
        let err = crate::error::SnipError::runtime_error("Failed to save merged library", None);
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::LocalPersistence
        );
    }

    #[test]
    fn test_classify_unknown_runtime() {
        let err = crate::error::SnipError::runtime_error("something went wrong", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Internal);
    }

    #[test]
    fn test_classify_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let err: crate::error::SnipError = io_err.into();
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::LocalPersistence
        );
    }

    #[test]
    fn test_classify_toml() {
        let toml_err = toml::from_str::<toml::Value>("invalid = [toml").unwrap_err();
        let err = crate::error::SnipError::toml_error("parse config", toml_err);
        assert_eq!(
            FailureClass::from_error(&err),
            FailureClass::LocalPersistence
        );
    }

    // ── Retry disposition tests ────────────────────────────────────

    #[test]
    fn test_retry_disposition_deferred_disabled() {
        let disp = FailureClass::DeferredDisabled.retry_disposition(0);
        assert_eq!(disp, RetryDisposition::WaitForConfigurationChange);
    }

    #[test]
    fn test_retry_disposition_deferred_not_configured() {
        let disp = FailureClass::DeferredNotConfigured.retry_disposition(0);
        assert_eq!(disp, RetryDisposition::WaitForConfigurationChange);
    }

    #[test]
    fn test_retry_disposition_transient_network() {
        let disp = FailureClass::TransientNetwork.retry_disposition(0);
        assert!(matches!(disp, RetryDisposition::RetryAfter(_)));
    }

    #[test]
    fn test_retry_disposition_authentication() {
        let disp = FailureClass::Authentication.retry_disposition(0);
        assert_eq!(disp, RetryDisposition::RequiresAttention);
    }

    #[test]
    fn test_retry_disposition_internal_bounded_retry() {
        // First 2 failures get RetryAfter
        let d0 = FailureClass::Internal.retry_disposition(0);
        assert!(matches!(d0, RetryDisposition::RetryAfter(_)));
        let d1 = FailureClass::Internal.retry_disposition(1);
        assert!(matches!(d1, RetryDisposition::RetryAfter(_)));
        let d2 = FailureClass::Internal.retry_disposition(2);
        assert!(matches!(d2, RetryDisposition::RetryAfter(_)));
        // 3rd failure gets RequiresAttention
        let d3 = FailureClass::Internal.retry_disposition(3);
        assert_eq!(d3, RetryDisposition::RequiresAttention);
    }

    // ── Backoff progression tests ──────────────────────────────────

    #[test]
    fn test_transient_backoff_progression() {
        let d0 = transient_backoff(0);
        let d1 = transient_backoff(1);
        let d2 = transient_backoff(2);
        let d3 = transient_backoff(3);
        // Each should be >= the previous (ignoring jitter)
        assert!(d1 >= d0 - Duration::from_secs(2), "d1 should be >= d0");
        assert!(d2 >= d1 - Duration::from_secs(2), "d2 should be >= d1");
        assert!(d3 >= d2 - Duration::from_secs(2), "d3 should be >= d2");
    }

    #[test]
    fn test_transient_backoff_cap() {
        // Even with very high failure count, should not exceed 15 minutes + jitter
        let d = transient_backoff(100);
        assert!(d <= Duration::from_secs(900 + 180)); // 15min + 20% jitter
    }

    #[test]
    fn test_transient_backoff_nonzero() {
        for i in 0..10 {
            assert!(
                !transient_backoff(i).is_zero(),
                "backoff at {i} must be nonzero"
            );
        }
    }

    #[test]
    fn test_re_enable_auto_sync_preserves_pending_intent() {
        let mut settings = crate::config::SyncSettings::default();
        settings.enabled = true;
        settings.auto_sync = false;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(
            policy.sync_configured,
            "sync_configured must remain true when auto_sync is disabled but sync is enabled"
        );

        settings.auto_sync = true;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(
            policy.sync_configured,
            "sync_configured must remain true after re-enabling auto_sync"
        );
        assert!(
            policy.enabled,
            "enabled must be true after re-enabling auto_sync"
        );
    }

    #[test]
    fn test_manual_sync_works_while_auto_sync_disabled() {
        let mut settings = crate::config::SyncSettings::default();
        settings.enabled = true;
        settings.auto_sync = false;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(
            policy.sync_configured,
            "sync_configured must be true so manual sync can use it"
        );
        assert!(
            !policy.enabled,
            "enabled must be false when auto_sync is disabled"
        );
        assert!(
            !policy.should_trigger(),
            "should_trigger must be false when auto_sync is disabled"
        );
    }

    #[test]
    fn test_malformed_settings_result_in_failure_not_disable() {
        let mut settings = crate::config::SyncSettings::default();
        settings.enabled = true;
        settings.auto_sync = true;
        settings.auto_sync_debounce_seconds = u64::MAX;
        let policy = AutoSyncPolicy::resolve(&settings);
        assert!(
            policy.sync_configured,
            "sync_configured must remain true despite malformed debounce"
        );
        assert!(
            policy.enabled,
            "enabled must remain true despite malformed debounce"
        );
        assert_eq!(
            policy.debounce,
            std::time::Duration::from_secs(MAX_DEBOUNCE_SECS),
            "debounce must be clamped to MAX"
        );
        assert!(
            !policy.sync_timeout.is_zero(),
            "sync_timeout must be non-zero despite malformed debounce"
        );
    }
}
