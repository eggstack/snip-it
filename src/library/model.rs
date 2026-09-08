//! **Layer: Domain/Core**
//!
//! Library data model: [`Snippet`], [`Snippets`], [`LibraryConfig`],
//! [`LibraryMeta`], read-only resolution types, and pure validation helpers.

use crate::error::{SnipError, SnipResult};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// Container for a collection of snippets.
///
/// Wraps a list of [`Snippet`] items and optional folder structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Snippets {
    // `snippets` is pet's canonical table name. Keep `Snippets` as an alias
    // so existing snp libraries and premade collections continue to load.
    #[serde(rename = "snippets", alias = "Snippets", default)]
    pub snippets: Vec<Snippet>,
    #[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
}

/// Individual snippet with metadata.
///
/// A snippet contains a command to execute along with optional description,
/// tags, and sync-related fields. The command may include variables using
/// `<name>` or `<name=default>` syntax.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Snippet {
    #[serde(rename = "id", alias = "Id", alias = "ID", default)]
    pub id: String,
    #[serde(alias = "Description", alias = "name", default)]
    pub description: String,
    #[serde(rename = "output", alias = "Output", default)]
    pub output: String,
    #[serde(
        rename = "tag",
        alias = "Tag",
        alias = "Tags",
        alias = "tags",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub tags: Vec<String>,
    #[serde(alias = "Command", alias = "cmd", default)]
    pub command: String,
    #[serde(default = "Vec::new", skip_serializing_if = "Vec::is_empty")]
    pub folders: Vec<String>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub device_id: String,
    #[serde(default)]
    pub deleted: bool,
}

/// Configuration for managing snippet libraries.
///
/// Stored in `libraries.toml` and tracks metadata for all libraries.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LibraryConfig {
    #[serde(default)]
    pub libraries: Vec<LibraryMeta>,
    /// Monotonic generation counter that increments on every mutation.
    /// Used by backup to verify coherent snapshots.
    #[serde(default)]
    pub generation: u64,
}

/// Metadata for a single snippet library.
///
/// Tracks the library filename, optional server linkage, sync state,
/// and whether it is the primary library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryMeta {
    pub filename: String,
    #[serde(default)]
    pub library_id: String,
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default)]
    pub last_sync: Option<i64>,
    #[serde(default)]
    pub server_id: Option<String>,
}

impl LibraryMeta {
    /// Creates a new library metadata entry with the given filename.
    pub fn new(filename: &str) -> Self {
        Self {
            filename: filename.to_string(),
            library_id: String::new(),
            is_primary: false,
            last_sync: None,
            server_id: None,
        }
    }
}

/// A single side-effect-free library source.
///
/// Canonical result of [`LibraryManager::resolve_readonly_sources`]. Carries
/// the canonical filename/name, library ID, and file path for one visible
/// library. Constructed without migration, directory creation, or metadata
/// rewrites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLibrarySource {
    /// Canonical library name (filename without `.toml`).
    pub name: String,
    /// Server-side library ID when linked, otherwise empty.
    pub library_id: String,
    /// Canonical path to the library TOML file.
    pub path: PathBuf,
}

/// Primary-library state observed by [`LibraryManager::inspect_library_index`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum PrimaryState {
    /// A primary library is set and its file exists.
    Present { name: String },
    /// A primary library is set but its file is missing.
    FileMissing { name: String, path: PathBuf },
    /// Libraries exist but none is marked primary.
    NoPrimary { count: usize },
    /// No libraries are registered.
    #[default]
    NoLibraries,
}

/// Read-only library index consistency view.
///
/// Returned by [`LibraryManager::inspect_library_index`]. Consumers
/// (`doctor`, `validate`, `repair`, `status`) render this shared state into
/// their own diagnostic types with distinct user semantics.
#[derive(Debug, Clone, Default)]
pub struct LibraryIndexInspection {
    /// Index entries whose library file does not exist: `(name, path)`.
    pub missing_files: Vec<(String, PathBuf)>,
    /// Library files on disk with no index entry.
    pub orphan_files: Vec<PathBuf>,
    /// Current primary-library state.
    pub primary: PrimaryState,
}

/// Canonical missing-library error.
///
/// All read-only source resolution paths use this constructor so "library
/// not found" reporting cannot drift between CLI, MCP, and diagnostics.
pub fn library_not_found(name: &str) -> SnipError {
    SnipError::runtime_error(
        "Library not found",
        Some(&format!(
            "Library '{name}' does not exist. Use 'snp library list' to see available libraries."
        )),
    )
}

/// Find usage IDs with no matching active snippet ID.
///
/// Pure helper shared by `validate` (warning diagnostics) and `repair`
/// (prune candidates). `active_ids` holds every live snippet ID; `usage_ids`
/// holds candidate usage entry IDs in stable order. Empty usage IDs are
/// ignored, matching both consumers' existing behavior.
pub fn find_orphaned_ids(active_ids: &HashSet<String>, usage_ids: &[String]) -> Vec<String> {
    let mut orphaned = Vec::new();
    for id in usage_ids {
        if !id.is_empty() && !active_ids.contains(id) {
            orphaned.push(id.clone());
        }
    }
    orphaned
}

pub(crate) fn validate_library_name(name: &str) -> Result<(), (&'static str, &'static str)> {
    if name.is_empty() {
        return Err(("Invalid library name", "Library name cannot be empty"));
    }
    if name.chars().count() > 50 {
        return Err((
            "Invalid library name",
            "Library name cannot exceed 50 characters",
        ));
    }
    if name.contains('/') || name.contains('\\') {
        return Err((
            "Invalid library name",
            "Library name cannot contain slashes",
        ));
    }
    if name.contains('\0') {
        return Err((
            "Invalid library name",
            "Library name cannot contain null bytes",
        ));
    }
    if name == "." || name == ".." {
        return Err((
            "Invalid library name",
            "Library name cannot contain path traversal sequences",
        ));
    }
    if name.ends_with(".toml") {
        return Err((
            "Invalid library name",
            "Library name cannot end with '.toml' (the extension is added automatically)",
        ));
    }
    Ok(())
}

impl Snippet {
    /// Creates a new snippet with the given description, command, and tags.
    ///
    /// Returns an error if the command or description is empty/whitespace.
    pub fn new(description: String, command: String, tags: Vec<String>) -> SnipResult<Self> {
        if command.trim().is_empty() {
            return Err(SnipError::runtime_error(
                "Empty command",
                Some("Snippet command cannot be empty"),
            ));
        }
        if description.trim().is_empty() {
            return Err(SnipError::runtime_error(
                "Empty description",
                Some("Snippet description cannot be empty"),
            ));
        }
        let now = chrono::Utc::now().timestamp();
        Ok(Self {
            id: String::new(),
            description,
            command,
            tags,
            output: String::new(),
            folders: Vec::new(),
            favorite: false,
            created_at: now,
            updated_at: now,
            device_id: String::new(),
            deleted: false,
        })
    }
}
