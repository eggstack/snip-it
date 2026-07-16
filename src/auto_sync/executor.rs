//! Executor subprocess exit codes and sync direction resolution.
//!
//! The executor is a one-shot subprocess spawned by the detached worker.
//! It acquires the execution lock, performs the sync, and exits with a
//! status code that the worker observes. This module defines the exit
//! code taxonomy, the effective sync direction resolver, and the
//! executor command entry point.

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
}

impl ExecutorExitCode {
    /// Convert an `ExitStatus` to an `ExecutorExitCode`.
    ///
    /// Maps successful exit (code 0) to `Success`, known nonzero
    /// codes to their variant, and unknown codes to `InternalError`.
    pub fn from_exit_status(status: ExitStatus) -> Self {
        match status.code() {
            Some(0) => Self::Success,
            Some(2) => Self::NotConfigured,
            Some(3) => Self::AuthFailure,
            Some(4) => Self::NetworkTimeout,
            Some(5) => Self::ConflictPartial,
            Some(6) => Self::LocalPersistence,
            Some(7) => Self::InternalError,
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

impl std::fmt::Display for ExecutorExitCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Success => write!(f, "sync completed successfully"),
            Self::NotConfigured => write!(f, "sync not configured or disabled"),
            Self::AuthFailure => write!(f, "authentication failure"),
            Self::NetworkTimeout => write!(f, "network or timeout failure"),
            Self::ConflictPartial => write!(f, "conflict or partial sync failure"),
            Self::LocalPersistence => write!(f, "local persistence failure"),
            Self::InternalError => write!(f, "internal error"),
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
/// direction, and maps the outcome to an exit code. Currently a
/// stub that returns `Success` — full implementation in Phase 2.
pub fn run_executor(state_dir: &Path) -> i32 {
    let _state_dir = state_dir;

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

    let _direction = effective_sync_direction(&settings, false, false);

    // TODO(Phase 2): acquire execution lock, run sync, map errors to exit codes
    tracing::info!("executor: sync completed (stub)");
    ExecutorExitCode::Success.to_exit_status()
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
            "network or timeout failure"
        );
        assert_eq!(
            ExecutorExitCode::ConflictPartial.to_string(),
            "conflict or partial sync failure"
        );
        assert_eq!(
            ExecutorExitCode::LocalPersistence.to_string(),
            "local persistence failure"
        );
        assert_eq!(
            ExecutorExitCode::InternalError.to_string(),
            "internal error"
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
        ];
        for code in &codes {
            let raw = code.to_exit_status();
            let reconstructed = match raw {
                0 => ExecutorExitCode::Success,
                2 => ExecutorExitCode::NotConfigured,
                3 => ExecutorExitCode::AuthFailure,
                4 => ExecutorExitCode::NetworkTimeout,
                5 => ExecutorExitCode::ConflictPartial,
                6 => ExecutorExitCode::LocalPersistence,
                7 => ExecutorExitCode::InternalError,
                _ => ExecutorExitCode::InternalError,
            };
            assert_eq!(*code, reconstructed);
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
}
