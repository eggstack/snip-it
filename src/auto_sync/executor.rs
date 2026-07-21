//! Executor subprocess exit codes and sync direction resolution.
//!
//! The executor is a one-shot subprocess spawned by the detached worker.
//! The worker already holds the shared execution lock for the duration
//! of the cycle, so the executor never acquires or even references that
//! lock — that would deadlock the worker waiting on its own child.
//! The executor simply invokes the canonical sync operation
//! (`crate::sync_commands::run_sync`) and exits with a status code the
//! worker observes via `ExecutorExitCode`. This module defines the
//! exit-code taxonomy, the effective sync direction resolver, and the
//! executor command entry point.

use crate::auto_sync::policy::FailureClass;
use crate::config::{SyncDirection, SyncSettings};
use std::path::Path;
use std::process::ExitStatus;

/// Executor subprocess exit codes.
///
/// These codes are consumed by the detached worker (or any parent
/// process) to determine the outcome of a sync operation.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorExitCode {
    /// Sync completed successfully.
    Success = 0,
    /// Sync is not configured or disabled.
    NotConfigured = 2,
    /// Authentication failure (bad API key or unregistered account).
    AuthFailure = 3,
    /// Network or timeout failure.
    NetworkTimeout = 4,
    /// Conflict or partial sync failure.
    ConflictPartial = 5,
    /// Local persistence failure (could not write library files).
    LocalPersistence = 6,
    /// Internal error (unexpected failure).
    InternalError = 7,
    /// Transient timeout (deadline exceeded during sync).
    TransientTimeout = 8,
    /// Credential storage (keychain) failure.
    CredentialStore = 9,
    /// Configuration error that requires operator intervention.
    Configuration = 10,
    /// Partial sync completion (some libraries succeeded, some failed).
    Partial = 11,
}

impl ExecutorExitCode {
    /// Convert an `ExitStatus` to an `ExecutorExitCode`.
    ///
    /// Maps successful exit (code 0) to `Success`, known nonzero
    /// codes to their variant, and unknown codes to `InternalError`.
    /// On Unix, signal death (no exit code) maps to `InternalError`
    /// and is logged as a signal termination.
    pub fn from_exit_status(status: ExitStatus) -> Self {
        match status.code() {
            Some(0) => Self::Success,
            Some(2) => Self::NotConfigured,
            Some(3) => Self::AuthFailure,
            Some(4) => Self::NetworkTimeout,
            Some(5) => Self::ConflictPartial,
            Some(6) => Self::LocalPersistence,
            Some(7) => Self::InternalError,
            Some(8) => Self::TransientTimeout,
            Some(9) => Self::CredentialStore,
            Some(10) => Self::Configuration,
            Some(11) => Self::Partial,
            None => {
                // On Unix, None means the process was killed by a signal.
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if let Some(signal) = status.signal() {
                        tracing::error!(signal, "executor killed by signal");
                    }
                }
                Self::InternalError
            }
            _ => Self::InternalError,
        }
    }

    /// Convert this exit code to an `ExitStatus`.
    ///
    /// Uses `ExitStatus::from_raw` which is unstable, so we use
    /// `process::exit` indirectly. This is exposed for the executor
    /// subprocess to call directly.
    pub fn to_exit_status(self) -> i32 {
        self as i32
    }
}

impl ExecutorExitCode {
    /// Map this exit code to a `FailureClass`.
    ///
    /// This is the reverse of `from_failure_class` and is used by the
    /// worker to determine retry disposition from the executor's result.
    pub fn failure_class(self) -> FailureClass {
        match self {
            Self::Success => FailureClass::Internal, // shouldn't be called on Success
            Self::NotConfigured => FailureClass::DeferredNotConfigured,
            Self::AuthFailure => FailureClass::Authentication,
            Self::NetworkTimeout => FailureClass::TransientNetwork,
            Self::ConflictPartial => FailureClass::Conflict,
            Self::LocalPersistence => FailureClass::LocalPersistence,
            Self::InternalError => FailureClass::Internal,
            Self::TransientTimeout => FailureClass::TransientTimeout,
            Self::CredentialStore => FailureClass::CredentialStore,
            Self::Configuration => FailureClass::Configuration,
            Self::Partial => FailureClass::Partial,
        }
    }

    /// Map a `FailureClass` to an `ExecutorExitCode`.
    ///
    /// This is used by the executor to encode the failure class into
    /// the process exit status for the worker to observe.
    pub fn from_failure_class(class: FailureClass) -> Self {
        match class {
            FailureClass::DeferredDisabled | FailureClass::DeferredNotConfigured => {
                Self::NotConfigured
            }
            FailureClass::TransientNetwork => Self::NetworkTimeout,
            FailureClass::TransientTimeout => Self::TransientTimeout,
            FailureClass::Authentication => Self::AuthFailure,
            FailureClass::CredentialStore => Self::CredentialStore,
            FailureClass::Configuration => Self::Configuration,
            FailureClass::Conflict => Self::ConflictPartial,
            FailureClass::Partial => Self::Partial,
            FailureClass::LocalPersistence => Self::LocalPersistence,
            FailureClass::Internal => Self::InternalError,
        }
    }
}

impl std::fmt::Display for ExecutorExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "sync completed successfully"),
            Self::NotConfigured => write!(f, "sync not configured or disabled"),
            Self::AuthFailure => write!(f, "authentication failure"),
            Self::NetworkTimeout => write!(f, "network failure"),
            Self::ConflictPartial => write!(f, "conflict failure"),
            Self::LocalPersistence => write!(f, "local persistence failure"),
            Self::InternalError => write!(f, "internal error"),
            Self::TransientTimeout => write!(f, "timeout failure"),
            Self::CredentialStore => write!(f, "credential store failure"),
            Self::Configuration => write!(f, "configuration error"),
            Self::Partial => write!(f, "partial sync failure"),
        }
    }
}

/// Determine the effective sync direction from config and CLI flags.
///
/// Rules:
/// - Explicit CLI flags (`cli_push_only`, `cli_pull_only`) override
///   the configuration setting.
/// - No CLI override means use `settings.sync_direction`.
/// - Simultaneous push+pull rejection is handled by Clap.
pub fn effective_sync_direction(
    settings: &SyncSettings,
    cli_push_only: bool,
    cli_pull_only: bool,
) -> SyncDirection {
    if cli_push_only {
        SyncDirection::Push
    } else if cli_pull_only {
        SyncDirection::Pull
    } else {
        settings.sync_direction.clone()
    }
}

/// Command to execute from the executor subprocess.
pub enum ExecutorCommand {
    /// Proceed with sync.
    RunSync,
    /// Sync is not configured; exit immediately with NotConfigured.
    NotConfigured,
}

/// Executor entry point.
///
/// Loads sync settings, checks if sync is configured, determines
/// direction, creates a Tokio runtime, runs one sync, and maps the
/// outcome to an internal exit code.
pub fn run_executor(_state_dir: &Path) -> i32 {
    let settings = match crate::config::load_sync_settings() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("executor: failed to load sync settings: {e}");
            return ExecutorExitCode::InternalError.to_exit_status();
        }
    };

    if !settings.enabled {
        tracing::info!("executor: sync not enabled");
        return ExecutorExitCode::NotConfigured.to_exit_status();
    }

    if settings.api_key.is_empty() {
        tracing::info!("executor: no API key configured");
        return ExecutorExitCode::NotConfigured.to_exit_status();
    }

    let direction = effective_sync_direction(&settings, false, false);
    let (push_only, pull_only) = match direction {
        SyncDirection::Push => (true, false),
        SyncDirection::Pull => (false, true),
        SyncDirection::Bidirectional => (false, false),
    };

    tracing::info!(
        direction = ?direction,
        server_url = %settings.server_url,
        "executor: starting sync"
    );

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("snp-sync-executor")
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("executor: failed to create tokio runtime: {e}");
            return ExecutorExitCode::InternalError.to_exit_status();
        }
    };

    let result = crate::sync_commands::run_sync(&settings, None, push_only, pull_only, &runtime);

    match result {
        Ok(()) => {
            tracing::info!("executor: sync completed successfully");
            ExecutorExitCode::Success.to_exit_status()
        }
        Err(e) => {
            let class = classify_sync_error(&e);
            let code = ExecutorExitCode::from_failure_class(class);
            tracing::error!(exit_code = ?code, failure_class = %class.as_code(), error = %e, "executor: sync failed");
            code.to_exit_status()
        }
    }
}

/// Map a `SnipError` from `run_sync` to a `FailureClass`.
///
/// This is the canonical classification function. The `FailureClass`
/// is then mapped to an `ExecutorExitCode` for process exit status,
/// and used by the worker for retry disposition and status persistence.
pub fn classify_sync_error(error: &crate::error::SnipError) -> FailureClass {
    FailureClass::from_error(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SyncSettings;

    #[test]
    fn test_exit_code_values() {
        assert_eq!(ExecutorExitCode::Success as i32, 0);
        assert_eq!(ExecutorExitCode::NotConfigured as i32, 2);
        assert_eq!(ExecutorExitCode::AuthFailure as i32, 3);
        assert_eq!(ExecutorExitCode::NetworkTimeout as i32, 4);
        assert_eq!(ExecutorExitCode::ConflictPartial as i32, 5);
        assert_eq!(ExecutorExitCode::LocalPersistence as i32, 6);
        assert_eq!(ExecutorExitCode::InternalError as i32, 7);
        assert_eq!(ExecutorExitCode::TransientTimeout as i32, 8);
        assert_eq!(ExecutorExitCode::CredentialStore as i32, 9);
        assert_eq!(ExecutorExitCode::Configuration as i32, 10);
        assert_eq!(ExecutorExitCode::Partial as i32, 11);
    }

    #[test]
    fn test_exit_code_display() {
        assert_eq!(
            ExecutorExitCode::Success.to_string(),
            "sync completed successfully"
        );
        assert_eq!(
            ExecutorExitCode::NotConfigured.to_string(),
            "sync not configured or disabled"
        );
        assert_eq!(
            ExecutorExitCode::AuthFailure.to_string(),
            "authentication failure"
        );
        assert_eq!(
            ExecutorExitCode::NetworkTimeout.to_string(),
            "network failure"
        );
        assert_eq!(
            ExecutorExitCode::ConflictPartial.to_string(),
            "conflict failure"
        );
        assert_eq!(
            ExecutorExitCode::LocalPersistence.to_string(),
            "local persistence failure"
        );
        assert_eq!(
            ExecutorExitCode::InternalError.to_string(),
            "internal error"
        );
        assert_eq!(
            ExecutorExitCode::TransientTimeout.to_string(),
            "timeout failure"
        );
        assert_eq!(
            ExecutorExitCode::CredentialStore.to_string(),
            "credential store failure"
        );
        assert_eq!(
            ExecutorExitCode::Configuration.to_string(),
            "configuration error"
        );
        assert_eq!(
            ExecutorExitCode::Partial.to_string(),
            "partial sync failure"
        );
    }

    #[test]
    fn test_from_exit_status_success() {
        use std::process::Command;
        let output = Command::new("true").output().unwrap();
        assert_eq!(
            ExecutorExitCode::from_exit_status(output.status),
            ExecutorExitCode::Success
        );
    }

    #[test]
    fn test_to_exit_status_roundtrip() {
        let codes = [
            ExecutorExitCode::Success,
            ExecutorExitCode::NotConfigured,
            ExecutorExitCode::AuthFailure,
            ExecutorExitCode::NetworkTimeout,
            ExecutorExitCode::ConflictPartial,
            ExecutorExitCode::LocalPersistence,
            ExecutorExitCode::InternalError,
            ExecutorExitCode::TransientTimeout,
            ExecutorExitCode::CredentialStore,
            ExecutorExitCode::Configuration,
            ExecutorExitCode::Partial,
        ];
        for code in &codes {
            let raw = code.to_exit_status();
            let reconstructed = ExecutorExitCode::from_exit_status(
                std::process::Command::new("sh")
                    .arg("-c")
                    .arg(format!("exit {raw}"))
                    .status()
                    .unwrap(),
            );
            assert_eq!(*code, reconstructed, "roundtrip failed for {code:?}");
        }
    }

    #[test]
    fn test_effective_sync_direction_push_only_cli() {
        let settings = SyncSettings::default();
        assert_eq!(
            effective_sync_direction(&settings, true, false),
            SyncDirection::Push
        );
    }

    #[test]
    fn test_effective_sync_direction_pull_only_cli() {
        let settings = SyncSettings::default();
        assert_eq!(
            effective_sync_direction(&settings, false, true),
            SyncDirection::Pull
        );
    }

    #[test]
    fn test_effective_sync_direction_config_fallback() {
        let mut settings = SyncSettings::default();
        settings.sync_direction = SyncDirection::Bidirectional;
        assert_eq!(
            effective_sync_direction(&settings, false, false),
            SyncDirection::Bidirectional
        );
    }

    #[test]
    fn test_effective_sync_direction_cli_push_overrides_pull_config() {
        let mut settings = SyncSettings::default();
        settings.sync_direction = SyncDirection::Pull;
        assert_eq!(
            effective_sync_direction(&settings, true, false),
            SyncDirection::Push
        );
    }

    #[test]
    fn test_effective_sync_direction_cli_pull_overrides_push_config() {
        let mut settings = SyncSettings::default();
        settings.sync_direction = SyncDirection::Push;
        assert_eq!(
            effective_sync_direction(&settings, false, true),
            SyncDirection::Pull
        );
    }

    #[test]
    fn test_executor_subcommand_name() {
        assert_eq!(
            crate::auto_sync::spawn::EXECUTOR_SUBCOMMAND,
            "auto-sync-execute"
        );
    }

    #[test]
    fn test_classify_sync_error_not_configured() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::NotConfigured,
            None,
        );
        assert_eq!(
            classify_sync_error(&err),
            FailureClass::DeferredNotConfigured
        );
    }

    #[test]
    fn test_classify_sync_error_sync_disabled() {
        let err = crate::error::SnipError::runtime_error("sync not enabled", None);
        assert_eq!(
            classify_sync_error(&err),
            FailureClass::DeferredNotConfigured
        );
    }

    #[test]
    fn test_classify_sync_error_connect_failed() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::ConnectFailed,
            Some("connection refused"),
        );
        assert_eq!(classify_sync_error(&err), FailureClass::TransientNetwork);
    }

    #[test]
    fn test_classify_sync_error_health_check() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::HealthCheckFailed,
            None,
        );
        assert_eq!(classify_sync_error(&err), FailureClass::TransientNetwork);
    }

    #[test]
    fn test_classify_sync_error_auth() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::AuthenticationFailed,
            Some("unauthorized"),
        );
        assert_eq!(classify_sync_error(&err), FailureClass::Authentication);
    }

    #[test]
    fn test_classify_sync_error_partial() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::PartialSyncFailure,
            None,
        );
        assert_eq!(classify_sync_error(&err), FailureClass::Partial);
    }

    #[test]
    fn test_classify_sync_error_save_library() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::SaveMergedLibraryFailed,
            Some("disk full"),
        );
        assert_eq!(classify_sync_error(&err), FailureClass::LocalPersistence);
    }

    #[test]
    fn test_classify_sync_error_library_manager() {
        let err = crate::error::SnipError::sync_failure(
            crate::error::SyncFailureKind::LibraryManagerInitFailed,
            Some("permission denied"),
        );
        assert_eq!(classify_sync_error(&err), FailureClass::LocalPersistence);
    }

    #[test]
    fn test_classify_sync_error_unknown_runtime() {
        let err = crate::error::SnipError::runtime_error("something went wrong", None);
        assert_eq!(classify_sync_error(&err), FailureClass::Internal);
    }

    #[test]
    fn test_classify_sync_error_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let err: crate::error::SnipError = io_err.into();
        assert_eq!(classify_sync_error(&err), FailureClass::LocalPersistence);
    }

    #[test]
    fn test_failure_class_to_exit_code_mapping() {
        let cases = [
            (
                FailureClass::DeferredDisabled,
                ExecutorExitCode::NotConfigured,
            ),
            (
                FailureClass::DeferredNotConfigured,
                ExecutorExitCode::NotConfigured,
            ),
            (
                FailureClass::TransientNetwork,
                ExecutorExitCode::NetworkTimeout,
            ),
            (
                FailureClass::TransientTimeout,
                ExecutorExitCode::TransientTimeout,
            ),
            (FailureClass::Authentication, ExecutorExitCode::AuthFailure),
            (
                FailureClass::CredentialStore,
                ExecutorExitCode::CredentialStore,
            ),
            (FailureClass::Configuration, ExecutorExitCode::Configuration),
            (FailureClass::Conflict, ExecutorExitCode::ConflictPartial),
            (FailureClass::Partial, ExecutorExitCode::Partial),
            (
                FailureClass::LocalPersistence,
                ExecutorExitCode::LocalPersistence,
            ),
            (FailureClass::Internal, ExecutorExitCode::InternalError),
        ];
        for (class, expected_code) in cases {
            assert_eq!(
                ExecutorExitCode::from_failure_class(class),
                expected_code,
                "FailureClass::{class:?} should map to {expected_code:?}"
            );
        }
    }

    #[test]
    fn test_exit_code_to_failure_class_roundtrip() {
        // Every exit code should map to a valid failure class
        let codes = [
            ExecutorExitCode::Success,
            ExecutorExitCode::NotConfigured,
            ExecutorExitCode::AuthFailure,
            ExecutorExitCode::NetworkTimeout,
            ExecutorExitCode::ConflictPartial,
            ExecutorExitCode::LocalPersistence,
            ExecutorExitCode::InternalError,
            ExecutorExitCode::TransientTimeout,
            ExecutorExitCode::CredentialStore,
            ExecutorExitCode::Configuration,
            ExecutorExitCode::Partial,
        ];
        for code in &codes {
            let class = code.failure_class();
            // The roundtrip through from_failure_class should give back
            // the same exit code (except Success which maps to Internal)
            if *code != ExecutorExitCode::Success {
                assert_eq!(
                    ExecutorExitCode::from_failure_class(class),
                    *code,
                    "roundtrip failed for {code:?}"
                );
            }
        }
    }

    #[test]
    fn test_distinct_exit_codes_per_failure_class() {
        // Every FailureClass must map to a unique exit code
        let all_classes = [
            FailureClass::DeferredDisabled,
            FailureClass::DeferredNotConfigured,
            FailureClass::TransientNetwork,
            FailureClass::TransientTimeout,
            FailureClass::Authentication,
            FailureClass::CredentialStore,
            FailureClass::Configuration,
            FailureClass::Conflict,
            FailureClass::Partial,
            FailureClass::LocalPersistence,
            FailureClass::Internal,
        ];
        let mut seen = std::collections::HashMap::new();
        for class in &all_classes {
            let code = ExecutorExitCode::from_failure_class(*class);
            let code_val = code as i32;
            if let Some(prev_class) = seen.insert(code_val, class) {
                // DeferredDisabled and DeferredNotConfigured both map to NotConfigured — that's OK
                if matches!(
                    class,
                    FailureClass::DeferredDisabled | FailureClass::DeferredNotConfigured
                ) && matches!(
                    prev_class,
                    FailureClass::DeferredDisabled | FailureClass::DeferredNotConfigured
                ) {
                    continue;
                }
                panic!(
                    "FailureClass::{class:?} and FailureClass::{prev_class:?} both map to exit code {code_val}"
                );
            }
        }
    }
}
