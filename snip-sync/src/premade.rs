use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_PREMADE_COMMAND_LENGTH: usize = 1024;
const MAX_PREMADE_DESCRIPTION_LENGTH: usize = 1024;
use tonic::Status;

use crate::toml_escapes::fix_invalid_toml_escapes;

#[derive(Debug, Clone)]
pub struct PremadeLibrary {
    pub name: String,
    pub filename: String,
    pub description: String,
    pub snippet_count: i32,
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Snippet {
    #[serde(rename = "command", alias = "Command", default)]
    command: Option<String>,
    #[serde(rename = "Description", default)]
    description: Option<String>,
    #[serde(rename = "Tag", default)]
    tags: Vec<String>,
    #[serde(flatten, default)]
    _extra: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SnippetFile {
    #[serde(rename = "Description", default)]
    description: Option<String>,
    #[serde(rename = "Snippets", default)]
    snippets: Vec<Snippet>,
}

#[derive(Debug, Clone)]
pub struct PremadeManager {
    dir: PathBuf,
    libraries: Vec<PremadeLibrary>,
}

impl PremadeManager {
    pub fn new(dir: PathBuf) -> Self {
        if !dir.exists() {
            if let Err(e) = fs::create_dir_all(&dir) {
                tracing::warn!(
                    "Failed to create premade libraries directory {}: {}",
                    dir.display(),
                    e
                );
            } else {
                tracing::info!("Created premade libraries directory: {}", dir.display());
            }
        }

        let libraries = Self::scan_directory(&dir);
        Self { dir, libraries }
    }

    fn scan_directory(dir: &Path) -> Vec<PremadeLibrary> {
        if !dir.exists() {
            tracing::warn!(
                "Premade libraries directory does not exist: {}",
                dir.display()
            );
            return Vec::new();
        }

        let mut libraries = Vec::new();

        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::error!("Failed to read premade libraries directory: {}", e);
                return Vec::new();
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }

            let filename = match path.file_stem().and_then(|s| s.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to read premade library {}: {}", filename, e);
                    continue;
                }
            };

            let content = fix_invalid_toml_escapes(&content);

            let snippet_file: SnippetFile = match toml::from_str(&content) {
                Ok(sf) => sf,
                Err(e) => {
                    tracing::warn!("Failed to parse premade library {}: {}", filename, e);
                    continue;
                }
            };

            if snippet_file.snippets.is_empty() {
                tracing::warn!("Premade library '{}' has no snippets, skipping", filename);
                continue;
            }

            let valid_snippets: Vec<_> = snippet_file
                .snippets
                .into_iter()
                .filter(|s| {
                    let cmd = s.command.as_deref().unwrap_or("");
                    if cmd.is_empty() {
                        tracing::warn!(
                            "Premade library '{}': skipping snippet with empty command",
                            filename
                        );
                        return false;
                    }
                    if cmd.len() > MAX_PREMADE_COMMAND_LENGTH {
                        tracing::warn!(
                            "Premade library '{}': skipping snippet with command exceeding {} bytes",
                            filename,
                            MAX_PREMADE_COMMAND_LENGTH
                        );
                        return false;
                    }
                    if let Some(ref desc) = s.description
                        && desc.len() > MAX_PREMADE_DESCRIPTION_LENGTH
                    {
                        tracing::warn!(
                            "Premade library '{}': skipping snippet with description exceeding {} bytes",
                            filename,
                            MAX_PREMADE_DESCRIPTION_LENGTH
                        );
                        return false;
                    }
                    true
                })
                .collect();

            if valid_snippets.is_empty() {
                tracing::warn!(
                    "Premade library '{}' has no valid snippets, skipping",
                    filename
                );
                continue;
            }

            let snippet_count = valid_snippets.len() as i32;

            let description = snippet_file.description.unwrap_or_else(|| filename.clone());

            let mut all_tags: Vec<String> = Vec::new();
            for s in &valid_snippets {
                for tag in &s.tags {
                    if !all_tags.contains(tag) {
                        all_tags.push(tag.clone());
                    }
                }
            }

            libraries.push(PremadeLibrary {
                name: filename.clone(),
                filename,
                description,
                snippet_count,
                tags: all_tags,
            });
        }

        libraries.sort_by(|a, b| a.filename.cmp(&b.filename));
        tracing::info!("Loaded {} premade libraries", libraries.len());
        libraries
    }

    pub fn list(&self) -> Vec<PremadeLibrary> {
        self.libraries.clone()
    }

    /// Search premade libraries by query string.
    /// Matches against library name, description, and tags.
    pub fn search(&self, query: &str) -> Vec<PremadeLibrary> {
        let q = query.to_lowercase();
        self.libraries
            .iter()
            .filter(|lib| {
                lib.name.to_lowercase().contains(&q)
                    || lib.description.to_lowercase().contains(&q)
                    || lib.tags.iter().any(|t| t.to_lowercase().contains(&q))
            })
            .cloned()
            .collect()
    }

    pub fn get(&self, filename: &str) -> Result<String, Status> {
        let path = self.dir.join(format!("{}.toml", filename));

        let canonical_dir = self
            .dir
            .canonicalize()
            .map_err(|_| Status::internal("failed to canonicalize premade directory"))?;
        let canonical_path = path
            .canonicalize()
            .map_err(|_| Status::invalid_argument("Invalid filename: cannot canonicalize path"))?;

        if !canonical_path.starts_with(&canonical_dir) {
            return Err(Status::invalid_argument(
                "Invalid filename: path traversal detected",
            ));
        }

        fs::read_to_string(&canonical_path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Status::not_found(format!("Premade library '{}' not found", filename))
                } else {
                    tracing::error!("Failed to read premade library: {}", e);
                    Status::internal("Internal error")
                }
            })
            .map(|content| fix_invalid_toml_escapes(&content))
    }

    pub fn is_empty(&self) -> bool {
        self.libraries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_get_valid_library() {
        let dir = TempDir::new().unwrap();
        let toml_content = r#"
Description = "Test library"

[[Snippets]]
Command = "echo hello"
Description = "Say hello"
"#;
        fs::write(dir.path().join("test.toml"), toml_content).unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        let result = manager.get("test");
        assert!(result.is_ok());
        assert!(result.unwrap().contains("echo hello"));
    }

    /// Directory holding the premade libraries that ship with the server.
    fn shipped_premade_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("premade-libraries")
    }

    #[test]
    fn test_every_shipped_premade_library_is_valid_toml() {
        let dir = shipped_premade_dir();
        let mut entries: Vec<_> = fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .flatten()
            .collect();
        entries.sort_by_key(|e| e.path());

        let mut checked = 0usize;
        for entry in entries {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                continue;
            }
            let raw = fs::read_to_string(&path).unwrap();
            // Parse the SHIPPED bytes verbatim — no escape fixing, no
            // rewriting. A shipped file must already be valid TOML.
            let parsed: SnippetFile = toml::from_str(&raw).unwrap_or_else(|e| {
                panic!(
                    "shipped premade library {} is not valid TOML: {e}",
                    path.display()
                )
            });
            assert!(
                !parsed.snippets.is_empty(),
                "shipped premade library {} has no snippets",
                path.display()
            );
            for snippet in &parsed.snippets {
                let cmd = snippet.command.as_deref().unwrap_or("");
                assert!(
                    !cmd.is_empty(),
                    "snippet in {} has no command — the server deserializer \
                     and the shipped key casing disagree",
                    path.display()
                );
            }
            checked += 1;
        }
        assert!(
            checked > 0,
            "no shipped premade libraries found in {}",
            dir.display()
        );
    }

    #[test]
    fn test_premade_manager_lists_all_shipped_libraries() {
        let manager = PremadeManager::new(shipped_premade_dir());
        let libs = manager.list();
        let names: Vec<&str> = libs.iter().map(|l| l.name.as_str()).collect();
        assert!(
            names.contains(&"bash-scripts"),
            "bash-scripts missing from shipped premade libraries: {names:?}"
        );
        for lib in &libs {
            assert!(
                lib.snippet_count > 0,
                "library {} reported {} snippets",
                lib.name,
                lib.snippet_count
            );
        }
    }

    #[test]
    fn test_bash_scripts_topcmds_command_parses() {
        let path = shipped_premade_dir().join("bash-scripts.toml");
        let raw = fs::read_to_string(&path).unwrap();
        let parsed: SnippetFile = toml::from_str(&raw).unwrap();
        let cmd = parsed
            .snippets
            .iter()
            .find(|s| s.command.as_deref().unwrap_or("").contains("topcmds"))
            .and_then(|s| s.command.clone())
            .expect("topcmds snippet");
        // The awk program must reference field 2 (`$2`), never an invalid
        // TOML escape.
        assert!(cmd.contains("{print $2}"), "unexpected command: {cmd}");
    }

    #[test]
    fn test_get_nonexistent_library() {
        let dir = TempDir::new().unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        let result = manager.get("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_path_traversal_rejected() {
        let dir = TempDir::new().unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        let result = manager.get("../../etc/passwd");
        assert!(result.is_err());
        let err = result.unwrap_err();
        // When canonicalize fails, we reject for safety (can't verify path is safe)
        assert!(err.message().contains("cannot canonicalize"));
    }

    #[test]
    fn test_get_dot_dot_slash_rejected() {
        let dir = TempDir::new().unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        let result = manager.get("../escape");
        assert!(result.is_err());
    }

    #[test]
    fn test_fix_invalid_toml_escapes_backslash_angle() {
        let input = r#"command = "echo \<hello\>""#;
        let output = fix_invalid_toml_escapes(input);
        assert!(output.contains("'echo \\<hello\\>'"));
    }

    #[test]
    fn test_fix_invalid_toml_escapes_no_fix_needed() {
        let input = r#"command = "echo normal""#;
        let output = fix_invalid_toml_escapes(input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_scan_directory_empty() {
        let dir = TempDir::new().unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        assert!(manager.is_empty());
    }

    #[test]
    fn test_scan_directory_with_valid_library() {
        let dir = TempDir::new().unwrap();
        let toml_content = r#"
Description = "Docker snippets"

[[Snippets]]
Command = "docker ps"
Description = "List containers"
"#;
        fs::write(dir.path().join("docker.toml"), toml_content).unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        assert_eq!(manager.list().len(), 1);
        assert_eq!(manager.list()[0].name, "docker");
    }

    #[test]
    fn test_scan_directory_skips_non_toml() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("readme.txt"), "not a toml").unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        assert!(manager.is_empty());
    }

    #[test]
    fn test_scan_directory_skips_empty_snippets() {
        let dir = TempDir::new().unwrap();
        let toml_content = r#"
Description = "Empty library"

[[Snippets]]
Command = ""
Description = "Empty command"
"#;
        fs::write(dir.path().join("empty.toml"), toml_content).unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        assert!(manager.is_empty());
    }

    #[test]
    fn test_search_by_name() {
        let dir = TempDir::new().unwrap();
        let toml1 = r#"
Description = "Docker containers"

[[Snippets]]
Command = "docker ps"
Description = "List containers"
Tag = ["docker"]
"#;
        let toml2 = r#"
Description = "Git commands"

[[Snippets]]
Command = "git status"
Description = "Check status"
Tag = ["git"]
"#;
        fs::write(dir.path().join("docker.toml"), toml1).unwrap();
        fs::write(dir.path().join("git.toml"), toml2).unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        let results = manager.search("docker");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "docker");
    }

    #[test]
    fn test_search_by_description() {
        let dir = TempDir::new().unwrap();
        let toml1 = r#"
Description = "Container orchestration"

[[Snippets]]
Command = "docker compose up"
Description = "Start services"
Tag = ["docker"]
"#;
        fs::write(dir.path().join("docker.toml"), toml1).unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        let results = manager.search("container");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "docker");
    }

    #[test]
    fn test_search_by_tag() {
        let dir = TempDir::new().unwrap();
        let toml1 = r#"
Description = "Docker basics"

[[Snippets]]
Command = "docker ps"
Description = "List"
Tag = ["docker", "containers"]
"#;
        fs::write(dir.path().join("docker.toml"), toml1).unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        let results = manager.search("containers");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "docker");
        assert!(results[0].tags.contains(&"docker".to_string()));
        assert!(results[0].tags.contains(&"containers".to_string()));
    }

    #[test]
    fn test_search_case_insensitive() {
        let dir = TempDir::new().unwrap();
        let toml1 = r#"
Description = "Docker commands"

[[Snippets]]
Command = "docker ps"
Description = "List"
Tag = ["docker"]
"#;
        fs::write(dir.path().join("docker.toml"), toml1).unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        let results = manager.search("DOCKER");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_no_results() {
        let dir = TempDir::new().unwrap();
        let toml1 = r#"
Description = "Docker commands"

[[Snippets]]
Command = "docker ps"
Description = "List"
Tag = ["docker"]
"#;
        fs::write(dir.path().join("docker.toml"), toml1).unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        let results = manager.search("nonexistent");
        assert!(results.is_empty());
    }

    #[test]
    fn test_scan_collects_tags() {
        let dir = TempDir::new().unwrap();
        let toml_content = r#"
Description = "Multi-tag library"

[[Snippets]]
Command = "docker ps"
Description = "List"
Tag = ["docker", "containers"]

[[Snippets]]
Command = "docker images"
Description = "Images"
Tag = ["docker", "images"]
"#;
        fs::write(dir.path().join("multi.toml"), toml_content).unwrap();
        let manager = PremadeManager::new(dir.path().to_path_buf());
        let libs = manager.list();
        assert_eq!(libs.len(), 1);
        assert!(libs[0].tags.contains(&"docker".to_string()));
        assert!(libs[0].tags.contains(&"containers".to_string()));
        assert!(libs[0].tags.contains(&"images".to_string()));
        assert_eq!(libs[0].snippet_count, 2);
    }
}
