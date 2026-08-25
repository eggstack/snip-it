//! # snip-it — supported public API
//!
//! This crate provides the snippet data model, library persistence,
//! variable expansion, deterministic snippet selection, sort primitives,
//! typed errors, and atomic file writes.  Everything else (TUI, CLI
//! command implementations, auto-sync internals, logging, sync client,
//! encryption, process locks, protobuf types) is an implementation
//! detail and are not documented for external use; they may be changed
//! in a semver-appropriate release.
//!
//! ## Supported types and functions
//!
//! | Module | Public items |
//! |--------|-------------|
//! | crate root | `Snippet`, `Snippets`, `LibraryConfig`, `LibraryMeta`, `load_library`, `save_library` |
//! | crate root | `AtomicWriteOptions`, `AtomicWriteReport`, `Durability`, `atomic_replace`, `write_private_atomic` |
//! | [`error`] | `SnipError`, `SnipResult`, `SyncFailureKind` |
//! | [`sort`] | `SnippetSort`, `SortOptions`, `rank_snippets` |
//! | [`config`] | `SyncSettings`, `SyncDirection`, `AutoSyncFailureMode`, related constants |
//! | [`outcome`] | `CliOutcome`, `exit_code::*`, `OutputContext` |
//!
//! Modules marked `#[doc(hidden)]` are internal implementation used by
//! the `snp` binary and integration tests.  They are public only because
//! the binary and library are separate crates within the same package.

// ── Supported API (stable) ──────────────────────────────────────────
pub mod config;
pub mod error;
pub mod outcome;
pub mod sort;

// ── Implementation-only (binary + test access, hidden from docs) ───
#[doc(hidden)]
pub mod auto_sync;
#[doc(hidden)]
pub mod commands;
#[doc(hidden)]
pub mod logging;
#[doc(hidden)]
pub mod process_file_lock;
pub(crate) use snip_proto as proto;
#[doc(hidden)]
pub mod selector;
#[doc(hidden)]
pub mod sync;
#[doc(hidden)]
pub mod ui;
#[doc(hidden)]
pub mod usage;

// ── Crate-internal ──────────────────────────────────────────────────
pub(crate) mod clipboard;
pub(crate) mod diagnostics;
pub(crate) mod encryption;
pub(crate) mod library;
pub(crate) mod local_data;
pub(crate) mod migration;
pub(crate) mod output;
pub(crate) mod status_snapshot;
pub(crate) mod sync_commands;
pub(crate) mod sync_failure;
pub(crate) mod test_failpoints;
#[cfg(not(feature = "test-support"))]
pub(crate) mod transaction;
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub mod transaction;
pub(crate) mod utils;

pub use error::{SnipError, SnipResult};

// Re-export domain types for integration tests and binary access.
// The library data types and atomic write utilities are the supported
// public surface; the rest is exposed for crate-boundary reasons.
pub use library::{LibraryConfig, LibraryMeta, Snippet, Snippets, load_library, save_library};
pub use utils::atomic::{
    AtomicWriteOptions, AtomicWriteReport, Durability, atomic_replace, write_private_atomic,
};

/// Aggregated data for all snippets passed to the TUI selector.
///
/// Contains parallel vectors of snippet metadata where index `i` corresponds
/// to the same snippet across all fields.
#[doc(hidden)]
pub struct SnippetData {
    pub descriptions: Vec<String>,
    pub commands: Vec<String>,
    pub outputs: Vec<String>,
    pub tags: Vec<Vec<String>>,
    pub folders: Vec<Vec<String>>,
    pub favorites: Vec<bool>,
}

/// Result of processing a snippet selection from the TUI.
#[non_exhaustive]
#[derive(Debug)]
#[doc(hidden)]
pub enum ProcessResult {
    /// User cancelled the selection.
    Cancel,
    /// No snippet was selected; continue to next prompt.
    Continue,
    /// A snippet command was selected; contains the expanded command string.
    Done(String),
    /// Child process exited with a nonzero exit code.
    Failed {
        /// The child process exit code, if available.
        exit_code: Option<i32>,
        /// Human-readable description of the failure.
        message: String,
    },
}

impl ProcessResult {
    /// Returns true if this result represents a successful execution.
    pub fn is_done(&self) -> bool {
        matches!(self, ProcessResult::Done(_))
    }
}

/// Top-level outcome returned by command implementations for exit-code mapping.
#[non_exhaustive]
#[doc(hidden)]
pub enum CommandOutcome {
    /// Command completed successfully.
    Success,
    /// User cancelled the selection.
    Cancelled,
    /// Snippet execution failed (child exit, signal, timeout, spawn failure).
    ExecutionFailed {
        /// The child process exit code, if available.
        child_code: Option<i32>,
    },
    /// Terminal outcome carrying an already-mapped process exit code.
    Exit(i32),
}

/// Internal outcome of the shared snippet-selection TUI loop.
///
/// This is distinct from `CommandOutcome`: `SelectionOutcome` is the raw
/// result of the TUI interaction, while `CommandOutcome` is the CLI-level
/// semantic result mapped to exit codes in `main.rs`.
#[non_exhaustive]
#[doc(hidden)]
pub enum SelectionOutcome {
    /// A snippet was selected and processed by the callback.
    Selected,
    /// The user cancelled the primary selector (q, Esc, Ctrl-C).
    Cancelled,
    /// The snippet command was selected but child execution failed.
    ExecutionFailed {
        /// The child process exit code, if available.
        exit_code: Option<i32>,
    },
}
