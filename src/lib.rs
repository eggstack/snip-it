//! Library for the `snp` snippet manager.
//!
//! Most of the implementation lives in submodules that are also used by
//! the `snp` binary entry point in `src/main.rs`. The library is also
//! re-exported so that integration tests under `tests/` can exercise the
//! public API (notably `sync::SyncClient` and the proto types) against a
//! real `snip-sync` server.

// Public modules form the stable API surface for crates.io consumers.
// Anything marked `pub(crate)` is internal implementation and may change
// without a semver bump.
//
// The `snp` binary lives in the same package as this library but is a
// separate crate, so it can only see `pub` items here. The CLI uses
// `commands`, `config`, `error`, `logging`, `sort`, and `ui` directly.
// Modules only used internally (sync client, encryption, proto,
// diagnostics, output, status, usage) are `pub(crate)`.
pub mod auto_sync;
pub mod commands;
pub mod config;
pub mod error;
pub mod logging;
pub mod outcome;
pub mod sort;
pub mod ui;

pub(crate) mod clipboard;
pub(crate) mod diagnostics;
pub(crate) mod encryption;
pub(crate) mod library;
pub(crate) mod local_data;
pub(crate) mod migration;
pub(crate) mod output;
pub mod proto;
pub mod selector;
pub(crate) mod status_snapshot;
pub mod sync;
pub(crate) mod sync_commands;
pub(crate) mod transaction;
pub mod usage;
pub(crate) mod utils;

pub use error::{SnipError, SnipResult};

// Re-export types needed by integration tests
pub use library::{LibraryConfig, LibraryMeta, Snippet, Snippets, load_library, save_library};
pub use utils::atomic::{
    AtomicWriteOptions, AtomicWriteReport, Durability, atomic_replace, write_private_atomic,
};

/// Aggregated data for all snippets passed to the TUI selector.
///
/// Contains parallel vectors of snippet metadata where index `i` corresponds
/// to the same snippet across all fields.
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

/// Top-level outcome returned by command implementations for exit-code mapping.
#[non_exhaustive]
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
}

/// Internal outcome of the shared snippet-selection TUI loop.
///
/// This is distinct from `CommandOutcome`: `SelectionOutcome` is the raw
/// result of the TUI interaction, while `CommandOutcome` is the CLI-level
/// semantic result mapped to exit codes in `main.rs`.
#[non_exhaustive]
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
