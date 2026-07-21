//! **Layer: Domain/Core**
//!
//! Public CLI outcome types and exit-code mapping.
//!
//! Provides [`CliOutcome`] for typed command results and [`exit_code`]
//! for centralized exit-code mapping. Internal worker/executor codes
//! remain hidden.

/// Typed application outcome for public CLI exit-code mapping.
///
/// Each variant maps to a stable, documented exit code. The central
/// exit mapper in `main.rs` converts these to process exit codes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CliOutcome {
    /// Command completed successfully.
    Success,
    /// Snippet or resource not found.
    NotFound,
    /// Multiple matches found but only one was expected.
    Ambiguous,
    /// User cancelled an interactive action.
    Cancelled,
    /// Data validation or local persistence failure.
    ValidationFailed,
    /// Atomic write or persistence layer error.
    PersistenceFailed,
    /// Synchronization with remote server failed.
    SyncFailed,
    /// Snippet execution failed (child process exit).
    ExecutionFailed {
        /// The child process exit code, if available.
        child_code: Option<i32>,
    },
    /// Destructive action refused or generation changed.
    ConflictOrRefused,
}

/// Stable public CLI exit codes.
///
/// These are documented in `--help` and shell integration. Changes
/// require a compatibility review.
pub mod exit_code {
    /// Success.
    pub const SUCCESS: i32 = 0;
    /// General operational failure.
    pub const GENERAL_ERROR: i32 = 1;
    /// CLI usage or argument error (typically Clap-controlled).
    pub const USAGE_ERROR: i32 = 2;
    /// Snippet or resource not found.
    pub const NOT_FOUND: i32 = 3;
    /// User cancelled an interactive action.
    pub const CANCELLED: i32 = 4;
    /// Ambiguous match (multiple candidates, unique policy requested).
    pub const AMBIGUOUS: i32 = 5;
    /// Validation or local persistence failure.
    pub const VALIDATION_FAILED: i32 = 6;
    /// Synchronization failure.
    pub const SYNC_FAILED: i32 = 7;
    /// Snippet execution failure wrapper.
    pub const EXECUTION_FAILED: i32 = 8;
    /// Destructive action refused or generation changed.
    pub const CONFLICT_OR_REFUSED: i32 = 9;
}

impl CliOutcome {
    /// Map a `CliOutcome` to a stable exit code.
    pub fn exit_code(&self) -> i32 {
        match self {
            CliOutcome::Success => exit_code::SUCCESS,
            CliOutcome::NotFound => exit_code::NOT_FOUND,
            CliOutcome::Ambiguous => exit_code::AMBIGUOUS,
            CliOutcome::Cancelled => exit_code::CANCELLED,
            CliOutcome::ValidationFailed => exit_code::VALIDATION_FAILED,
            CliOutcome::PersistenceFailed => exit_code::GENERAL_ERROR,
            CliOutcome::SyncFailed => exit_code::SYNC_FAILED,
            CliOutcome::ExecutionFailed { child_code } => {
                // If the child had a valid exit code, propagate it;
                // otherwise use the wrapper code.
                child_code.unwrap_or(exit_code::EXECUTION_FAILED)
            }
            CliOutcome::ConflictOrRefused => exit_code::CONFLICT_OR_REFUSED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_success_exit_code() {
        assert_eq!(CliOutcome::Success.exit_code(), 0);
    }

    #[test]
    fn test_not_found_exit_code() {
        assert_eq!(CliOutcome::NotFound.exit_code(), 3);
    }

    #[test]
    fn test_ambiguous_exit_code() {
        assert_eq!(CliOutcome::Ambiguous.exit_code(), 5);
    }

    #[test]
    fn test_cancelled_exit_code() {
        assert_eq!(CliOutcome::Cancelled.exit_code(), 4);
    }

    #[test]
    fn test_validation_failed_exit_code() {
        assert_eq!(CliOutcome::ValidationFailed.exit_code(), 6);
    }

    #[test]
    fn test_sync_failed_exit_code() {
        assert_eq!(CliOutcome::SyncFailed.exit_code(), 7);
    }

    #[test]
    fn test_execution_failed_no_child_code() {
        assert_eq!(
            CliOutcome::ExecutionFailed { child_code: None }.exit_code(),
            8
        );
    }

    #[test]
    fn test_execution_failed_with_child_code() {
        assert_eq!(
            CliOutcome::ExecutionFailed {
                child_code: Some(127)
            }
            .exit_code(),
            127
        );
    }

    #[test]
    fn test_conflict_or_refused_exit_code() {
        assert_eq!(CliOutcome::ConflictOrRefused.exit_code(), 9);
    }

    #[test]
    fn test_persistence_failed_uses_general_error() {
        assert_eq!(CliOutcome::PersistenceFailed.exit_code(), 1);
    }

    #[test]
    fn test_all_exit_codes_are_distinct() {
        let codes = [
            CliOutcome::Success.exit_code(),
            CliOutcome::NotFound.exit_code(),
            CliOutcome::Ambiguous.exit_code(),
            CliOutcome::Cancelled.exit_code(),
            CliOutcome::ValidationFailed.exit_code(),
            CliOutcome::SyncFailed.exit_code(),
            CliOutcome::ExecutionFailed { child_code: None }.exit_code(),
            CliOutcome::ConflictOrRefused.exit_code(),
        ];
        let mut sorted = codes.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(codes.len(), sorted.len(), "Exit codes must be distinct");
    }
}
