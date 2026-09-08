//! **Layer: Domain/Core**
//!
//! [`LibraryManager`]: registry/primary-library behavior and the canonical
//! read-only resolver.

use super::model::validate_library_name;
use super::model::{
    LibraryConfig, LibraryIndexInspection, LibraryMeta, PrimaryState, ResolvedLibrarySource,
    library_not_found,
};
use super::persistence::write_library_file;
use crate::config::{cached_read_toml, invalidate_toml_cache};
use crate::error::{SnipError, SnipResult};
use crate::test_failpoints::mutation_barrier;
use crate::utils::config::{get_config_dir, get_snippets_path};
use crate::utils::toml_helpers::fix_invalid_toml_escapes;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// Manages snippet libraries and premade collections.
///
/// LibraryManager handles:
/// - Loading and saving the libraries configuration
/// - Creating, deleting, and managing individual libraries
/// - Loading premade libraries
/// - Determining whether to use single-file or library mode
#[derive(Debug)]
pub struct LibraryManager {
    pub(crate) config_dir: PathBuf,
    pub(crate) libraries_dir: PathBuf,
    pub(crate) premade_dir: PathBuf,
    pub(crate) config: LibraryConfig,
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
                    // Best-effort backup of corrupted file before returning the error.
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
                            "Failed to parse config, backed up to file"
                        );
                    }
                    return Err(SnipError::toml_error(
                        &format!("parse libraries config: {}", config_path.display()),
                        e,
                    ));
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
                    if let Err(copy_err) = fs::copy(&config_path, &backup) {
                        tracing::warn!(
                            error = %e,
                            backup_error = %copy_err,
                            "Failed to parse config (backup also failed)"
                        );
                    } else {
                        tracing::warn!(
                            error = %e,
                            backup = %backup.display(),
                            "Failed to parse config, backed up to file"
                        );
                    }
                    return Err(SnipError::toml_error(
                        &format!("parse libraries config: {}", config_path.display()),
                        e,
                    ));
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
    pub fn get_legacy_snippets_path() -> PathBuf {
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
        let _lock = self.acquire_local_data_lock()?;
        let legacy_path = Self::get_legacy_snippets_path();

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

    /// Resolve read-only library sources without mutating any state.
    ///
    /// This is the single canonical implementation for side-effect-free
    /// library discovery. It never calls [`Self::ensure_library_mode`],
    /// never creates directories or files, and never rewrites metadata.
    /// Legacy single-file mode is represented as the implicit `snippets`
    /// library for compatibility with normal CLI resolution.
    ///
    /// `library` selects scope: `None` for the default (primary, or legacy
    /// file), `Some("all")` for every visible library, or `Some(name)` for
    /// one named library.
    pub fn resolve_readonly_sources(
        &self,
        library: Option<&str>,
    ) -> SnipResult<Vec<ResolvedLibrarySource>> {
        if self.is_single_file_mode() {
            let path = Self::get_default_snippets_path();
            let available = path.exists();
            return match library {
                None | Some("all") if available => Ok(vec![ResolvedLibrarySource {
                    name: "snippets".to_string(),
                    library_id: String::new(),
                    path,
                }]),
                Some("snippets") if available => Ok(vec![ResolvedLibrarySource {
                    name: "snippets".to_string(),
                    library_id: String::new(),
                    path,
                }]),
                Some("all") | None => Ok(Vec::new()),
                Some(name) => Err(library_not_found(name)),
            };
        }

        let make_source = |meta: &LibraryMeta| ResolvedLibrarySource {
            name: meta.filename.clone(),
            library_id: meta.library_id.clone(),
            path: self.libraries_dir.join(format!("{}.toml", meta.filename)),
        };

        match library {
            Some("all") => Ok(self.config.libraries.iter().map(make_source).collect()),
            Some(name) => self
                .get_library_by_filename(name)
                .map(|meta| vec![make_source(meta)])
                .ok_or_else(|| library_not_found(name)),
            None => Ok(self
                .get_primary_library()
                .map(make_source)
                .into_iter()
                .collect()),
        }
    }

    /// Inspect library index consistency without mutating any state.
    ///
    /// Reports index entries whose files are missing, library files with no
    /// index entry, and the current primary-library state. Callers render
    /// the result into their own diagnostic types; this function performs
    /// no I/O beyond existence checks and directory listing.
    pub fn inspect_library_index(&self) -> LibraryIndexInspection {
        let mut missing_files = Vec::new();
        for meta in &self.config.libraries {
            let path = self.libraries_dir.join(format!("{}.toml", meta.filename));
            if !path.exists() {
                missing_files.push((meta.filename.clone(), path));
            }
        }

        let mut orphan_files = Vec::new();
        if self.libraries_dir.exists()
            && let Ok(entries) = std::fs::read_dir(&self.libraries_dir)
        {
            let indexed: HashSet<&str> = self
                .config
                .libraries
                .iter()
                .map(|l| l.filename.as_str())
                .collect();
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "toml")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                    && !indexed.contains(stem)
                {
                    orphan_files.push(path);
                }
            }
        }

        let primary = match self.get_primary_library() {
            Some(meta) => {
                let path = self.libraries_dir.join(format!("{}.toml", meta.filename));
                if path.exists() {
                    PrimaryState::Present {
                        name: meta.filename.clone(),
                    }
                } else {
                    PrimaryState::FileMissing {
                        name: meta.filename.clone(),
                        path,
                    }
                }
            }
            None => {
                if self.config.libraries.is_empty() {
                    PrimaryState::NoLibraries
                } else {
                    PrimaryState::NoPrimary {
                        count: self.config.libraries.len(),
                    }
                }
            }
        };

        LibraryIndexInspection {
            missing_files,
            orphan_files,
            primary,
        }
    }

    /// Creates a new snippet library file and registers it in the config.
    ///
    /// The first library created is automatically marked as primary.
    /// Returns the path to the newly created library file.
    pub fn create_library(&mut self, filename: &str) -> SnipResult<PathBuf> {
        let _lock = self.acquire_local_data_lock()?;
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

        mutation_barrier("library-create-after-file-before-index");

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
        let _lock = self.acquire_local_data_lock()?;
        let config_before_delete = self.config.clone();
        let (was_primary, deleted_was_server) = self
            .get_library_by_filename(filename)
            .map(|l| (l.is_primary, l.server_id.is_some()))
            .ok_or_else(|| SnipError::runtime_error("Library not found", Some(filename)))?;

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

        mutation_barrier("library-delete-after-index-before-file");

        if path.exists()
            && let Err(e) = fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            self.config = config_before_delete;
            if let Err(restore_error) = self.save_config() {
                tracing::error!(
                    error = %restore_error,
                    path = %path.display(),
                    "Failed to restore library config after file deletion failure"
                );
                return Err(SnipError::runtime_error(
                    "Delete library failed",
                    Some(&format!(
                        "Could not delete {} ({e}); restoring libraries.toml also failed: {restore_error}",
                        path.display()
                    )),
                ));
            }
            return Err(SnipError::io_error("delete library file", path.clone(), e));
        }

        Ok(())
    }

    /// Sets the given library as primary, unmarking all others.
    pub fn set_primary(&mut self, filename: &str) -> SnipResult<()> {
        let _lock = self.acquire_local_data_lock()?;
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
        let _lock = self.acquire_local_data_lock()?;
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.library_id = library_id.to_string();

            self.bump_generation();
            self.save_config()?;
        }
        Ok(())
    }

    /// Links a local library to a server-side library.
    pub fn link_server_library(&mut self, filename: &str, server_id: &str) -> SnipResult<()> {
        let _lock = self.acquire_local_data_lock()?;
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.library_id = server_id.to_string();
            lib.server_id = Some(server_id.to_string());

            self.bump_generation();
            self.save_config()?;
        }
        Ok(())
    }

    /// Atomically persists server linkage and the sync cursor for recovery.
    pub fn relink_server_library(
        &mut self,
        filename: &str,
        server_id: &str,
        last_sync: Option<i64>,
    ) -> SnipResult<()> {
        let _lock = self.acquire_local_data_lock()?;
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.library_id = server_id.to_string();
            lib.server_id = Some(server_id.to_string());
            lib.last_sync = last_sync;

            self.bump_generation();
            self.save_config()?;
        }
        Ok(())
    }

    /// Clears server linkage metadata for a local library.
    pub fn unlink_server_library(&mut self, filename: &str) -> SnipResult<()> {
        let _lock = self.acquire_local_data_lock()?;
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
        let _lock = self.acquire_local_data_lock()?;
        validate_library_name(filename)
            .map_err(|(title, detail)| SnipError::runtime_error(title, Some(detail)))?;

        if self.get_library_by_filename(filename).is_some() {
            return Ok(());
        }

        let is_first = self.config.libraries.is_empty();
        let meta = LibraryMeta {
            filename: filename.to_string(),
            library_id: String::new(),
            is_primary: is_first,
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
        let _lock = self.acquire_local_data_lock()?;
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.last_sync = Some(timestamp);

            self.bump_generation();
            self.save_config()?;
        }
        Ok(())
    }

    /// Acquires the local-data lock to serialize against backup snapshot capture.
    ///
    /// This ensures that backup sees either the complete before-state or
    /// complete after-state of any config mutation, never a mixed state.
    fn acquire_local_data_lock(&self) -> SnipResult<crate::local_data::LocalDataLock> {
        let transaction_dir = self.config_dir.join(".transaction");
        crate::local_data::acquire_local_data_lock(&transaction_dir)
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
        let _lock = self.acquire_local_data_lock()?;
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
        let _lock = self.acquire_local_data_lock()?;
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

/// Resolve read-only library sources from the current process config.
///
/// Convenience wrapper around [`LibraryManager::new`] plus
/// [`LibraryManager::resolve_readonly_sources`]. Performs no migration and
/// creates no files or directories.
pub fn readonly_library_sources(library: Option<&str>) -> SnipResult<Vec<ResolvedLibrarySource>> {
    let manager = LibraryManager::new()?;
    manager.resolve_readonly_sources(library)
}
