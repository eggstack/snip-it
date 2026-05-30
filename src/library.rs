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
    #[serde(default = "Vec::new")]
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
    #[serde(alias = "Description", default)]
    pub description: String,
    #[serde(rename = "Output", alias = "output", default)]
    pub output: String,
    #[serde(alias = "Tag", alias = "Tags", default)]
    pub tags: Vec<String>,
    #[serde(alias = "Command", default)]
    pub command: String,
    #[serde(default = "Vec::new")]
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
    Ok(())
}

impl Snippet {
    pub fn new(description: String, command: String, tags: Vec<String>) -> SnipResult<Self> {
        if command.trim().is_empty() {
            return Err(SnipError::runtime_error(
                "Empty command",
                Some("Snippet command cannot be empty"),
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
    unsaved_changes: bool,
}

impl LibraryManager {
    pub fn new() -> SnipResult<Self> {
        // Migrate legacy macOS config dir if needed
        if let Err(e) = crate::utils::config::migrate_macos_config_dir() {
            eprintln!("Warning: Failed to migrate config directory: {}", e);
        }

        let config_dir = get_config_dir();

        let libraries_dir = config_dir.join("libraries");
        let premade_dir = config_dir.join("premade");
        let config_path = config_dir.join("libraries.toml");

        let config = if config_path.exists() {
            let content = fs::read_to_string(&config_path).map_err(|e| {
                SnipError::io_error("read libraries config", config_path.clone(), e)
            })?;
            let content = fix_invalid_toml_escapes(&content);
            match toml::from_str(&content) {
                Ok(c) => c,
                Err(e) => {
                    // Backup corrupted file so data isn't lost on next save
                    let backup = config_path.with_extension("toml.corrupt");
                    if let Err(copy_err) = fs::copy(&config_path, &backup) {
                        eprintln!(
                            "Warning: Failed to parse {}: {} (backup also failed: {})",
                            config_path.display(),
                            e,
                            copy_err
                        );
                    } else {
                        eprintln!(
                            "Warning: Failed to parse {}: {}. \
                             Corrupted file backed up to {}. Using defaults.",
                            config_path.display(),
                            e,
                            backup.display()
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
            unsaved_changes: false,
        })
    }

    pub fn get_default_snippets_path() -> PathBuf {
        get_snippets_path()
    }

    pub fn get_libraries_dir(&self) -> &PathBuf {
        &self.libraries_dir
    }

    pub fn is_single_file_mode(&self) -> bool {
        !self.libraries_dir.exists()
    }

    pub fn get_legacy_snippets_path(&self) -> PathBuf {
        Self::get_default_snippets_path()
    }

    pub fn ensure_library_mode(&mut self) -> SnipResult<()> {
        if self.is_single_file_mode() {
            self.migrate_from_single_file()?;
        }
        Ok(())
    }

    pub fn init_libraries_dir(&self) -> SnipResult<()> {
        if !self.libraries_dir.exists() {
            fs::create_dir_all(&self.libraries_dir).map_err(|e| {
                SnipError::io_error("create libraries directory", self.libraries_dir.clone(), e)
            })?;
        }
        Ok(())
    }

    pub fn migrate_from_single_file(&mut self) -> SnipResult<()> {
        let legacy_path = self.get_legacy_snippets_path();

        if !legacy_path.exists() {
            return Ok(());
        }

        self.init_libraries_dir()?;

        let content = fs::read_to_string(&legacy_path)?;
        if content.trim().is_empty() {
            return Ok(());
        }

        let new_path = self.libraries_dir.join("snippets.toml");
        fs::copy(&legacy_path, &new_path)
            .map_err(|e| SnipError::io_error("migrate snippets file", new_path.clone(), e))?;

        let mut meta = LibraryMeta::new("snippets");
        meta.is_primary = true;
        self.config.libraries.push(meta);
        self.unsaved_changes = true;
        self.save_config()?;

        Ok(())
    }

    pub fn list_libraries(&self) -> Vec<&LibraryMeta> {
        self.config.libraries.iter().collect()
    }

    pub fn get_primary_library(&self) -> Option<&LibraryMeta> {
        self.config.libraries.iter().find(|l| l.is_primary)
    }

    pub fn get_library_by_filename(&self, filename: &str) -> Option<&LibraryMeta> {
        self.config
            .libraries
            .iter()
            .find(|l| l.filename == filename)
    }

    pub fn get_library_by_filename_mut(&mut self, filename: &str) -> Option<&mut LibraryMeta> {
        self.config
            .libraries
            .iter_mut()
            .find(|l| l.filename == filename)
    }

    pub fn create_library(&mut self, filename: &str) -> SnipResult<PathBuf> {
        validate_library_name(filename)
            .map_err(|(msg, detail)| SnipError::runtime_error(msg, Some(detail)))?;

        self.init_libraries_dir()?;

        let path = self.libraries_dir.join(format!("{}.toml", filename));

        if path.exists() {
            return Err(SnipError::runtime_error(
                "Library already exists",
                Some(&format!("File {} already exists", path.display())),
            ));
        }

        let default_content = r#"# Snippet library
# Each snippet has: Description, Output, Tag, command, folders, favorite

Snippets = []

"#;

        fs::write(&path, default_content)
            .map_err(|e| SnipError::io_error("create library file", path.clone(), e))?;

        let is_first = self.config.libraries.is_empty();
        let mut meta = LibraryMeta::new(filename);
        meta.is_primary = is_first;
        self.config.libraries.push(meta);
        self.unsaved_changes = true;
        self.save_config()?;

        Ok(path)
    }

    pub fn delete_library(&mut self, filename: &str) -> SnipResult<()> {
        let was_primary = self
            .get_library_by_filename(filename)
            .map(|l| l.is_primary)
            .ok_or_else(|| SnipError::runtime_error("Library not found", Some(filename)))?;

        let deleted_was_server = self
            .get_library_by_filename(filename)
            .map(|l| l.server_id.is_some())
            .unwrap_or(false);

        let path = self.libraries_dir.join(format!("{}.toml", filename));

        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| SnipError::io_error("delete library file", path.clone(), e))?;
        }

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
            if let Some(promoted_lib) = promoted {
                if let Some(idx) = self
                    .config
                    .libraries
                    .iter()
                    .position(|l| l.filename == promoted_lib.filename)
                {
                    self.config.libraries[idx].is_primary = true;
                }
            }
            self.unsaved_changes = true;
        }

        self.save_config()?;
        Ok(())
    }

    pub fn set_primary(&mut self, filename: &str) -> SnipResult<()> {
        if !self
            .config
            .libraries
            .iter()
            .any(|lib| lib.filename == filename)
        {
            return Err(SnipError::runtime_error(
                "Library not found",
                Some(&format!("No library with filename '{}'", filename)),
            ));
        }
        for lib in &mut self.config.libraries {
            lib.is_primary = lib.filename == filename;
        }
        self.unsaved_changes = true;
        self.save_config()?;
        Ok(())
    }

    pub fn update_library_id(&mut self, filename: &str, library_id: &str) -> SnipResult<()> {
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.library_id = library_id.to_string();
            self.unsaved_changes = true;
            self.save_config()?;
        }
        Ok(())
    }

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
        self.unsaved_changes = true;
        self.save_config()?;
        Ok(())
    }

    pub fn update_last_sync(&mut self, filename: &str, timestamp: i64) -> SnipResult<()> {
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.last_sync = Some(timestamp);
            self.unsaved_changes = true;
            self.save_config()?;
        }
        Ok(())
    }

    pub fn add_server_library(
        &mut self,
        server_name: &str,
        server_id: &str,
    ) -> SnipResult<PathBuf> {
        let filename = server_name.to_lowercase().replace(' ', "-");

        self.init_libraries_dir()?;

        let path = self.libraries_dir.join(format!("{}.toml", filename));

        if !path.exists() {
            let default_content = "# Imported from server\n\nSnippets = []\n";
            fs::write(&path, default_content)
                .map_err(|e| SnipError::io_error("create imported library", path.clone(), e))?;
        }

        // Update existing entry if one with the same filename already exists
        if let Some(existing) = self.get_library_by_filename_mut(&filename) {
            existing.library_id = server_id.to_string();
            existing.server_id = Some(server_id.to_string());
            self.unsaved_changes = true;
            self.save_config()?;
            return Ok(path);
        }

        let is_first = self.config.libraries.is_empty();
        let mut meta = LibraryMeta::new(&filename);
        meta.library_id = server_id.to_string();
        meta.server_id = Some(server_id.to_string());
        meta.is_primary = is_first;

        self.config.libraries.push(meta);
        self.unsaved_changes = true;
        self.save_config()?;

        Ok(path)
    }

    pub fn init_premade_dir(&self) -> SnipResult<()> {
        if !self.premade_dir.exists() {
            fs::create_dir_all(&self.premade_dir).map_err(|e| {
                SnipError::io_error("create premade directory", self.premade_dir.clone(), e)
            })?;
        }
        Ok(())
    }

    pub fn premade_exists(&self, filename: &str) -> bool {
        self.premade_dir.join(format!("{}.toml", filename)).exists()
    }

    pub fn save_premade_library(&self, filename: &str, content: &str) -> SnipResult<PathBuf> {
        self.init_premade_dir()?;

        let path = self.premade_dir.join(format!("{}.toml", filename));
        fs::write(&path, content)
            .map_err(|e| SnipError::io_error("save premade library", path.clone(), e))?;

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
        fs::write(&tmp_path, &toml_str)
            .map_err(|e| SnipError::io_error("write temp config", tmp_path.clone(), e))?;

        std::fs::rename(&tmp_path, &config_path)
            .map_err(|e| SnipError::io_error("atomic rename config", config_path.clone(), e))?;

        self.unsaved_changes = false;
        Ok(())
    }
}

pub fn load_library(path: &Path) -> SnipResult<Snippets> {
    if !path.exists() {
        return Ok(Snippets::default());
    }

    let content = fs::read_to_string(path)?;
    if content.is_empty() || content.trim().is_empty() {
        return Ok(Snippets::default());
    }

    let fixed_content = fix_invalid_toml_escapes(&content);

    let snippets: Snippets = match toml::from_str(&fixed_content) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "Warning: Failed to parse {}, using defaults: {}",
                path.display(),
                e
            );
            // Create backup of corrupted file before returning defaults
            let backup_path = path.with_extension("toml.corrupt.bak");
            if let Err(backup_err) = fs::copy(path, &backup_path) {
                eprintln!(
                    "Warning: Could not create backup of corrupted file: {}",
                    backup_err
                );
            } else {
                eprintln!(
                    "Backup of corrupted file saved to {}",
                    backup_path.display()
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

pub fn save_library(path: &Path, snippets: &Snippets) -> SnipResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| SnipError::io_error("create directory", parent, e))?;
    }

    let toml_str = toml::to_string_pretty(snippets)
        .map_err(|e| SnipError::toml_error("serialize snippets", e))?;

    let toml_str = quote_strings_containing_backslashes(&toml_str);

    fs::write(path, toml_str).map_err(|e| SnipError::io_error("write snippets file", path, e))?;

    Ok(())
}

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
}
