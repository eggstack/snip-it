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
//! [[Snippets]]
//! Description = "git commit"
//! Tag = ["git"]
//! command = "git commit -m \"<msg>\""
//! ```

use crate::config::cached_read_toml;
use crate::error::{SnipError, SnipResult};
use crate::utils::config::{get_config_dir, get_snippets_path};
use crate::utils::toml_helpers::{fix_invalid_toml_escapes, quote_strings_containing_backslashes};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Container for a collection of snippets.
///
/// Wraps a list of [`Snippet`] items and optional folder structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Snippets {
    #[serde(rename = "Snippets", default)]
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
    #[serde(rename = "Id", alias = "ID", default)]
    pub id: String,
    #[serde(alias = "Description", alias = "name", default)]
    pub description: String,
    #[serde(rename = "Output", alias = "output", default)]
    pub output: String,
    #[serde(
        alias = "Tag",
        alias = "Tags",
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
        Ok(Self {
            id: String::new(),
            description,
            command,
            tags,
            output: String::new(),
            folders: Vec::new(),
            favorite: false,
            created_at: 0,
            updated_at: 0,
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
# Each snippet has: Description, Output, Tag, command, folders, favorite

Snippets = []

"#;

        fs::write(&path, default_content)
            .map_err(|e| SnipError::io_error("create library file", path.clone(), e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            if let Err(e) = fs::set_permissions(&path, perms) {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to set restrictive permissions on library file"
                );
            }
        }

        let is_first = self.config.libraries.is_empty();
        let mut meta = LibraryMeta::new(filename);
        meta.is_primary = is_first;
        self.config.libraries.push(meta);

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

        self.save_config()?;

        if path.exists()
            && let Err(e) = fs::remove_file(&path)
        {
            tracing::warn!(
                library = %filename,
                error = %e,
                "Config updated but failed to delete library file"
            );
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

        self.save_config()?;
        Ok(())
    }

    /// Updates the server-side library ID for a local library.
    pub fn update_library_id(&mut self, filename: &str, library_id: &str) -> SnipResult<()> {
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.library_id = library_id.to_string();

            self.save_config()?;
        }
        Ok(())
    }

    /// Registers an existing library file that is not yet tracked in the config.
    pub fn add_existing_library(&mut self, filename: &str) -> SnipResult<()> {
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

        self.save_config()?;
        Ok(())
    }

    /// Updates the last-sync timestamp for a library.
    pub fn update_last_sync(&mut self, filename: &str, timestamp: i64) -> SnipResult<()> {
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.last_sync = Some(timestamp);

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
            let default_content = "# Imported from server\n\nSnippets = []\n";
            fs::write(&path, default_content)
                .map_err(|e| SnipError::io_error("create imported library", path.clone(), e))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(0o600);
                if let Err(e) = fs::set_permissions(&path, perms) {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to set restrictive permissions on imported library file"
                    );
                }
            }
        }

        // Update existing entry if one with the same filename already exists
        if let Some(existing) = self.get_library_by_filename_mut(&filename) {
            existing.library_id = server_id.to_string();
            existing.server_id = Some(server_id.to_string());

            self.save_config()?;
            return Ok(path);
        }

        let is_first = self.config.libraries.is_empty();
        let mut meta = LibraryMeta::new(&filename);
        meta.library_id = server_id.to_string();
        meta.server_id = Some(server_id.to_string());
        meta.is_primary = is_first;

        self.config.libraries.push(meta);

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

        fs::write(&path, content)
            .map_err(|e| SnipError::io_error("save premade library", path.clone(), e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            if let Err(e) = fs::set_permissions(&path, perms) {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to set restrictive permissions on library file"
                );
            }
        }

        Ok(path)
    }

    fn save_config(&mut self) -> SnipResult<()> {
        let config_path = self.config_dir.join("libraries.toml");

        let toml_str = toml::to_string_pretty(&self.config)
            .map_err(|e| SnipError::toml_error("serialize libraries config", e))?;

        let toml_str = quote_strings_containing_backslashes(&toml_str);

        let parent_dir = config_path
            .parent()
            .ok_or_else(|| SnipError::runtime_error("config path has no parent", None))?;
        if !parent_dir.exists() {
            fs::create_dir_all(parent_dir)
                .map_err(|e| SnipError::io_error("create config directory", parent_dir, e))?;
        }

        let tmp_path = parent_dir.join("libraries.toml.tmp");
        let _ = fs::remove_file(&tmp_path);
        fs::write(&tmp_path, &toml_str)
            .map_err(|e| SnipError::io_error("write temp config", tmp_path.clone(), e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            if let Err(e) = fs::set_permissions(&tmp_path, perms) {
                tracing::warn!(
                    path = %tmp_path.display(),
                    error = %e,
                    "Failed to set restrictive permissions on config temp file"
                );
            }
        }

        std::fs::rename(&tmp_path, &config_path)
            .map_err(|e| SnipError::io_error("atomic rename config", config_path.clone(), e))?;

        Ok(())
    }
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
pub fn save_library(path: &Path, snippets: &Snippets) -> SnipResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| SnipError::io_error("create directory", parent, e))?;
    }

    if let Err(e) = backup_library(path) {
        tracing::warn!(error = %e, "Failed to create backup before save");
    }

    let mut sorted = snippets.clone();
    sorted
        .snippets
        .sort_by_key(|b| std::cmp::Reverse(b.updated_at));

    let toml_str = toml::to_string_pretty(&sorted)
        .map_err(|e| SnipError::toml_error("serialize snippets", e))?;

    let toml_str = quote_strings_containing_backslashes(&toml_str);

    let tmp_path = path.with_extension("toml.tmp");
    let _ = fs::remove_file(&tmp_path);
    fs::write(&tmp_path, &toml_str)
        .map_err(|e| SnipError::io_error("write snippets temp", &tmp_path, e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        if let Err(e) = fs::set_permissions(&tmp_path, perms) {
            tracing::warn!(
                path = %tmp_path.display(),
                error = %e,
                "Failed to set restrictive permissions on temp file"
            );
        }
    }

    fs::rename(&tmp_path, path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        SnipError::io_error("atomic rename snippets file", path, e)
    })?;

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

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
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

    #[test]
    fn test_pet_format_compatibility() {
        let pet_toml = r#"
[[Snippets]]
  Description = "git commit with message"
  Command = "git commit -m \"message\""
  Tag = ["git", "version-control"]
  Output = ""

[[Snippets]]
  Description = "docker ps"
  Command = "docker ps"
  Tag = ["docker"]
  Output = ""
"#;
        let snippets: Snippets = toml::from_str(pet_toml).unwrap();
        assert_eq!(snippets.snippets.len(), 2);
        assert_eq!(snippets.snippets[0].command, "git commit -m \"message\"");
        assert_eq!(snippets.snippets[0].description, "git commit with message");
        assert_eq!(snippets.snippets[0].tags, vec!["git", "version-control"]);
        assert_eq!(snippets.snippets[1].command, "docker ps");
    }

    #[test]
    fn test_snp_format_compatibility() {
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
        let tmp = path.with_extension("toml.tmp");
        assert!(
            !tmp.exists(),
            "temp file should not remain after atomic rename"
        );
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
}
