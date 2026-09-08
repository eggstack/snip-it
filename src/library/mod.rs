//! **Layer: Domain/Core**
//!
//! Core data structures and library management.
//!
//! This module provides the foundational types for storing and managing snippets:
//! - [`Snippet`]: Individual snippet with command, description, tags, etc.
//! - [`Snippets`]: Collection container for multiple snippets
//! - [`LibraryManager`]: Manages multiple snippet libraries and premade collections
//!
//! # Snippet TOML Format
//!
//! ```toml
//! [[snippets]]
//! description = "git commit"
//! tag = ["git"]
//! command = "git commit -m \"<msg>\""
//! ```
//!
//! ## Layout
//!
//! - [`model`]: pure data types (`Snippet`, `Snippets`, `LibraryConfig`,
//!   `LibraryMeta`), read-only resolution types (`ResolvedLibrarySource`,
//!   `PrimaryState`, `LibraryIndexInspection`), and pure helpers
//!   (`library_not_found`, `find_orphaned_ids`, `validate_library_name`).
//! - [`persistence`]: load/save, deterministic ID normalization, backups.
//! - [`manager`]: [`LibraryManager`] registry/primary behavior and the
//!   canonical read-only resolver.
//!
//! Root re-exports preserve the pre-split `crate::library::*` paths.

pub mod manager;
pub mod model;
pub mod persistence;

// Root re-exports preserve the pre-split `crate::library::*` paths.
// Glob re-exports avoid unused-import warnings for items currently used
// only through inference (e.g. `LibraryIndexInspection` as a return type).
pub use manager::*;
pub use model::*;
pub use persistence::*;

#[cfg(test)]
mod tests;
