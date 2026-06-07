//! Library for the `snp` snippet manager.
//!
//! Most of the implementation lives in submodules that are also used by
//! the `snp` binary entry point in `src/main.rs`. The library is also
//! re-exported so that integration tests under `tests/` can exercise the
//! public API (notably `sync::SyncClient` and the proto types) against a
//! real `snip-sync` server.

pub mod clipboard;
pub mod commands;
pub mod config;
pub mod encryption;
pub mod error;
pub mod library;
pub mod logging;
pub mod proto;
pub mod sync;
pub mod sync_commands;
pub mod ui;
pub mod utils;

pub use error::{SnipError, SnipResult};

/// Aggregated data for all snippets passed to the TUI selector.
///
/// Contains parallel vectors of snippet metadata where index `i` corresponds
/// to the same snippet across all fields.
pub struct SnippetData {
    pub descriptions: Vec<String>,
    pub commands: Vec<String>,
    pub tags: Vec<Vec<String>>,
    pub folders: Vec<Vec<String>>,
    pub favorites: Vec<bool>,
}

/// Result of processing a snippet selection from the TUI.
pub enum ProcessResult {
    /// User cancelled the selection.
    Cancel,
    /// No snippet was selected; continue to next prompt.
    Continue,
    /// A snippet command was selected; contains the expanded command string.
    Done(String),
}
