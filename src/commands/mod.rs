pub mod clip_cmd;
pub mod cron_cmd;
pub mod edit_cmd;
pub mod keybindings_cmd;
pub mod library_cmd;
pub mod list_cmd;
pub mod new_cmd;
pub mod premade_cmd;
pub mod register_cmd;
pub mod run_cmd;
pub mod search_cmd;
pub mod sync_cmd;

use crate::error::{SnipError, SnipResult};
use std::path::PathBuf;

pub fn get_config_path(config: &Option<PathBuf>) -> SnipResult<PathBuf> {
    use std::fs::{self, File};

    match config {
        Some(path) => {
            if path.exists() {
                Ok(path.clone())
            } else {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|e| SnipError::io_error("create directory", parent, e))?;
                }
                File::create(path)
                    .map_err(|e| SnipError::io_error("create config file", path.clone(), e))?;
                Ok(path.clone())
            }
        }
        None => Ok(crate::CONFIG_PATH.clone()),
    }
}

pub fn get_library_path(library_name: Option<String>) -> SnipResult<Option<PathBuf>> {
    use crate::library::LibraryManager;

    let mut mgr = LibraryManager::new()?;

    if let Err(e) = mgr.ensure_library_mode() {
        eprintln!("Warning: Failed to ensure library mode: {}", e);
    }

    let path = if let Some(name) = library_name {
        if let Some(lib) = mgr.get_library_by_filename(&name) {
            Some(
                mgr.get_libraries_dir()
                    .join(format!("{}.toml", lib.filename)),
            )
        } else {
            return Err(SnipError::runtime_error(
                "Library not found",
                Some(&format!("Library '{}' does not exist. Use 'snp library list' to see available libraries.", name)),
            ));
        }
    } else {
        mgr.get_primary_library().map(|primary| {
            mgr.get_libraries_dir()
                .join(format!("{}.toml", primary.filename))
        })
    };

    Ok(path)
}

pub fn load_snippets(config: &Option<PathBuf>) -> SnipResult<crate::library::Snippets> {
    use std::fs;

    let path = get_config_path(config)?;
    crate::logging::log_config_operation("load", &path, &Ok(()));

    if !path.exists() {
        return Ok(crate::library::Snippets::default());
    }

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            crate::logging::log_config_operation("load", &path, &Err(&e.to_string()));
            return Ok(crate::library::Snippets::default());
        }
    };

    if content.is_empty() || content.trim().is_empty() {
        return Ok(crate::library::Snippets::default());
    }

    let snippets: crate::library::Snippets = match toml::from_str(&content) {
        Ok(s) => s,
        Err(e) => {
            crate::logging::log_config_operation("parse", &path, &Err(&e.to_string()));
            eprintln!(
                "Warning: Failed to parse config file, using defaults: {}",
                e
            );
            crate::library::Snippets::default()
        }
    };

    Ok(snippets)
}

pub fn save_snippets(s: &crate::library::Snippets, config: &Option<PathBuf>) -> SnipResult<()> {
    use std::fs;

    let path = get_config_path(config)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| SnipError::io_error("create config directory", parent, e))?;
    }

    let toml_str =
        toml::to_string_pretty(s).map_err(|e| SnipError::toml_error("serialize config", e))?;

    fs::write(&path, toml_str)
        .map_err(|e| SnipError::io_error("write config file", path.clone(), e))?;

    crate::logging::log_config_operation("save", &path, &Ok(()));
    Ok(())
}

pub fn get_snippet_data(snippets: &crate::library::Snippets) -> crate::SnippetData {
    let descriptions: Vec<String> = snippets
        .snippets
        .iter()
        .map(|s| s.description.clone())
        .collect();
    let commands: Vec<String> = snippets
        .snippets
        .iter()
        .map(|s| s.command.clone())
        .collect();
    let tags: Vec<Vec<String>> = snippets.snippets.iter().map(|s| s.tags.clone()).collect();
    let folders: Vec<Vec<String>> = snippets
        .snippets
        .iter()
        .map(|s| s.folders.clone())
        .collect();
    let favorites: Vec<bool> = snippets.snippets.iter().map(|s| s.favorite).collect();
    (descriptions, commands, tags, folders, favorites)
}
