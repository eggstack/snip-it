use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use tonic::Status;

/// Fixes invalid TOML escape sequences (`\<` and `\>`) in double-quoted strings.
/// Converts affected strings to single-quoted (raw literal) form, preserving the
/// backslash content. Only processes double-quoted strings; single-quoted strings
/// and non-string content are left unchanged.
fn fix_invalid_toml_escapes(toml_str: &str) -> String {
    let mut result = String::with_capacity(toml_str.len());
    let mut chars = toml_str.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '"' {
            let mut string_content = String::new();
            let mut needs_fix = false;

            while let Some(&next) = chars.peek() {
                chars.next();
                if next == '"' {
                    break;
                }
                if next == '\\' {
                    if let Some(&after) = chars.peek() {
                        chars.next();
                        string_content.push('\\');
                        string_content.push(after);
                        if after == '<' || after == '>' {
                            needs_fix = true;
                        }
                    } else {
                        string_content.push('\\');
                    }
                } else {
                    string_content.push(next);
                }
            }

            if needs_fix && !string_content.contains('\'') {
                result.push('\'');
                result.push_str(&string_content);
                result.push('\'');
            } else if needs_fix {
                result.push('"');
                for ch in string_content.chars() {
                    if ch == '\\' {
                        result.push('\\');
                        result.push('\\');
                    } else {
                        result.push(ch);
                    }
                }
                result.push('"');
            } else {
                result.push('"');
                result.push_str(&string_content);
                result.push('"');
            }
        } else {
            result.push(c);
        }
    }

    result
}

#[derive(Debug, Clone)]
pub struct PremadeLibrary {
    pub name: String,
    pub filename: String,
    pub description: String,
    pub snippet_count: i32,
}

#[derive(Debug, Deserialize)]
struct Snippet {
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

        let canonical_dir = self.dir.canonicalize().unwrap_or_else(|_| self.dir.clone());
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());

        if !canonical_path.starts_with(&canonical_dir) {
            return Err(Status::invalid_argument(
                "Invalid filename: path traversal detected",
            ));
        }

        fs::read_to_string(&canonical_path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Status::not_found(format!("Premade library '{}' not found", filename))
            } else {
                Status::internal(format!("Failed to read premade library: {}", e))
            }
        })
    }

    pub fn is_empty(&self) -> bool {
        self.libraries.is_empty()
    }
}
