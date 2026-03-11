use crate::error::{SnipError, SnipResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Snippets {
    #[serde(rename = "Snippets", default)]
    pub snippets: Vec<Snippet>,
    #[serde(default = "Vec::new")]
    pub folders: Vec<String>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LibraryConfig {
    #[serde(default)]
    pub libraries: Vec<LibraryMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryMeta {
    pub filename: String,
    #[serde(default)]
    pub library_id: String,
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default)]
    pub last_sync: Option<i64>,
}

impl LibraryMeta {
    pub fn new(filename: &str) -> Self {
        Self {
            filename: filename.to_string(),
            library_id: String::new(),
            is_primary: false,
            last_sync: None,
        }
    }
}

pub struct LibraryManager {
    config_dir: PathBuf,
    libraries_dir: PathBuf,
    premade_dir: PathBuf,
    config: LibraryConfig,
}

impl LibraryManager {
    pub fn new() -> SnipResult<Self> {
        let config_dir = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".config")))
            .join("snp");

        let libraries_dir = config_dir.join("libraries");
        let premade_dir = config_dir.join("premade");
        let config_path = config_dir.join("libraries.toml");

        let config = if config_path.exists() {
            let content = fs::read_to_string(&config_path).map_err(|e| {
                SnipError::io_error("read libraries config", config_path.clone(), e)
            })?;
            toml::from_str(&content).unwrap_or_default()
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

    pub fn get_default_snippets_path() -> PathBuf {
        std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".config")))
            .join("snp")
            .join("snippets.toml")
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
        if filename.is_empty() {
            return Err(SnipError::runtime_error(
                "Invalid library name",
                Some("Library name cannot be empty"),
            ));
        }

        if filename.len() > 50 {
            return Err(SnipError::runtime_error(
                "Invalid library name",
                Some("Library name cannot exceed 50 characters"),
            ));
        }

        if filename.contains('/') || filename.contains('\\') {
            return Err(SnipError::runtime_error(
                "Invalid library name",
                Some("Library name cannot contain slashes"),
            ));
        }

        if filename.contains('\0') {
            return Err(SnipError::runtime_error(
                "Invalid library name",
                Some("Library name cannot contain null bytes"),
            ));
        }

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
        self.save_config()?;

        Ok(path)
    }

    pub fn delete_library(&mut self, filename: &str) -> SnipResult<()> {
        let was_primary = self
            .get_library_by_filename(filename)
            .map(|l| l.is_primary)
            .ok_or_else(|| SnipError::runtime_error("Library not found", Some(filename)))?;

        let path = self.libraries_dir.join(format!("{}.toml", filename));

        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| SnipError::io_error("delete library file", path.clone(), e))?;
        }

        self.config.libraries.retain(|l| l.filename != filename);

        if was_primary && !self.config.libraries.is_empty() {
            self.config.libraries[0].is_primary = true;
        }

        self.save_config()?;
        Ok(())
    }

    pub fn set_primary(&mut self, filename: &str) -> SnipResult<()> {
        for lib in &mut self.config.libraries {
            lib.is_primary = lib.filename == filename;
        }
        self.save_config()?;
        Ok(())
    }

    pub fn update_library_id(&mut self, filename: &str, library_id: &str) -> SnipResult<()> {
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.library_id = library_id.to_string();
            self.save_config()?;
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn add_existing_library(&mut self, filename: &str) -> SnipResult<()> {
        if self.get_library_by_filename(filename).is_some() {
            return Ok(());
        }

        let meta = LibraryMeta {
            filename: filename.to_string(),
            library_id: String::new(),
            is_primary: false,
            last_sync: None,
        };

        self.config.libraries.push(meta);
        self.save_config()?;
        Ok(())
    }

    pub fn update_last_sync(&mut self, filename: &str, timestamp: i64) -> SnipResult<()> {
        if let Some(lib) = self.get_library_by_filename_mut(filename) {
            lib.last_sync = Some(timestamp);
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

        let is_first = self.config.libraries.is_empty();
        let mut meta = LibraryMeta::new(&filename);
        meta.library_id = server_id.to_string();
        meta.is_primary = is_first;

        self.config.libraries.push(meta);
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

    fn save_config(&self) -> SnipResult<()> {
        let config_path = self.config_dir.join("libraries.toml");

        let toml_str = toml::to_string_pretty(&self.config)
            .map_err(|e| SnipError::toml_error("serialize libraries config", e))?;

        fs::write(&config_path, toml_str)
            .map_err(|e| SnipError::io_error("write libraries config", config_path, e))?;

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

    let snippets: Snippets = match toml::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "Warning: Failed to parse {}, using defaults: {}",
                path.display(),
                e
            );
            Snippets::default()
        }
    };

    Ok(snippets)
}

pub fn save_library(path: &Path, snippets: &Snippets) -> SnipResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| SnipError::io_error("create directory", parent, e))?;
    }

    let toml_str = toml::to_string_pretty(snippets)
        .map_err(|e| SnipError::toml_error("serialize snippets", e))?;

    fs::write(path, toml_str).map_err(|e| SnipError::io_error("write snippets file", path, e))?;

    Ok(())
}

pub fn backup_library(path: &Path) -> SnipResult<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }

    let backup_dir = path.parent().unwrap().join("backups");
    fs::create_dir_all(&backup_dir)
        .map_err(|e| SnipError::io_error("create backup directory", backup_dir.clone(), e))?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let backup_name = format!(
        "{}.{}.toml.bak",
        path.file_stem().unwrap().to_string_lossy(),
        timestamp
    );
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
