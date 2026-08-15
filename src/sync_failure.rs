//! **Layer: Sync-Client**
//!
//! Typed failure classification for sync operations.
//!
//! `FailureClass` lives in the sync-client layer (not the application layer)
//! so that the gRPC sync client ([`crate::sync`]) can classify its own errors
//! without depending upward on `crate::auto_sync`. The application layer
//! ([`crate::auto_sync::policy`]) re-exports this type and layers scheduling
//! policy (`RetryDisposition`, `transient_backoff`, `retry_disposition`) on top
//! of it, which respects the `application → sync-client → core` dependency
//! direction documented in `docs/LOGICAL_LAYERS.md`.

use crate::error::{SnipError, SyncFailureKind};

/// Typed failure classification for sync operations.
///
/// Collapsed to distinct user-action categories: transient retryable
/// failure, configuration/authentication failure requiring user correction,
/// local persistence/corruption failure requiring repair, and internal
/// errors with bounded retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FailureClass {
    /// Transient network, timeout, or partial sync failure — retry with backoff.
    Transient,
    /// Configuration, authentication, or credential failure — requires user correction.
    Configuration,
    /// Local persistence, conflict, or corruption failure — requires repair.
    LocalFailure,
    /// Internal or unclassified error — bounded retry, then requires attention.
    Internal,
}

impl FailureClass {
    pub fn from_error(err: &SnipError) -> Self {
        match err {
            SnipError::SyncFailure { kind, .. } => match kind {
                SyncFailureKind::NotConfigured => FailureClass::Configuration,
                SyncFailureKind::ConnectFailed => FailureClass::Transient,
                SyncFailureKind::HealthCheckFailed => FailureClass::Transient,
                SyncFailureKind::AuthenticationFailed => FailureClass::Configuration,
                SyncFailureKind::SyncRequestFailed => FailureClass::Transient,
                SyncFailureKind::CreateLibraryFailed => FailureClass::Configuration,
                SyncFailureKind::GetPremadeLibraryFailed => FailureClass::Transient,
                SyncFailureKind::RegistrationFailed => FailureClass::Configuration,
                SyncFailureKind::LibraryManagerInitFailed => FailureClass::LocalFailure,
                SyncFailureKind::LibraryModeInitFailed => FailureClass::LocalFailure,
                SyncFailureKind::LibrariesDirReadFailed => FailureClass::LocalFailure,
                SyncFailureKind::NoLibrariesToSync => FailureClass::Internal,
                SyncFailureKind::SaveMergedLibraryFailed => FailureClass::LocalFailure,
                SyncFailureKind::PartialSyncFailure => FailureClass::Transient,
                SyncFailureKind::PremadePartialFailure => FailureClass::Transient,
                SyncFailureKind::EncryptionFailed => FailureClass::Internal,
                SyncFailureKind::DecryptionFailed => FailureClass::Internal,
                SyncFailureKind::LibraryNotFound => FailureClass::Configuration,
                SyncFailureKind::Timeout => FailureClass::Transient,
                SyncFailureKind::RequestTooLarge => FailureClass::Configuration,
                SyncFailureKind::ClockSkew => FailureClass::Configuration,
            },
            SnipError::Runtime { message, detail } => {
                let combined = format!("{message} {}", detail.as_deref().unwrap_or(""));
                let lower = combined.to_lowercase();
                if lower.contains("not configured")
                    || lower.contains("sync not enabled")
                    || lower.contains("api key")
                    || lower.contains("auth")
                    || lower.contains("unauthorized")
                    || lower.contains("forbidden")
                    || lower.contains("permission denied")
                    || lower.contains("credential")
                    || lower.contains("keychain")
                {
                    FailureClass::Configuration
                } else if lower.contains("health check")
                    || lower.contains("server")
                    || lower.contains("network")
                    || lower.contains("connection")
                    || lower.contains("dns")
                    || lower.contains("connection refused")
                    || lower.contains("connect")
                    || lower.contains("unavailable")
                    || lower.contains("unreachable")
                    || lower.contains("timeout")
                    || lower.contains("timed out")
                    || lower.contains("failed to sync")
                    || lower.contains("some libraries")
                    || lower.contains("skipped")
                {
                    FailureClass::Transient
                } else if lower.contains("failed to save")
                    || lower.contains("failed to read")
                    || lower.contains("failed to initialize")
                    || lower.contains("failed to create")
                    || lower.contains("i/o")
                    || lower.contains("permission")
                    || lower.contains("conflict")
                    || lower.contains("merge")
                {
                    FailureClass::LocalFailure
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
                    FailureClass::Transient
                } else {
                    FailureClass::LocalFailure
                }
            }
            SnipError::Toml { .. } => FailureClass::LocalFailure,
            _ => FailureClass::Internal,
        }
    }

    pub fn as_code(&self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Configuration => "configuration",
            Self::LocalFailure => "local_failure",
            Self::Internal => "internal",
        }
    }

    pub fn from_code(code: &str) -> Self {
        match code {
            "transient" => Self::Transient,
            "configuration" => Self::Configuration,
            "local_failure" => Self::LocalFailure,
            "internal" => Self::Internal,
            // Legacy codes for backward compatibility
            "deferred_disabled"
            | "deferred_not_configured"
            | "authentication"
            | "credential_store" => Self::Configuration,
            "transient_network" | "transient_timeout" | "partial" => Self::Transient,
            "conflict" | "local_persistence" => Self::LocalFailure,
            _ => Self::Internal,
        }
    }

    pub fn allows_automatic_retry(&self) -> bool {
        matches!(self, Self::Transient | Self::Internal)
    }

    pub fn is_deferred(&self) -> bool {
        matches!(self, Self::Configuration)
    }
}
