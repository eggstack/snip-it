//! **Layer: Domain/Core**
//!
//! Public CLI outcome types, exit-code mapping, and machine-output guard.
//!
//! Provides [`CliOutcome`] for typed command results, [`exit_code`]
//! for centralized exit-code mapping, and [`OutputContext`] for
//! enforcing machine-output purity rules. Internal worker/executor
//! codes remain hidden.

use std::io::Write;

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

/// Color output policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorPolicy {
    /// Use ANSI colors if terminal supports it (default).
    #[default]
    Auto,
    /// Force ANSI color codes.
    Always,
    /// Never emit ANSI color codes.
    Never,
}

/// Output mode for command results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputMode {
    /// Human-readable output (default).
    #[default]
    Human,
    /// Machine-readable JSON output.
    Json,
    /// Machine-readable CSV output.
    Csv,
    /// Raw byte output (no trailing newline).
    Raw,
    /// Field-specific output (no trailing newline).
    Field,
    /// Expanded variable output.
    Expanded,
}

/// Application-level guard for machine-output purity.
///
/// Commands that produce machine output (`--json`, `--csv`, `--raw`,
/// `--field`) should construct an `OutputContext` and use its guard
/// methods to ensure stdout is not contaminated by ANSI codes, update
/// notices, tracing output, or prompts.
///
/// # Rules
///
/// - Data only on stdout; diagnostics on stderr.
/// - No ANSI unless explicitly requested in a human mode.
/// - No update notices or auto-sync advisories on stdout.
/// - No prompts in machine mode.
/// - No progress spinners.
/// - No tracing subscriber writing to stdout.
/// - No extra newline in exact-byte modes (`Raw`, `Field`).
/// - Broken pipe handled gracefully without backtrace/noise.
/// - Serialization failure returns nonzero exit code.
#[derive(Debug, Clone)]
pub struct OutputContext {
    /// The active output mode.
    pub mode: OutputMode,
    /// The color policy.
    pub color: ColorPolicy,
    /// Whether the output is going to an interactive terminal.
    pub interactive: bool,
}

impl OutputContext {
    /// Create a new output context with default settings.
    pub fn human() -> Self {
        Self {
            mode: OutputMode::Human,
            color: ColorPolicy::Auto,
            interactive: true,
        }
    }

    /// Create a context for JSON output.
    pub fn json() -> Self {
        Self {
            mode: OutputMode::Json,
            color: ColorPolicy::Never,
            interactive: false,
        }
    }

    /// Create a context for CSV output.
    pub fn csv() -> Self {
        Self {
            mode: OutputMode::Csv,
            color: ColorPolicy::Never,
            interactive: false,
        }
    }

    /// Create a context for raw byte output.
    pub fn raw() -> Self {
        Self {
            mode: OutputMode::Raw,
            color: ColorPolicy::Never,
            interactive: false,
        }
    }

    /// Create a context for field-specific output.
    pub fn field() -> Self {
        Self {
            mode: OutputMode::Field,
            color: ColorPolicy::Never,
            interactive: false,
        }
    }

    /// Returns `true` if the output mode is machine-readable.
    pub fn is_machine_mode(&self) -> bool {
        matches!(
            self.mode,
            OutputMode::Json | OutputMode::Csv | OutputMode::Raw | OutputMode::Field
        )
    }

    /// Returns `true` if ANSI color should be suppressed.
    pub fn suppress_ansi(&self) -> bool {
        self.color == ColorPolicy::Never
            || self.is_machine_mode()
            || (self.color == ColorPolicy::Auto && !self.interactive)
    }

    /// Write raw bytes to stdout, handling broken pipe gracefully.
    pub fn write_stdout(&self, data: &[u8]) -> std::io::Result<()> {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        match handle.write_all(data) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Write a line to stdout (adds newline).
    pub fn writeln(&self, text: &str) -> std::io::Result<()> {
        let mut data = text.as_bytes().to_vec();
        data.push(b'\n');
        self.write_stdout(&data)
    }

    /// Write a diagnostic message to stderr.
    pub fn diagnostic(&self, text: &str) {
        let _ = writeln!(std::io::stderr(), "{text}");
    }

    /// Ensure no ANSI codes are present in machine mode output.
    pub fn strip_ansi_if_needed(&self, text: &str) -> String {
        if self.suppress_ansi() {
            // Strip ANSI escape sequences
            let mut result = String::with_capacity(text.len());
            let mut chars = text.chars();
            while let Some(c) = chars.next() {
                if c == '\x1b' {
                    // Skip until 'm' (SGR sequence end) or non-control
                    for next in chars.by_ref() {
                        if next == 'm' {
                            break;
                        }
                    }
                } else {
                    result.push(c);
                }
            }
            result
        } else {
            text.to_string()
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
    fn test_output_context_human() {
        let ctx = OutputContext::human();
        assert_eq!(ctx.mode, OutputMode::Human);
        assert!(!ctx.is_machine_mode());
    }

    #[test]
    fn test_output_context_json_is_machine() {
        let ctx = OutputContext::json();
        assert!(ctx.is_machine_mode());
        assert!(ctx.suppress_ansi());
    }

    #[test]
    fn test_output_context_raw_is_machine() {
        let ctx = OutputContext::raw();
        assert!(ctx.is_machine_mode());
    }

    #[test]
    fn test_output_context_field_is_machine() {
        let ctx = OutputContext::field();
        assert!(ctx.is_machine_mode());
    }

    #[test]
    fn test_suppress_ansi_never_policy() {
        let ctx = OutputContext {
            color: ColorPolicy::Never,
            ..OutputContext::human()
        };
        assert!(ctx.suppress_ansi());
    }

    #[test]
    fn test_suppress_ansi_auto_noninteractive() {
        let ctx = OutputContext {
            color: ColorPolicy::Auto,
            interactive: false,
            ..OutputContext::human()
        };
        assert!(ctx.suppress_ansi());
    }

    #[test]
    fn test_suppress_ansi_auto_interactive() {
        let ctx = OutputContext::human();
        assert!(!ctx.suppress_ansi());
    }

    #[test]
    fn test_strip_ansi_in_machine_mode() {
        let ctx = OutputContext::json();
        let text = "\x1b[32msome text\x1b[0m";
        let stripped = ctx.strip_ansi_if_needed(text);
        assert_eq!(stripped, "some text");
    }

    #[test]
    fn test_no_strip_ansi_in_human_mode() {
        let ctx = OutputContext::human();
        let text = "\x1b[32msome text\x1b[0m";
        let result = ctx.strip_ansi_if_needed(text);
        assert_eq!(result, text);
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
