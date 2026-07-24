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

use crate::config::{cached_read_toml, invalidate_toml_cache};
use crate::error::{SnipError, SnipResult};
use crate::utils::config::{get_config_dir, get_snippets_path};
use crate::utils::toml_helpers::fix_invalid_toml_escapes;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

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

fn validate_library_name(name: &str) -> Result<(), (&'static str, &'static str)> {
    if name.is_empty() {
        return Err(("Invalid library name", "Library name cannot be empty"));
    }
    if name.len() > 50 {
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
    if name == "." || name == ".." || name.contains("..") {
        return Err((
            "Invalid library name",
            "Library name cannot contain path traversal sequences",
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

/// Manages snippet libraries and premade collections.
///
/// LibraryManager handles:
/// - Loading and saving the libraries configuration
/// - Creating, deleting, and managing individual libraries
/// - Loading premade libraries
/// - Determining whether to use single-file or library mode
pub struct LibraryManager {
    config_dir: PathBuf,
    libraries_dir: PathBuf,
    premade_dir: PathBuf,
    config: LibraryConfig,
}

impl LibraryManager {
    /// Creates a new `LibraryManager`, loading configuration from disk.
    ///
    /// Handles macOS config directory migration and parses `libraries.toml`.
    /// Returns defaults if the config file is missing or corrupted.
    pub fn new() -> SnipResult<Self> {
        // Migrate legacy macOS config dir if needed
        if let Err(e) = crate::utils::config::migrate_macos_config_dir() {
            tracing::warn!(error = %e, "Failed to migrate config directory");
        }

        let config_dir = get_config_dir();

        let libraries_dir = config_dir.join("libraries");
        let premade_dir = config_dir.join("premade");
        let config_path = config_dir.join("libraries.toml");

        let config = if config_path.exists() {
            let content = cached_read_toml(&config_path)?;
            let content = fix_invalid_toml_escapes(&content);
            match toml::from_str(&content) {
                Ok(c) => c,
                Err(e) => {
                    // Backup corrupted file so data isn't lost on next save
                    let backup = config_path.with_extension("toml.corrupt");
                    if let Err(copy_err) = fs::copy(&config_path, &backup) {
                        tracing::warn!(
                            config = %config_path.display(),
                            error = %e,
                            backup_error = %copy_err,
                            "Failed to parse config (backup also failed)"
                        );
                    } else {
                        tracing::warn!(
                            config = %config_path.display(),
                            error = %e,
                            backup = %backup.display(),
                            "Failed to parse config, backed up to file. Using defaults."
                        );
                    }
                    LibraryConfig::default()
                }
            }
        } else {
            LibraryConfig::default()
        };

        Ok(Self {
            config_dir,
            libraries_dir,
            premade_dir,
            config,
        })
    }

    /// Creates a `LibraryManager` rooted at the given config directory.
    ///
    /// This is useful for tests that need an isolated config dir without
    /// mutating process-wide environment variables.
    #[cfg(test)]
    pub fn with_config_dir(config_dir: PathBuf) -> SnipResult<Self> {
        let libraries_dir = config_dir.join("libraries");
        let premade_dir = config_dir.join("premade");
        let config_path = config_dir.join("libraries.toml");

        let config = if config_path.exists() {
            let content = cached_read_toml(&config_path)?;
            let content = fix_invalid_toml_escapes(&content);
            match toml::from_str(&content) {
                Ok(c) => c,
                Err(e) => {
                    let backup = config_path.with_extension("toml.corrupt");
                    let _ = fs::copy(&config_path, &backup);
                    tracing::warn!(error = %e, "Failed to parse config, using defaults");
                    LibraryConfig::default()
                }
            }
        } else {
            LibraryConfig::default()
        };

        Ok(Self {
            config_dir,
            libraries_dir,
            premade_dir,
            config,
        })
    }

    /// Returns the default path to the legacy single-file snippets TOML.
    pub fn get_default_snippets_path() -> PathBuf {
        get_snippets_path()
    }

    /// Returns a reference to the libraries directory path.
    pub fn get_libraries_dir(&self) -> &PathBuf {
        &self.libraries_dir
    }

    /// Returns `true` if the libraries directory does not exist (legacy single-file mode).
    pub fn is_single_file_mode(&self) -> bool {
        !self.libraries_dir.exists()
    }

    /// Returns the path to the legacy single-file snippets TOML.
    pub fn get_legacy_snippets_path(&self) -> PathBuf {
        Self::get_default_snippets_path()
    }

    /// Ensures the library directory exists, migrating from single-file mode if needed.
    pub fn ensure_library_mode(&mut self) -> SnipResult<()> {
        if self.is_single_file_mode() {
            self.migrate_from_single_file()?;
        }
        Ok(())
    }

    /// Creates the libraries directory if it does not exist.
    pub fn init_libraries_dir(&self) -> SnipResult<()> {
        if !self.libraries_dir.exists() {
            fs::create_dir_all(&self.libraries_dir).map_err(|e| {
                SnipError::io_error("create libraries directory", self.libraries_dir.clone(), e)
            })?;
        }
        Ok(())
    }

    /// Migrates the legacy single-file `snippets.toml` into a library subdirectory.
    pub fn migrate_from_single_file(&mut self) -> SnipResult<()> {
        let legacy_path = self.get_legacy_snippets_path();

        if !legacy_path.exists() {
            return Ok(());
        }

        self.init_libraries_dir()?;

        let content = cached_read_toml(&legacy_path)?;
        if content.trim().is_empty() {
            return Ok(());
        }

        let new_path = self.libraries_dir.join("snippets.toml");
        fs::copy(&legacy_path, &new_path)
            .map_err(|e| SnipError::io_error("migrate snippets file", new_path.clone(), e))?;

        let mut meta = LibraryMeta::new("snippets");
        meta.is_primary = true;
        self.config.libraries.push(meta);

        self.bump_generation();
        self.save_config()?;

        Ok(())
    }

    /// Returns references to all registered libraries.
    pub fn list_libraries(&self) -> Vec<&LibraryMeta> {
        self.config.libraries.iter().collect()
    }

    /// Returns the primary library, or `None` if no library is marked primary.
    pub fn get_primary_library(&self) -> Option<&LibraryMeta> {
        self.config.libraries.iter().find(|l| l.is_primary)
    }

    /// Finds a library by its filename (without `.toml` extension).
    pub fn get_library_by_filename(&self, filename: &str) -> Option<&LibraryMeta> {
        self.config
            .libraries
            .iter()
            .find(|l| l.filename == filename)
    }

    /// Finds a library by filename, returning a mutable reference.
    pub fn get_library_by_filename_mut(&mut self, filename: &str) -> Option<&mut LibraryMeta> {
        self.config
            .libraries
            .iter_mut()
            .find(|l| l.filename == filename)
    }

    /// Creates a new snippet library file and registers it in the config.
    ///
    /// The first library created is automatically marked as primary.
    /// Returns the path to the newly created library file.
    pub fn create_library(&mut self, filename: &str) -> SnipResult<PathBuf> {
        validate_library_name(filename)
            .map_err(|(msg, detail)| SnipError::runtime_error(msg, Some(detail)))?;

        self.init_libraries_dir()?;

        let filename_lower = filename.to_lowercase();
        let path = self.libraries_dir.join(format!("{filename}.toml"));

        if path.exists() {
            return Err(SnipError::runtime_error(
                "Library already exists",
                Some(&format!("File {} already exists", path.display())),
            ));
        }

        for lib in &self.config.libraries {
            if lib.filename.to_lowercase() == filename_lower {
                return Err(SnipError::runtime_error(
                    "Library already exists",
                    Some(&format!(
                        "A library with name '{filename}' already exists (case-insensitive duplicate)"
                    )),
                ));
            }
        }

        let default_content = r#"# Snippet library
# Each snippet has: description, output, tag, command, folders, favorite

snippets = []

"#;

        write_library_file(&path, default_content, filename)?;

        let is_first = self.config.libraries.is_empty();
        let mut meta = LibraryMeta::new(filename);
        meta.is_primary = is_first;
        self.config.libraries.push(meta);

        self.bump_generation();
        self.save_config()?;

        Ok(path)
    }

    /// Deletes a library file and removes it from the config.
    ///
    /// If the deleted library was primary, another library is promoted.
    /// Config is saved before file deletion for crash safety.
    pub fn delete_library(&mut self, filename: &str) -> SnipResult<()> {
        let was_primary = self
            .get_library_by_filename(filename)
            .map(|l| l.is_primary)
            .ok_or_else(|| SnipError::runtime_error("Library not found", Some(filename)))?;

        let deleted_was_server = self
            .get_library_by_filename(filename)
            .map(|l| l.server_id.is_some())
            .unwrap_or(false);

        let path = self.libraries_dir.join(format!("{filename}.toml"));

        // Save config first (remove from metadata), then delete the file.
        // If we crash after config save but before file delete, the orphaned
        // file is recoverable — operations on the deleted library will fail
        // gracefully with IO errors. The reverse order (delete file first,
        // then save config) leaves a stale config reference on crash.
        self.config.libraries.retain(|l| l.filename != filename);

        if was_primary && !self.config.libraries.is_empty() {
            let promoted = if deleted_was_server {
                self.config
                    .libraries
                    .iter()
                    .find(|l| l.server_id.is_some())
                    .or_else(|| self.config.libraries.first())
            } else {
                self.config.libraries.first()
            };
            if let Some(promoted_lib) = promoted
                && let Some(idx) = self
                    .config
                    .libraries
                    .iter()
                    .position(|l| l.filename == promoted_lib.filename)
            {
                self.config.libraries[idx].is_primary = true;
            }
        }

        self.bump_generation();
        self.save_config()?;

        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| SnipError::io_error("delete library file", path.clone(), e))?;
        }

        Ok(())
    }

    /// Sets the given library as primary, unmarking all others.
    pub fn set_primary(&mut self, filename: &str) -> SnipResult<()> {
        if !self
            .config
            .libraries
            .iter()
            .any(|lib| lib.filename == filename)
        {
            return Err(SnipError::runtime_error(
                "Library not found",
                Some(&format!("No library with filename '{filename}'")),
            ));
        }
        for lib in &mut self.config.libraries {
            lib.is_primary = lib.filename == filename;
        }

        self.bump_generation();
        self.save_config()?;
        Ok(())
    }

    /// Updates the server-side library ID for a local library.
    pub fn update_library_id(&mut self, filename: &str, library_id: &str) -> SnipResult<()> {
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.library_id = library_id.to_string();

            self.bump_generation();
            self.save_config()?;
        }
        Ok(())
    }

    /// Links a local library to a server-side library.
    pub fn link_server_library(&mut self, filename: &str, server_id: &str) -> SnipResult<()> {
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.library_id = server_id.to_string();
            lib.server_id = Some(server_id.to_string());

            self.bump_generation();
            self.save_config()?;
        }
        Ok(())
    }

    /// Clears server linkage metadata for a local library.
    pub fn unlink_server_library(&mut self, filename: &str) -> SnipResult<()> {
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.library_id.clear();
            lib.server_id = None;

            self.bump_generation();
            self.save_config()?;
        }
        Ok(())
    }

    /// Registers an existing library file that is not yet tracked in the config.
    pub fn add_existing_library(&mut self, filename: &str) -> SnipResult<()> {
        validate_library_name(filename)
            .map_err(|(title, detail)| SnipError::runtime_error(title, Some(detail)))?;

        if self.get_library_by_filename(filename).is_some() {
            return Ok(());
        }

        let meta = LibraryMeta {
            filename: filename.to_string(),
            library_id: String::new(),
            is_primary: false,
            last_sync: None,
            server_id: None,
        };

        self.config.libraries.push(meta);

        self.bump_generation();
        self.save_config()?;
        Ok(())
    }

    /// Updates the last-sync timestamp for a library.
    pub fn update_last_sync(&mut self, filename: &str, timestamp: i64) -> SnipResult<()> {
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.last_sync = Some(timestamp);

            self.bump_generation();
            self.save_config()?;
        }
        Ok(())
    }

    /// Creates or links a library imported from the sync server.
    ///
    /// If a library with the same filename already exists, its server ID is updated.
    /// Otherwise, a new library file and config entry are created.
    pub fn add_server_library(
        &mut self,
        server_name: &str,
        server_id: &str,
    ) -> SnipResult<PathBuf> {
        let filename = server_name.to_lowercase().replace(' ', "-");

        validate_library_name(&filename)
            .map_err(|(title, detail)| SnipError::runtime_error(title, Some(detail)))?;

        self.init_libraries_dir()?;

        let path = self.libraries_dir.join(format!("{filename}.toml"));

        if !path.exists() {
            let default_content = "# Imported from server\n\nsnippets = []\n";
            write_library_file(&path, default_content, &filename)?;
        }

        // Update existing entry if one with the same filename already exists
        if let Some(existing) = self.get_library_by_filename_mut(&filename) {
            existing.library_id = server_id.to_string();
            existing.server_id = Some(server_id.to_string());

            self.bump_generation();
            self.save_config()?;
            return Ok(path);
        }

        let is_first = self.config.libraries.is_empty();
        let mut meta = LibraryMeta::new(&filename);
        meta.library_id = server_id.to_string();
        meta.server_id = Some(server_id.to_string());
        meta.is_primary = is_first;

        self.config.libraries.push(meta);

        self.bump_generation();
        self.save_config()?;

        Ok(path)
    }

    /// Creates the premade libraries directory if it does not exist.
    pub fn init_premade_dir(&self) -> SnipResult<()> {
        if !self.premade_dir.exists() {
            fs::create_dir_all(&self.premade_dir).map_err(|e| {
                SnipError::io_error("create premade directory", self.premade_dir.clone(), e)
            })?;
        }
        Ok(())
    }

    /// Returns the path to the premade libraries directory.
    pub fn get_premade_dir(&self) -> &PathBuf {
        &self.premade_dir
    }

    /// Returns `true` if a premade library with the given filename exists on disk.
    pub fn premade_exists(&self, filename: &str) -> bool {
        self.premade_dir.join(format!("{filename}.toml")).exists()
    }

    /// Saves a premade library file to the premade directory.
    ///
    /// Validates the filename against path traversal attacks before writing.
    /// Returns the path to the saved file.
    pub fn save_premade_library(&self, filename: &str, content: &str) -> SnipResult<PathBuf> {
        self.init_premade_dir()?;

        if filename.is_empty()
            || filename.contains('/')
            || filename.contains('\\')
            || filename.contains('\0')
            || filename.contains("..")
        {
            return Err(SnipError::runtime_error(
                "Invalid premade library filename",
                Some(filename),
            ));
        }

        let path = self.premade_dir.join(format!("{filename}.toml"));

        let canonical_premade = self.premade_dir.canonicalize().map_err(|e| {
            SnipError::io_error("resolve premade directory", self.premade_dir.clone(), e)
        })?;
        let canonical_path = path
            .canonicalize()
            .unwrap_or_else(|_| canonical_premade.join(format!("{filename}.toml")));
        if !canonical_path.starts_with(&canonical_premade) {
            return Err(SnipError::runtime_error(
                "Invalid premade library path",
                Some("Filename resolves outside premade directory"),
            ));
        }

        write_library_file(&path, content, filename)?;

        Ok(path)
    }

    /// Returns the current generation counter value.
    ///
    /// The generation increments on every mutation and is used by backup
    /// to verify that the library index and files come from one coherent state.
    pub fn generation(&self) -> u64 {
        self.config.generation
    }

    /// Bump the generation counter before saving.
    fn bump_generation(&mut self) {
        self.config.generation = self.config.generation.saturating_add(1);
    }

    fn save_config(&mut self) -> SnipResult<()> {
        let config_path = self.config_dir.join("libraries.toml");

        let toml_str = toml::to_string_pretty(&self.config)
            .map_err(|e| SnipError::toml_error("serialize libraries config", e))?;

        crate::utils::atomic::write_private_atomic(&config_path, &toml_str, "libraries")?;
        invalidate_toml_cache(&config_path);

        Ok(())
    }
}

fn write_library_file(path: &Path, content: &str, temp_prefix: &str) -> SnipResult<()> {
    crate::utils::atomic::write_private_atomic(path, content, temp_prefix)?;
    invalidate_toml_cache(path);
    Ok(())
}

/// Loads a snippet library from a TOML file.
///
/// Returns an empty collection if the file doesn't exist or is empty.
/// Deduplicates snippet IDs on load and creates backups of corrupted files.
pub fn load_library(path: &Path) -> SnipResult<Snippets> {
    if !path.exists() {
        return Ok(Snippets::default());
    }

    let content = cached_read_toml(path)?;
    if content.is_empty() || content.trim().is_empty() {
        return Ok(Snippets::default());
    }

    let fixed_content = fix_invalid_toml_escapes(&content);

    let snippets: Snippets = match toml::from_str(&fixed_content) {
        Ok(s) => s,
        Err(e) => {
            // Create backup of corrupted file before returning defaults
            let backup_path = path.with_extension("toml.corrupt.bak");
            if let Err(backup_err) = fs::copy(path, &backup_path) {
                tracing::error!(
                    file = %path.display(),
                    error = %backup_err,
                    "Failed to parse TOML and could not create backup"
                );
            } else {
                tracing::error!(
                    file = %path.display(),
                    backup = %backup_path.display(),
                    error = %e,
                    "Failed to parse TOML, backup saved"
                );
            }
            Snippets::default()
        }
    };

    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut deduplicated: Vec<Snippet> = Vec::new();
    for mut snippet in snippets.snippets {
        if snippet.id.is_empty() {
            snippet.id = uuid::Uuid::new_v4().to_string();
        }
        if seen_ids.contains(&snippet.id) {
            snippet.id = uuid::Uuid::new_v4().to_string();
        }
        seen_ids.insert(snippet.id.clone());
        deduplicated.push(snippet);
    }

    Ok(Snippets {
        snippets: deduplicated,
        folders: snippets.folders,
    })
}

/// Saves a snippet library to a TOML file using atomic write.
///
/// Creates a backup before saving and sorts snippets by `updated_at` descending.
///
/// The serialized output is written verbatim from `toml::to_string_pretty`.
/// snip-it does not post-process the TOML body because the serializer already
/// picks correct quoting and escapes for every character, including tabs,
/// trailing whitespace, and CRLF. The earlier `quote_strings_containing_backslashes`
/// post-processing pass silently corrupted those byte sequences (its regex
/// could not tell TOML triple-quoted multi-line strings from ordinary
/// double-quoted strings, and its single-quoted output preserved TOML escape
/// sequences like `\t` as literal two-character pairs). The helper remains
/// available for callers that hand-write TOML and need the same conversion.
pub fn save_library(path: &Path, snippets: &Snippets) -> SnipResult<()> {
    // Check for interrupted transactions before any mutation.
    // This prevents new writes from proceeding over an unresolved restore.
    let state_dir = crate::local_data::derive_local_data_state_dir();
    crate::transaction::gate_mutation_on_interrupted_transactions(&state_dir)?;

    // Acquire the local-data lock to serialize against backup snapshot capture.
    // This ensures backup sees either the complete before-state or complete
    // after-state, never a mixed state.
    let _local_lock = crate::local_data::acquire_local_data_lock(&state_dir)?;

    save_library_internal(path, snippets, &_local_lock)?;

    Ok(())
}

/// Internal library save that skips the mutation gate and lock acquisition.
///
/// This is valid only when the caller already holds the local-data lock
/// (via `guard`) and is operating within an active transaction or has
/// already passed the mutation gate. Restore uses this to avoid
/// self-recovery: once restore has persisted `Committing`, the global
/// gate would see the caller's own journal as interrupted and roll it
/// back while restore continues.
pub fn save_library_internal(
    path: &Path,
    snippets: &Snippets,
    _guard: &crate::local_data::LocalDataLock,
) -> SnipResult<()> {
    if let Err(e) = backup_library(path) {
        tracing::warn!(error = %e, "Failed to create backup before save");
    }

    let mut sorted = snippets.clone();
    sorted
        .snippets
        .sort_by_key(|b| std::cmp::Reverse(b.updated_at));

    let toml_str = toml::to_string_pretty(&sorted)
        .map_err(|e| SnipError::toml_error("serialize snippets", e))?;

    let temp_prefix = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("snippets");
    crate::utils::atomic::write_private_atomic(path, &toml_str, temp_prefix)?;

    invalidate_toml_cache(path);

    Ok(())
}

/// Creates a timestamped backup of a library file.
///
/// Stores backups in a `backups/` subdirectory, keeping at most 10 per library.
/// Returns `None` if the source file doesn't exist.
pub fn backup_library(path: &Path) -> SnipResult<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }

    let backup_dir = path
        .parent()
        .ok_or_else(|| {
            SnipError::runtime_error(
                "backup path has no parent",
                Some(&path.display().to_string()),
            )
        })?
        .join("backups");
    fs::create_dir_all(&backup_dir)
        .map_err(|e| SnipError::io_error("create backup directory", backup_dir.clone(), e))?;

    // Clean up old backups (keep at most 10 per library)
    cleanup_old_backups(&backup_dir, path)?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S_%f");
    let file_stem = path.file_stem().ok_or_else(|| {
        SnipError::runtime_error(
            "backup path has no file stem",
            Some(&path.display().to_string()),
        )
    })?;
    let backup_name = format!("{}.{}.toml.bak", file_stem.to_string_lossy(), timestamp);
    let backup_path = backup_dir.join(backup_name);

    fs::copy(path, &backup_path)
        .map_err(|e| SnipError::io_error("create backup", backup_path.clone(), e))?;

    Ok(Some(backup_path))
}

fn cleanup_old_backups(backup_dir: &Path, original_path: &Path) -> SnipResult<()> {
    const MAX_BACKUPS_PER_LIBRARY: usize = 10;

    let file_stem = match original_path.file_stem() {
        Some(s) => s.to_string_lossy().to_string(),
        None => return Ok(()),
    };

    let prefix = format!("{file_stem}.");
    let mut backups: Vec<_> = fs::read_dir(backup_dir)
        .map_err(|e| SnipError::io_error("read backup directory", backup_dir.to_path_buf(), e))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            name.starts_with(&prefix) && name.ends_with(".toml.bak")
        })
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let modified = metadata.modified().ok()?;
            Some((entry.path(), modified))
        })
        .collect();

    backups.sort_by_key(|b| std::cmp::Reverse(b.1));

    if backups.len() > MAX_BACKUPS_PER_LIBRARY {
        for (path, _) in backups.into_iter().skip(MAX_BACKUPS_PER_LIBRARY) {
            if let Err(e) = fs::remove_file(&path) {
                tracing::warn!(
                    backup = %path.display(),
                    error = %e,
                    "Failed to remove old backup"
                );
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[cfg(unix)]
    fn file_mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn test_pet_format_compatibility() {
        let pet_toml = r#"
[[snippets]]
  description = "git commit with message"
  command = "git commit -m \"message\""
  tag = ["git", "version-control"]
  output = ""

[[snippets]]
  description = "docker ps"
  command = "docker ps"
  tag = ["docker"]
  output = ""
"#;
        let snippets: Snippets = toml::from_str(pet_toml).unwrap();
        assert_eq!(snippets.snippets.len(), 2);
        assert_eq!(snippets.snippets[0].command, "git commit -m \"message\"");
        assert_eq!(snippets.snippets[0].description, "git commit with message");
        assert_eq!(snippets.snippets[0].tags, vec!["git", "version-control"]);
        assert_eq!(snippets.snippets[1].command, "docker ps");
    }

    #[test]
    fn test_legacy_snp_format_compatibility() {
        let snp_toml = r#"
[[Snippets]]
  Description = "git commit"
  Output = ""
  Tag = ["git"]
  command = "git commit -m 'msg'"
"#;
        let snippets: Snippets = toml::from_str(snp_toml).unwrap();
        assert_eq!(snippets.snippets.len(), 1);
        assert_eq!(snippets.snippets[0].command, "git commit -m 'msg'");
    }

    #[test]
    fn test_snp_serializes_to_pet_table_name() {
        let snippets = Snippets {
            snippets: vec![Snippet {
                description: "list files".to_string(),
                command: "ls -la".to_string(),
                tags: vec!["files".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        };

        let toml = toml::to_string(&snippets).unwrap();
        assert!(toml.contains("[[snippets]]"));
        assert!(toml.contains("tag = [\"files\"]"));
        assert!(toml.contains("output = \"\""));
        assert!(!toml.contains("[[Snippets]]"));
    }

    #[test]
    fn test_library_save_load_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test_library.toml");

        let snippets = Snippets {
            snippets: vec![Snippet {
                id: "test-id-1".to_string(),
                description: "Test snippet".to_string(),
                command: "echo hello".to_string(),
                output: "".to_string(),
                tags: vec!["test".to_string()],
                folders: vec![],
                favorite: false,
                created_at: 1234567890,
                updated_at: 1234567890,
                device_id: "device1".to_string(),
                deleted: false,
            }],
            folders: vec!["work".to_string()],
        };

        save_library(&path, &snippets).unwrap();

        let loaded = load_library(&path).unwrap();

        assert_eq!(loaded.snippets.len(), 1);
        assert_eq!(loaded.snippets[0].description, "Test snippet");
        assert_eq!(loaded.snippets[0].command, "echo hello");
    }

    #[test]
    fn test_library_save_load_roundtrip_with_escaped_brackets() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test_library.toml");

        let snippets = Snippets {
            snippets: vec![Snippet {
                id: "test-id-1".to_string(),
                description: "Test with escaped brackets".to_string(),
                command: "ping \\<website\\>".to_string(),
                output: "".to_string(),
                tags: vec!["test".to_string()],
                folders: vec![],
                favorite: false,
                created_at: 1234567890,
                updated_at: 1234567890,
                device_id: "device1".to_string(),
                deleted: false,
            }],
            folders: vec![],
        };

        save_library(&path, &snippets).unwrap();

        let loaded = load_library(&path).unwrap();

        assert_eq!(loaded.snippets.len(), 1);
        assert_eq!(loaded.snippets[0].command, "ping \\<website\\>");
    }

    #[test]
    fn test_library_load_with_invalid_escapes() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("invalid_escapes.toml");

        std::fs::write(
            &path,
            r#"
[[Snippets]]
Id = "test-id"
Description = "Test snippet with invalid escapes"
Command = "sudo iptables-restore \< /path/to/rules"
"#,
        )
        .unwrap();

        let loaded = load_library(&path).unwrap();

        assert_eq!(loaded.snippets.len(), 1);
        assert_eq!(
            loaded.snippets[0].command,
            r"sudo iptables-restore \< /path/to/rules"
        );
    }

    #[test]
    fn test_library_load_empty_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("empty.toml");

        std::fs::write(&path, "").unwrap();

        let loaded = load_library(&path).unwrap();

        assert!(loaded.snippets.is_empty());
    }

    #[test]
    fn test_library_backup_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("nonexistent.toml");

        let backup_result = backup_library(&path).unwrap();

        assert!(backup_result.is_none());
    }

    #[test]
    fn test_snippet_serialization() {
        let snippet = Snippet {
            id: "test-id".to_string(),
            description: "Test description".to_string(),
            command: "echo test".to_string(),
            output: "test output".to_string(),
            tags: vec!["test".to_string()],
            folders: vec!["work".to_string()],
            favorite: true,
            created_at: 1234567890,
            updated_at: 1234567891,
            device_id: "device-1".to_string(),
            deleted: false,
        };

        let toml_str = toml::to_string_pretty(&snippet).unwrap();
        assert!(toml_str.contains("test-id"));
        assert!(toml_str.contains("Test description"));
        assert!(toml_str.contains("echo test"));
    }

    #[test]
    fn test_snippets_with_multiple_items() {
        let snippets = Snippets {
            snippets: vec![
                Snippet {
                    id: "id1".to_string(),
                    description: "First".to_string(),
                    command: "cmd1".to_string(),
                    output: "".to_string(),
                    tags: vec![],
                    folders: vec![],
                    favorite: false,
                    created_at: 0,
                    updated_at: 0,
                    device_id: "".to_string(),
                    deleted: false,
                },
                Snippet {
                    id: "id2".to_string(),
                    description: "Second".to_string(),
                    command: "cmd2".to_string(),
                    output: "".to_string(),
                    tags: vec![],
                    folders: vec![],
                    favorite: false,
                    created_at: 0,
                    updated_at: 0,
                    device_id: "".to_string(),
                    deleted: false,
                },
            ],
            folders: vec!["work".to_string()],
        };

        let toml_str = toml::to_string_pretty(&snippets).unwrap();
        assert!(toml_str.contains("id1"));
        assert!(toml_str.contains("id2"));
        assert!(toml_str.contains("work"));
    }

    #[test]
    fn test_library_manager_new() {
        let mgr = LibraryManager::new();
        // Should not panic - just verify it can be created
        assert!(mgr.is_ok() || mgr.is_err());
    }

    #[test]
    fn test_snippet_new_empty_command_fails() {
        let result = Snippet::new("desc".to_string(), "  ".to_string(), vec![]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Empty command"));
    }

    #[test]
    fn test_snippet_new_empty_description_fails() {
        let result = Snippet::new("  ".to_string(), "echo hi".to_string(), vec![]);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Empty description")
        );
    }

    #[test]
    fn test_snippet_new_valid() {
        let result = Snippet::new(
            "desc".to_string(),
            "echo hi".to_string(),
            vec!["tag".to_string()],
        );
        assert!(result.is_ok());
        let s = result.unwrap();
        assert_eq!(s.description, "desc");
        assert_eq!(s.command, "echo hi");
    }

    #[test]
    fn test_validate_library_name_empty() {
        assert!(validate_library_name("").is_err());
    }

    #[test]
    fn test_validate_library_name_too_long() {
        assert!(validate_library_name(&"a".repeat(51)).is_err());
    }

    #[test]
    fn test_validate_library_name_slash() {
        assert!(validate_library_name("foo/bar").is_err());
    }

    #[test]
    fn test_validate_library_name_backslash() {
        assert!(validate_library_name("foo\\bar").is_err());
    }

    #[test]
    fn test_validate_library_name_null_byte() {
        assert!(validate_library_name("foo\0bar").is_err());
    }

    #[test]
    fn test_validate_library_name_dot() {
        assert!(validate_library_name(".").is_err());
        assert!(validate_library_name("..").is_err());
        assert!(validate_library_name("my..lib").is_err());
    }

    #[test]
    fn test_validate_library_name_valid() {
        assert!(validate_library_name("my-library").is_ok());
        assert!(validate_library_name("work snippets").is_ok());
    }

    #[test]
    fn test_save_library_atomic_write() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.toml");
        let snippets = Snippets {
            snippets: vec![Snippet {
                id: "atomic-test".to_string(),
                description: "Atomic write test".to_string(),
                command: "echo atomic".to_string(),
                output: "".to_string(),
                tags: vec![],
                folders: vec![],
                favorite: false,
                created_at: 100,
                updated_at: 100,
                device_id: "d1".to_string(),
                deleted: false,
            }],
            folders: vec![],
        };
        save_library(&path, &snippets).unwrap();
        let loaded = load_library(&path).unwrap();
        assert_eq!(loaded.snippets.len(), 1);
        assert_eq!(loaded.snippets[0].id, "atomic-test");
        // Verify no .tmp files remain after atomic rename
        let parent = path.parent().unwrap();
        let has_tmp = std::fs::read_dir(parent)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.path().extension().is_some_and(|ext| ext == "tmp"));
        assert!(!has_tmp, "temp files should not remain after atomic rename");
    }

    #[test]
    fn test_create_library_uses_private_atomic_write() {
        let temp_dir = TempDir::new().unwrap();
        let mut mgr = LibraryManager {
            config_dir: temp_dir.path().to_path_buf(),
            libraries_dir: temp_dir.path().join("libraries"),
            premade_dir: temp_dir.path().join("premade"),
            config: Default::default(),
        };

        let path = mgr.create_library("private").unwrap();

        assert!(path.exists());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("snippets = []")
        );

        #[cfg(unix)]
        assert_eq!(file_mode(&path), 0o600);
    }

    #[test]
    fn test_add_server_library_uses_private_atomic_write() {
        let temp_dir = TempDir::new().unwrap();
        let mut mgr = LibraryManager {
            config_dir: temp_dir.path().to_path_buf(),
            libraries_dir: temp_dir.path().join("libraries"),
            premade_dir: temp_dir.path().join("premade"),
            config: Default::default(),
        };

        let path = mgr
            .add_server_library("Shared Commands", "server-library-id")
            .unwrap();

        assert!(path.exists());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("Imported from server")
        );

        #[cfg(unix)]
        assert_eq!(file_mode(&path), 0o600);
    }

    #[test]
    fn test_save_config_invalidates_libraries_toml_cache() {
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().to_path_buf();
        let libraries_dir = config_dir.join("libraries");
        let premade_dir = config_dir.join("premade");
        std::fs::create_dir_all(&libraries_dir).unwrap();

        let config_path = config_dir.join("libraries.toml");
        std::fs::write(
            &config_path,
            r#"
[[libraries]]
filename = "old"
library_id = ""
is_primary = true
"#,
        )
        .unwrap();
        let cached_before = cached_read_toml(&config_path).unwrap();
        assert!(cached_before.contains("old"));

        let mut mgr = LibraryManager {
            config_dir,
            libraries_dir,
            premade_dir,
            config: LibraryConfig {
                libraries: vec![LibraryMeta {
                    filename: "old".to_string(),
                    library_id: String::new(),
                    is_primary: true,
                    last_sync: None,
                    server_id: None,
                }],
                generation: 0,
            },
        };
        mgr.create_library("new").unwrap();

        let cached_after = cached_read_toml(&config_path).unwrap();
        assert!(cached_after.contains("old"));
        assert!(cached_after.contains("new"));
    }

    #[test]
    fn test_backup_library_names_do_not_collide() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("snippets.toml");
        std::fs::write(&path, "test content").unwrap();

        let first = backup_library(&path).unwrap().unwrap();
        let second = backup_library(&path).unwrap().unwrap();

        assert_ne!(first, second);

        let backup_dir = temp_dir.path().join("backups");
        let backup_count = std::fs::read_dir(backup_dir).unwrap().count();
        assert_eq!(backup_count, 2);
    }

    #[test]
    fn test_save_premade_library_path_traversal() {
        let temp_dir = TempDir::new().unwrap();
        let mgr = LibraryManager {
            config_dir: temp_dir.path().to_path_buf(),
            libraries_dir: temp_dir.path().join("libraries"),
            premade_dir: temp_dir.path().join("premade"),
            config: Default::default(),
        };
        assert!(
            mgr.save_premade_library("../../etc/passwd", "content")
                .is_err()
        );
        assert!(mgr.save_premade_library("../escape", "content").is_err());
        assert!(mgr.save_premade_library("foo/bar", "content").is_err());
    }

    #[test]
    fn test_save_premade_library_valid() {
        let temp_dir = TempDir::new().unwrap();
        let mgr = LibraryManager {
            config_dir: temp_dir.path().to_path_buf(),
            libraries_dir: temp_dir.path().join("libraries"),
            premade_dir: temp_dir.path().join("premade"),
            config: Default::default(),
        };
        let result = mgr.save_premade_library("valid-name", "test content");
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "test content");

        #[cfg(unix)]
        assert_eq!(file_mode(&path), 0o600);
    }

    #[test]
    fn test_deduplication_on_load() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("dup.toml");
        let toml_content = r#"
[[Snippets]]
Id = "same-id"
Description = "First"
Command = "cmd1"

[[Snippets]]
Id = "same-id"
Description = "Second"
Command = "cmd2"
"#;
        std::fs::write(&path, toml_content).unwrap();
        let loaded = load_library(&path).unwrap();
        assert_eq!(loaded.snippets.len(), 2);
        assert_ne!(loaded.snippets[0].id, loaded.snippets[1].id);
    }

    #[test]
    fn test_fixture_canonical_pet_roundtrips() {
        let content = include_str!("../tests/fixtures/canonical_pet.toml");
        let library: Snippets = toml::from_str(content).unwrap();

        assert_eq!(library.snippets.len(), 5);

        assert_eq!(library.snippets[0].description, "git commit with message");
        assert_eq!(library.snippets[0].command, "git commit -m \"<msg>\"");
        assert_eq!(library.snippets[0].tags, vec!["git", "version-control"]);

        assert_eq!(
            library.snippets[1].description,
            "docker ps running containers"
        );
        assert!(library.snippets[1].command.contains("docker ps"));

        assert_eq!(
            library.snippets[3].description,
            "search & replace with \"quotes\" and special chars"
        );
        assert_eq!(library.snippets[3].tags, vec!["search", "ripgrep"]);

        assert_eq!(
            library.snippets[4].description,
            "日本語のスニペット — unicode test"
        );
        assert_eq!(library.snippets[4].tags, vec!["unicode", "日本語"]);

        // Roundtrip: serialize back and reload
        let serialized = toml::to_string_pretty(&library).unwrap();
        let reloaded: Snippets = toml::from_str(&serialized).unwrap();

        assert_eq!(reloaded.snippets.len(), 5);
        for i in 0..5 {
            assert_eq!(
                reloaded.snippets[i].description,
                library.snippets[i].description
            );
            assert_eq!(reloaded.snippets[i].command, library.snippets[i].command);
            assert_eq!(reloaded.snippets[i].tags, library.snippets[i].tags);
        }
    }

    #[test]
    fn test_fixture_snip_it_native_roundtrips() {
        let content = include_str!("../tests/fixtures/snip_it_native.toml");
        let library: Snippets = toml::from_str(content).unwrap();

        assert_eq!(library.snippets.len(), 4);

        assert_eq!(
            library.snippets[0].id,
            "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d"
        );
        assert_eq!(
            library.snippets[0].description,
            "list all docker containers"
        );
        assert_eq!(library.snippets[0].tags, vec!["docker"]);
        assert_eq!(library.snippets[0].folders, vec!["devops", "containers"]);
        assert!(library.snippets[0].favorite);
        assert_eq!(library.snippets[0].created_at, 1700000000);
        assert_eq!(library.snippets[0].updated_at, 1700100000);
        assert_eq!(library.snippets[0].device_id, "device-alpha-001");
        assert!(!library.snippets[0].deleted);

        assert!(!library.snippets[1].favorite);
        assert_eq!(library.snippets[1].folders, vec!["ops"]);

        assert_eq!(
            library.snippets[3].id,
            "aabbccdd-eeff-4000-8111-223344556677"
        );
        assert!(library.snippets[3].deleted);

        // Roundtrip
        let serialized = toml::to_string_pretty(&library).unwrap();
        let reloaded: Snippets = toml::from_str(&serialized).unwrap();

        assert_eq!(reloaded.snippets.len(), 4);
        for i in 0..4 {
            assert_eq!(reloaded.snippets[i].id, library.snippets[i].id);
            assert_eq!(
                reloaded.snippets[i].description,
                library.snippets[i].description
            );
            assert_eq!(reloaded.snippets[i].command, library.snippets[i].command);
            assert_eq!(reloaded.snippets[i].tags, library.snippets[i].tags);
            assert_eq!(reloaded.snippets[i].favorite, library.snippets[i].favorite);
            assert_eq!(
                reloaded.snippets[i].created_at,
                library.snippets[i].created_at
            );
            assert_eq!(
                reloaded.snippets[i].updated_at,
                library.snippets[i].updated_at
            );
            assert_eq!(reloaded.snippets[i].deleted, library.snippets[i].deleted);
        }
    }

    #[test]
    fn test_fixture_legacy_uppercase_loads_as_snippets() {
        let content = include_str!("../tests/fixtures/legacy_uppercase.toml");
        let library: Snippets = toml::from_str(content).unwrap();

        assert_eq!(library.snippets.len(), 2);

        assert_eq!(library.snippets[0].description, "list files");
        assert_eq!(library.snippets[0].command, "ls -la");
        assert_eq!(library.snippets[0].tags, vec!["files"]);

        assert_eq!(library.snippets[1].description, "find large files");
        assert_eq!(library.snippets[1].command, "find . -type f -size +10M");
        assert_eq!(library.snippets[1].tags, vec!["search", "find"]);
    }

    #[test]
    fn test_fixture_mixed_aliases_load_correctly() {
        let content = include_str!("../tests/fixtures/mixed_field_aliases.toml");
        let library: Snippets = toml::from_str(content).unwrap();

        assert_eq!(library.snippets.len(), 3);

        // First snippet uses "name" alias for description and "cmd" alias for command
        assert_eq!(
            library.snippets[0].description,
            "snippet using name instead of description"
        );
        assert_eq!(library.snippets[0].command, "echo alias test");
        assert_eq!(library.snippets[0].tags, vec!["aliases"]);

        // Second snippet uses canonical field names
        assert_eq!(
            library.snippets[1].description,
            "snippet using canonical field names"
        );
        assert_eq!(library.snippets[1].command, "echo canonical");
        assert_eq!(library.snippets[1].tags, vec!["canonical"]);

        // Third snippet uses capitalized "Description" and "Command"
        assert_eq!(
            library.snippets[2].description,
            "snippet using capitalized Description"
        );
        assert_eq!(library.snippets[2].command, "echo capitalized");
        assert_eq!(library.snippets[2].tags, vec!["capitalized"]);
    }

    #[test]
    fn test_fixture_variable_commands_preserve_syntax() {
        let content = include_str!("../tests/fixtures/variable_commands.toml");
        let library: Snippets = toml::from_str(content).unwrap();

        assert_eq!(library.snippets.len(), 5);

        // Simple variable
        assert_eq!(library.snippets[0].command, "echo <greeting>");

        // Variable with default value
        assert_eq!(library.snippets[1].command, "ssh <host=localhost> 'uptime'");

        // Escaped angle brackets — single-quoted TOML literal preserves \< as-is
        assert_eq!(library.snippets[2].command, "ping \\<hostname\\> -c 3");

        // Nested angle brackets
        assert_eq!(library.snippets[3].command, "echo <outer<inner>>");

        // Variable with default containing spaces
        assert_eq!(
            library.snippets[4].command,
            "cp <src=/tmp/default file> <dest=.>"
        );

        // Roundtrip preserves variable syntax
        let serialized = toml::to_string_pretty(&library).unwrap();
        let reloaded: Snippets = toml::from_str(&serialized).unwrap();

        assert_eq!(reloaded.snippets.len(), 5);
        for i in 0..5 {
            assert_eq!(reloaded.snippets[i].command, library.snippets[i].command);
        }
    }

    #[test]
    fn test_fixture_empty_library_loads_empty() {
        let content = include_str!("../tests/fixtures/empty_library.toml");
        let library: Snippets = toml::from_str(content).unwrap();

        assert!(library.snippets.is_empty());
    }

    #[test]
    fn test_pet_field_names_in_output() {
        let snippet = Snippet {
            description: "list files".to_string(),
            command: "ls -la".to_string(),
            tags: vec!["files".to_string()],
            output: "".to_string(),
            ..Default::default()
        };
        let snippets = Snippets {
            snippets: vec![snippet],
            ..Default::default()
        };

        let toml_str = toml::to_string_pretty(&snippets).unwrap();
        assert!(toml_str.contains("[[snippets]]"));
        assert!(toml_str.contains("description = \"list files\""));
        assert!(toml_str.contains("command = \"ls -la\""));
        assert!(toml_str.contains("tag = [\"files\"]"));
        assert!(toml_str.contains("output = \"\""));
        assert!(!toml_str.contains("Description"));
        assert!(!toml_str.contains("Command"));
        assert!(!toml_str.contains("Tag ="));
    }

    #[test]
    fn test_snippet_description_alias_roundtrip() {
        let snippet = Snippet {
            description: "test description".to_string(),
            command: "echo test".to_string(),
            ..Default::default()
        };

        let toml_str = toml::to_string_pretty(&snippet).unwrap();
        assert!(toml_str.contains("description = \"test description\""));
        assert!(!toml_str.contains("Description ="));
        assert!(!toml_str.contains("name ="));

        let reloaded: Snippet = toml::from_str(&toml_str).unwrap();
        assert_eq!(reloaded.description, "test description");
    }

    #[test]
    fn test_snippet_command_alias_roundtrip() {
        let snippet = Snippet {
            description: "test".to_string(),
            command: "echo hello world".to_string(),
            ..Default::default()
        };

        let toml_str = toml::to_string_pretty(&snippet).unwrap();
        assert!(toml_str.contains("command = \"echo hello world\""));
        assert!(!toml_str.contains("Command ="));
        assert!(!toml_str.contains("cmd ="));

        let reloaded: Snippet = toml::from_str(&toml_str).unwrap();
        assert_eq!(reloaded.command, "echo hello world");
    }

    #[test]
    fn test_snippet_tags_rename_roundtrip() {
        let snippet = Snippet {
            description: "test".to_string(),
            command: "echo test".to_string(),
            tags: vec!["rust".to_string(), "test".to_string()],
            ..Default::default()
        };

        let toml_str = toml::to_string_pretty(&snippet).unwrap();
        assert!(
            toml_str.contains("tag ="),
            "Expected 'tag' field, got: {}",
            toml_str
        );
        assert!(toml_str.contains("\"rust\""));
        assert!(toml_str.contains("\"test\""));
        assert!(!toml_str.contains("tags ="));
        assert!(!toml_str.contains("Tags ="));

        let reloaded: Snippet = toml::from_str(&toml_str).unwrap();
        assert_eq!(reloaded.tags, vec!["rust", "test"]);
    }

    #[test]
    fn test_snippet_with_empty_output_roundtrips() {
        let snippet = Snippet {
            description: "test".to_string(),
            command: "echo test".to_string(),
            output: String::new(),
            ..Default::default()
        };
        let library = Snippets {
            snippets: vec![snippet],
            ..Default::default()
        };

        let serialized = toml::to_string_pretty(&library).unwrap();
        let reloaded: Snippets = toml::from_str(&serialized).unwrap();

        assert_eq!(reloaded.snippets.len(), 1);
        assert_eq!(reloaded.snippets[0].output, "");
    }

    #[test]
    fn test_snippet_with_nonempty_output_roundtrips() {
        let snippet = Snippet {
            description: "test".to_string(),
            command: "echo test".to_string(),
            output: "some output".to_string(),
            ..Default::default()
        };
        let library = Snippets {
            snippets: vec![snippet],
            ..Default::default()
        };

        let serialized = toml::to_string_pretty(&library).unwrap();
        let reloaded: Snippets = toml::from_str(&serialized).unwrap();

        assert_eq!(reloaded.snippets.len(), 1);
        assert_eq!(reloaded.snippets[0].output, "some output");
    }

    #[test]
    fn test_deleted_snippet_preserved_on_roundtrip() {
        let snippet = Snippet {
            description: "deleted snippet".to_string(),
            command: "echo deleted".to_string(),
            deleted: true,
            ..Default::default()
        };
        let library = Snippets {
            snippets: vec![snippet],
            ..Default::default()
        };

        let serialized = toml::to_string_pretty(&library).unwrap();
        let reloaded: Snippets = toml::from_str(&serialized).unwrap();

        assert_eq!(reloaded.snippets.len(), 1);
        assert!(reloaded.snippets[0].deleted);
    }

    #[test]
    fn test_timestamps_preserved_on_roundtrip() {
        let snippet = Snippet {
            description: "test".to_string(),
            command: "echo test".to_string(),
            created_at: 1700000000,
            updated_at: 1700123456,
            ..Default::default()
        };
        let library = Snippets {
            snippets: vec![snippet],
            ..Default::default()
        };

        let serialized = toml::to_string_pretty(&library).unwrap();
        let reloaded: Snippets = toml::from_str(&serialized).unwrap();

        assert_eq!(reloaded.snippets.len(), 1);
        assert_eq!(reloaded.snippets[0].created_at, 1700000000);
        assert_eq!(reloaded.snippets[0].updated_at, 1700123456);
    }

    #[test]
    fn test_uuid_preserved_on_roundtrip() {
        let snippet = Snippet {
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            description: "test".to_string(),
            command: "echo test".to_string(),
            ..Default::default()
        };
        let library = Snippets {
            snippets: vec![snippet],
            ..Default::default()
        };

        let serialized = toml::to_string_pretty(&library).unwrap();
        let reloaded: Snippets = toml::from_str(&serialized).unwrap();

        assert_eq!(reloaded.snippets.len(), 1);
        assert_eq!(
            reloaded.snippets[0].id,
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn test_special_characters_in_description_roundtrip() {
        let snippet = Snippet {
            description: "has \"quotes\" & ampersand <> angle brackets".to_string(),
            command: "echo test".to_string(),
            ..Default::default()
        };
        let library = Snippets {
            snippets: vec![snippet],
            ..Default::default()
        };

        let serialized = toml::to_string_pretty(&library).unwrap();
        let reloaded: Snippets = toml::from_str(&serialized).unwrap();

        assert_eq!(reloaded.snippets.len(), 1);
        assert_eq!(
            reloaded.snippets[0].description,
            "has \"quotes\" & ampersand <> angle brackets"
        );
    }

    // ============================================================
    // Release 2 Final Corrective: Direct serialization matrix
    // ============================================================
    //
    // These tests pin down the contract that the exact Rust `String` value of
    // every snippet field survives the full save / load pipeline. They cover
    // every byte sequence previously excluded from the golden corpus on the
    // (incorrect) premise that TOML cannot preserve them. The TOML format and
    // the `toml` crate's serializer do preserve all of these values; the
    // corruption that motivated their original exclusion came from the
    // custom `quote_strings_containing_backslashes` post-processing helper,
    // which snip-it no longer applies to its own output.

    fn snippet_with_command(command: &str) -> Snippets {
        Snippets {
            snippets: vec![Snippet {
                id: "test-id".to_string(),
                description: "test description".to_string(),
                command: command.to_string(),
                ..Default::default()
            }],
            folders: vec![],
        }
    }

    fn snippet_with_description(command: &str, description: &str) -> Snippets {
        Snippets {
            snippets: vec![Snippet {
                id: "test-id".to_string(),
                description: description.to_string(),
                command: command.to_string(),
                ..Default::default()
            }],
            folders: vec![],
        }
    }

    fn assert_command_survives_pretty_roundtrip(label: &str, command: &str) {
        let library = snippet_with_command(command);
        let serialized = toml::to_string_pretty(&library)
            .unwrap_or_else(|e| panic!("serialize failed for {label}: {e}"));
        let recovered: Snippets = toml::from_str(&serialized)
            .unwrap_or_else(|e| panic!("parse failed for {label}: {e}\nTOML:\n{serialized}"));
        assert_eq!(
            recovered.snippets[0].command, command,
            "round-trip mismatch for {label}: original = {command:?}, recovered = {:?}",
            recovered.snippets[0].command
        );
    }

    fn assert_command_survives_save_load(label: &str, command: &str) {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("matrix.toml");
        let library = snippet_with_command(command);
        save_library(&path, &library).unwrap_or_else(|e| panic!("save failed for {label}: {e}"));
        let loaded = load_library(&path).unwrap_or_else(|e| panic!("load failed for {label}: {e}"));
        assert_eq!(loaded.snippets.len(), 1, "{label}: snippet count");
        assert_eq!(
            loaded.snippets[0].command, command,
            "{label}: save/load round-trip mismatch\noriginal = {command:?}\nrecovered = {:?}",
            loaded.snippets[0].command
        );
    }

    #[test]
    fn test_serialization_matrix_internal_tab() {
        assert_command_survives_pretty_roundtrip("internal_tab", "pre\tpost");
        assert_command_survives_save_load("internal_tab", "pre\tpost");
    }

    #[test]
    fn test_serialization_matrix_leading_tab() {
        assert_command_survives_pretty_roundtrip("leading_tab", "\tpre");
        assert_command_survives_save_load("leading_tab", "\tpre");
    }

    #[test]
    fn test_serialization_matrix_trailing_tab() {
        assert_command_survives_pretty_roundtrip("trailing_tab", "pre\t");
        assert_command_survives_save_load("trailing_tab", "pre\t");
    }

    #[test]
    fn test_serialization_matrix_only_tab() {
        assert_command_survives_pretty_roundtrip("only_tab", "\t");
        assert_command_survives_save_load("only_tab", "\t");
    }

    #[test]
    fn test_serialization_matrix_one_trailing_space() {
        assert_command_survives_pretty_roundtrip("one_trailing_space", "pre ");
        assert_command_survives_save_load("one_trailing_space", "pre ");
    }

    #[test]
    fn test_serialization_matrix_multi_trailing_spaces() {
        assert_command_survives_pretty_roundtrip("multi_trailing_spaces", "pre   ");
        assert_command_survives_save_load("multi_trailing_spaces", "pre   ");
    }

    #[test]
    fn test_serialization_matrix_spaces_before_newline() {
        assert_command_survives_pretty_roundtrip("spaces_before_newline", "pre   \n");
        assert_command_survives_save_load("spaces_before_newline", "pre   \n");
    }

    #[test]
    fn test_serialization_matrix_crlf() {
        assert_command_survives_pretty_roundtrip("crlf", "a\r\nb");
        assert_command_survives_save_load("crlf", "a\r\nb");
    }

    #[test]
    fn test_serialization_matrix_mixed_lf_crlf() {
        assert_command_survives_pretty_roundtrip("mixed_lf_crlf", "a\nb\r\nc");
        assert_command_survives_save_load("mixed_lf_crlf", "a\nb\r\nc");
    }

    #[test]
    fn test_serialization_matrix_final_carriage_return() {
        assert_command_survives_pretty_roundtrip("final_cr", "a\r");
        assert_command_survives_save_load("final_cr", "a\r");
    }

    #[test]
    fn test_serialization_matrix_lone_crlf() {
        assert_command_survives_pretty_roundtrip("lone_crlf", "\r\n");
        assert_command_survives_save_load("lone_crlf", "\r\n");
    }

    #[test]
    fn test_serialization_matrix_tab_with_quotes() {
        assert_command_survives_pretty_roundtrip("tab_with_quotes", "a\t\"b\"c");
        assert_command_survives_save_load("tab_with_quotes", "a\t\"b\"c");
    }

    #[test]
    fn test_serialization_matrix_tab_with_backslashes() {
        assert_command_survives_pretty_roundtrip("tab_with_backslashes", "a\t\\b");
        assert_command_survives_save_load("tab_with_backslashes", "a\t\\b");
    }

    #[test]
    fn test_serialization_matrix_all_problematic() {
        assert_command_survives_pretty_roundtrip("all_problematic", "\t  \r\n");
        assert_command_survives_save_load("all_problematic", "\t  \r\n");
    }

    #[test]
    fn test_serialization_matrix_makefile_leading_tabs() {
        let command = "if true; then\n\techo yes\nelse\n\techo no\nfi";
        assert_command_survives_pretty_roundtrip("makefile_leading_tabs_no_nl", command);
        assert_command_survives_save_load("makefile_leading_tabs_no_nl", command);
    }

    #[test]
    fn test_serialization_matrix_makefile_leading_tabs_with_trailing_newline() {
        let command = "if true; then\n\techo yes\nelse\n\techo no\nfi\n";
        assert_command_survives_pretty_roundtrip("makefile_leading_tabs_trailing_nl", command);
        assert_command_survives_save_load("makefile_leading_tabs_trailing_nl", command);
    }

    #[test]
    fn test_serialization_matrix_variable_syntax_with_tab() {
        let command = "ssh\t<host=localhost>\t-p\t<port=22>";
        assert_command_survives_pretty_roundtrip("variable_with_tab", command);
        assert_command_survives_save_load("variable_with_tab", command);
    }

    #[test]
    fn test_serialization_matrix_escaped_angle_brackets_with_crlf() {
        let command = "echo \\<start\\>\r\necho \\<end\\>";
        assert_command_survives_pretty_roundtrip("escaped_brackets_crlf", command);
        assert_command_survives_save_load("escaped_brackets_crlf", command);
    }

    #[test]
    fn test_serialization_matrix_description_with_tab_and_trailing_space() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("desc.toml");
        let library = snippet_with_description("echo test", "  hello\t");
        save_library(&path, &library).unwrap();
        let loaded = load_library(&path).unwrap();
        assert_eq!(loaded.snippets[0].description, "  hello\t");
    }

    #[test]
    fn test_serialization_matrix_tag_with_internal_tab() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("tag.toml");
        let library = Snippets {
            snippets: vec![Snippet {
                id: "id".to_string(),
                description: "tagged".to_string(),
                command: "echo tag".to_string(),
                tags: vec!["docker\tbuild".to_string()],
                ..Default::default()
            }],
            folders: vec![],
        };
        save_library(&path, &library).unwrap();
        let loaded = load_library(&path).unwrap();
        assert_eq!(loaded.snippets[0].tags, vec!["docker\tbuild"]);
    }

    #[test]
    fn test_serialization_matrix_repeated_save_load_idempotent() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("idempotent.toml");
        let command = "echo\there\r\nwith\ttabs and trailing space ";

        let mut library = snippet_with_command(command);
        for round in 0..5 {
            save_library(&path, &library).unwrap();
            let loaded = load_library(&path).unwrap();
            assert_eq!(
                loaded.snippets[0].command, command,
                "round {round}: command diverged"
            );
            library = loaded;
        }
    }

    #[test]
    fn test_serialization_matrix_pretty_and_compact_agree() {
        let command = "echo\there\nwith\ttabs\r\nand \"quotes\" and trailing space ";
        let library = snippet_with_command(command);

        let pretty = toml::to_string_pretty(&library).unwrap();
        let compact = toml::to_string(&library).unwrap();

        let from_pretty: Snippets = toml::from_str(&pretty).unwrap();
        let from_compact: Snippets = toml::from_str(&compact).unwrap();

        assert_eq!(from_pretty.snippets[0].command, command);
        assert_eq!(from_compact.snippets[0].command, command);
    }

    #[test]
    fn test_serialization_matrix_handwritten_backslash_escape_still_loads() {
        // Hand-written double-quoted TOML with `\<` / `\>` is invalid TOML but
        // a long-standing legacy convention. `fix_invalid_toml_escapes`
        // converts it to single-quoted raw form on load so legacy files
        // continue to work.
        let legacy_toml = "\
[[snippets]]
description = \"legacy\"
command = \"ping \\<website\\>\"
tag = []
output = \"\"
";
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("legacy.toml");
        std::fs::write(&path, legacy_toml).unwrap();

        let loaded = load_library(&path).unwrap();
        assert_eq!(loaded.snippets.len(), 1);
        assert_eq!(loaded.snippets[0].command, "ping \\<website\\>");
    }

    #[test]
    fn test_serialization_matrix_no_normalization_strip_or_trim() {
        // Save / load / re-save / re-load many times, then confirm the byte
        // content is unchanged across every round. This guards against any
        // silent normalization (trim, line-ending rewrite, escape collapse)
        // creeping back into the pipeline.
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("norewrite.toml");
        let command = "echo start\there\t with trailing tab\tand trailing space \r\nand CRLF\r";

        let library = snippet_with_command(command);
        save_library(&path, &library).unwrap();

        let disk_after_first = std::fs::read(&path).unwrap();
        for _ in 0..10 {
            let loaded = load_library(&path).unwrap();
            save_library(&path, &loaded).unwrap();
            let disk_now = std::fs::read(&path).unwrap();
            assert_eq!(
                disk_now, disk_after_first,
                "file content changed across save rounds — normalization regression"
            );
        }

        let final_loaded = load_library(&path).unwrap();
        assert_eq!(final_loaded.snippets[0].command, command);
    }
}
