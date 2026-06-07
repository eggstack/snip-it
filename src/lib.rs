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
// `commands`, `config`, `error`, `logging`, and `ui` directly. The
// truly internal modules (`clipboard`, `library`, `sync_commands`,
// `utils`) are accessed only via `crate::` from sibling modules, so
// they can be hidden from external consumers.
pub mod commands;
pub mod config;
pub mod encryption;
pub mod error;
pub mod logging;
pub mod proto;
pub mod sync;
pub mod ui;

pub(crate) mod clipboard;
pub(crate) mod library;
pub(crate) mod sync_commands;
pub(crate) mod utils;

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
