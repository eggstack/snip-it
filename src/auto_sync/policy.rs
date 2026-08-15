//! Effective auto-sync policy resolved from persisted configuration.

use crate::config::{AutoSyncFailureMode, DEFAULT_SYNC_TIMEOUT_SECS, SyncSettings};
use std::time::Duration;

pub const MAX_DEBOUNCE_SECS: u64 = 300;

#[derive(Debug, Clone)]
pub struct AutoSyncPolicy {
    /// Whether a sync account is configured (`settings.enabled`).
    pub sync_configured: bool,
    /// Whether auto-sync is actively running (`settings.auto_sync && settings.enabled`).
    pub enabled: bool,
    pub debounce: Duration,
    pub failure_mode: AutoSyncFailureMode,
    pub sync_timeout: Duration,
    /// Maximum time the worker stays alive before exiting. This is the
    /// sole pre-sync debounce window — the worker exits when this
    /// expires regardless of how many debounce cycles have completed.
    pub max_lifetime: Duration,
}

/// Resolve configured sync direction with optional foreground CLI overrides.
pub fn effective_sync_direction(
    settings: &SyncSettings,
    cli_push_only: bool,
    cli_pull_only: bool,
) -> crate::config::SyncDirection {
    if cli_push_only {
        crate::config::SyncDirection::Push
    } else if cli_pull_only {
        crate::config::SyncDirection::Pull
    } else {
        settings.sync_direction.clone()
    }
}

impl AutoSyncPolicy {
    pub fn resolve(settings: &SyncSettings) -> Self {
        Self {
            sync_configured: settings.enabled,
            enabled: settings.auto_sync && settings.enabled,
            debounce: settings.auto_sync_debounce(),
            failure_mode: settings.auto_sync_failure.clone(),
            sync_timeout: settings.auto_sync_timeout(),
            max_lifetime: settings.auto_sync_max_delay(),
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
            max_lifetime: Duration::from_secs(300),
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
    SnippetRun,
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

// `FailureClass` is defined in the sync-client layer (`crate::sync_failure`) so
// that `src/sync.rs` can classify its own gRPC errors without depending upward
// on `crate::auto_sync`. The application layer re-exports it here unchanged.
pub use crate::sync_failure::FailureClass;

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
    pub fn retry_disposition(&self, consecutive_failures: u32) -> RetryDisposition {
        match self {
            Self::Configuration => RetryDisposition::WaitForConfigurationChange,
            Self::Transient => {
                RetryDisposition::RetryAfter(transient_backoff(consecutive_failures))
            }
            Self::LocalFailure => RetryDisposition::RequiresAttention,
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
        for (kind, expected) in [
            (MutationKind::SnippetCreate, true),
            (MutationKind::SnippetUpdate, true),
            (MutationKind::SnippetDelete, true),
            (MutationKind::Import, true),
            (MutationKind::LibraryChange, true),
            (MutationKind::PremadeInstall, true),
            (MutationKind::SyncConflictWrite, true),
            (MutationKind::AccountConfig, false),
        ] {
            assert_eq!(kind.is_syncable_mutation(), expected);
        }
    }

    #[test]
    fn test_origin_suppression() {
        for (origin, expected) in [
            (MutationOrigin::SyncMerge, true),
            (MutationOrigin::User, false),
            (MutationOrigin::Import, false),
            (MutationOrigin::Recovery, false),
        ] {
            assert_eq!(origin.should_suppress(), expected);
        }
    }

    #[test]
    fn test_failure_class_code_roundtrip() {
        for class in [
            FailureClass::Configuration,
            FailureClass::Configuration,
            FailureClass::Transient,
            FailureClass::Transient,
            FailureClass::Configuration,
            FailureClass::Configuration,
            FailureClass::LocalFailure,
            FailureClass::Transient,
            FailureClass::LocalFailure,
            FailureClass::Configuration,
            FailureClass::Internal,
        ] {
            assert_eq!(FailureClass::from_code(class.as_code()), class);
        }
    }

    #[test]
    fn test_failure_class_allows_automatic_retry() {
        for (class, expected) in [
            (FailureClass::Transient, true),
            (FailureClass::Internal, true),
            (FailureClass::Configuration, false),
            (FailureClass::LocalFailure, false),
        ] {
            assert_eq!(class.allows_automatic_retry(), expected);
        }
    }

    #[test]
    fn test_failure_class_is_deferred() {
        for (class, expected) in [
            (FailureClass::Configuration, true),
            (FailureClass::Transient, false),
            (FailureClass::LocalFailure, false),
            (FailureClass::Internal, false),
        ] {
            assert_eq!(class.is_deferred(), expected);
        }
    }

    // ── Table-driven classification tests (SyncFailure variants) ────

    #[test]
    fn test_classify_sync_failure_not_configured() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::NotConfigured,
            None,
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Configuration);
    }

    #[test]
    fn test_classify_sync_failure_connect_failed() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::ConnectFailed,
            Some("connection refused"),
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Transient);
    }

    #[test]
    fn test_classify_sync_failure_health_check() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::HealthCheckFailed,
            None,
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Transient);
    }

    #[test]
    fn test_classify_sync_failure_auth() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::AuthenticationFailed,
            Some("unauthorized"),
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Configuration);
    }

    #[test]
    fn test_classify_sync_failure_sync_request() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::SyncRequestFailed,
            Some("tonic status: cancelled"),
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Transient);
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
        assert_eq!(FailureClass::from_error(&err), FailureClass::LocalFailure);
    }

    #[test]
    fn test_classify_sync_failure_partial() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::PartialSyncFailure,
            None,
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Transient);
    }

    #[test]
    fn test_classify_sync_failure_registration() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::RegistrationFailed,
            Some("device limit reached"),
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Configuration);
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
        assert_eq!(FailureClass::from_error(&err), FailureClass::Configuration);
    }

    #[test]
    fn test_classify_sync_disabled() {
        let err = crate::error::SnipError::runtime_error("sync not enabled", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Configuration);
    }

    #[test]
    fn test_classify_api_key() {
        let err = crate::error::SnipError::runtime_error(
            "Sync is enabled but no API key configured",
            None,
        );
        assert_eq!(FailureClass::from_error(&err), FailureClass::Configuration);
    }

    #[test]
    fn test_classify_health_check() {
        let err = crate::error::SnipError::runtime_error("Server health check failed", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Transient);
    }

    #[test]
    fn test_classify_server_unreachable() {
        let err =
            crate::error::SnipError::runtime_error("Server is not reachable", Some("timeout"));
        assert_eq!(FailureClass::from_error(&err), FailureClass::Transient);
    }

    #[test]
    fn test_classify_network() {
        let err = crate::error::SnipError::runtime_error("network error", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Transient);
    }

    #[test]
    fn test_classify_timeout() {
        let err = crate::error::SnipError::runtime_error("request timed out", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Transient);
    }

    #[test]
    fn test_classify_partial_failure() {
        let err = crate::error::SnipError::runtime_error("Some libraries failed to sync", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::Transient);
    }

    #[test]
    fn test_classify_conflict() {
        let err = crate::error::SnipError::runtime_error("merge conflict detected", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::LocalFailure);
    }

    #[test]
    fn test_classify_library_manager() {
        let err =
            crate::error::SnipError::runtime_error("Failed to initialize library manager", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::LocalFailure);
    }

    #[test]
    fn test_classify_save() {
        let err = crate::error::SnipError::runtime_error("Failed to save merged library", None);
        assert_eq!(FailureClass::from_error(&err), FailureClass::LocalFailure);
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
        assert_eq!(FailureClass::from_error(&err), FailureClass::LocalFailure);
    }

    #[test]
    fn test_classify_toml() {
        let toml_err = toml::from_str::<toml::Value>("invalid = [toml").unwrap_err();
        let err = crate::error::SnipError::toml_error("parse config", toml_err);
        assert_eq!(FailureClass::from_error(&err), FailureClass::LocalFailure);
    }

    // ── Retry disposition tests ────────────────────────────────────

    #[test]
    fn test_retry_disposition_deferred_disabled() {
        let disp = FailureClass::Configuration.retry_disposition(0);
        assert_eq!(disp, RetryDisposition::WaitForConfigurationChange);
    }

    #[test]
    fn test_retry_disposition_deferred_not_configured() {
        let disp = FailureClass::Configuration.retry_disposition(0);
        assert_eq!(disp, RetryDisposition::WaitForConfigurationChange);
    }

    #[test]
    fn test_retry_disposition_transient_network() {
        let disp = FailureClass::Transient.retry_disposition(0);
        assert!(matches!(disp, RetryDisposition::RetryAfter(_)));
    }

    #[test]
    fn test_retry_disposition_configuration_waits_for_change() {
        let disp = FailureClass::Configuration.retry_disposition(0);
        assert_eq!(disp, RetryDisposition::WaitForConfigurationChange);
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
