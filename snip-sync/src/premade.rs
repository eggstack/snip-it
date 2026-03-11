use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use tonic::Status;

#[derive(Debug, Clone)]
pub struct PremadeLibrary {
    pub name: String,
    pub filename: String,
    pub description: String,
    pub snippet_count: i32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Snippet {
    #[serde(rename = "Description", default)]
    description: Option<String>,
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

            let snippet_count = snippet_file.snippets.len() as i32;

            let description = snippet_file.description.unwrap_or_else(|| filename.clone());

            libraries.push(PremadeLibrary {
                name: filename.clone(),
                filename,
                description,
                snippet_count,
            });
        }

        tracing::info!("Loaded {} premade libraries", libraries.len());
        libraries
    }

    pub fn list(&self) -> Vec<PremadeLibrary> {
        self.libraries.clone()
    }

    pub fn get(&self, filename: &str) -> Result<String, Status> {
        let path = self.dir.join(format!("{}.toml", filename));

        if !path.starts_with(&self.dir) {
            return Err(Status::invalid_argument(
                "Invalid filename: path traversal detected",
            ));
        }

        if !path.exists() {
            return Err(Status::not_found(format!(
                "Premade library '{}' not found",
                filename
            )));
        }

        fs::read_to_string(&path)
            .map_err(|e| Status::internal(format!("Failed to read premade library: {}", e)))
    }

    pub fn is_empty(&self) -> bool {
        self.libraries.is_empty()
    }
}
