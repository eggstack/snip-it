//! CLI command implementations.
//!
//! Each subcommand in the CLI has its own module with a `run()` function.
//! This module also provides shared utilities for loading and saving snippets.

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
use crate::utils::toml_helpers::{fix_invalid_toml_escapes, quote_strings_containing_backslashes};
use std::path::PathBuf;

/// Result of expanding a snippet command with variables.
pub enum ExpandedCommand {
    /// User cancelled the operation.
    Cancel,
    /// User chose to skip (continue to next snippet).
    Skip,
    /// Command was expanded successfully.
    Expanded(String),
}

/// Resolves the config path from CLI argument or default location.
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

/// Resolves the path to a named library or returns the primary library path.
pub fn get_library_path(library_name: Option<String>) -> SnipResult<Option<PathBuf>> {
    use crate::library::LibraryManager;

    let mut mgr = LibraryManager::new()?;

    if let Err(e) = mgr.ensure_library_mode() {
        eprintln!("Warning: Failed to ensure library mode: {}", e);
    }

    let path = match library_name {
        Some(name) => {
            let lib = mgr.get_library_by_filename(&name)
                .ok_or_else(|| SnipError::runtime_error(
                    "Library not found",
                    Some(&format!("Library '{}' does not exist. Use 'snp library list' to see available libraries.", name)),
                ))?;
            Some(
                mgr.get_libraries_dir()
                    .join(format!("{}.toml", lib.filename)),
            )
        }
        None => mgr.get_primary_library().map(|primary| {
            mgr.get_libraries_dir()
                .join(format!("{}.toml", primary.filename))
        }),
    };

    Ok(path)
}

/// Initializes a LibraryManager with library mode enabled, handling errors gracefully.
pub fn init_library_manager() -> SnipResult<crate::library::LibraryManager> {
    let mut mgr = crate::library::LibraryManager::new()?;
    if let Err(e) = mgr.ensure_library_mode() {
        eprintln!("Warning: Failed to ensure library mode: {}", e);
    }
    Ok(mgr)
}

/// Loads snippets from a TOML file, returning an empty collection if the file doesn't exist.
pub fn load_snippets(config: &Option<PathBuf>) -> SnipResult<crate::library::Snippets> {
    use std::fs;

    let path = get_config_path(config)?;
    crate::logging::log_config_operation("load", &path, &Ok(()));

    if !path.exists() {
        return Ok(crate::library::Snippets::default());
    }

    let content = fs::read_to_string(&path).map_err(|e| {
        crate::logging::log_config_operation("load", &path, &Err(&e.to_string()));
        SnipError::io_error("read snippets file", path.clone(), e)
    })?;

    if content.is_empty() || content.trim().is_empty() {
        return Ok(crate::library::Snippets::default());
    }

    let fixed_content = fix_invalid_toml_escapes(&content);

    let snippets: crate::library::Snippets = toml::from_str(&fixed_content).map_err(|e| {
        crate::logging::log_config_operation("parse", &path, &Err(&e.to_string()));
        let backup_path = path.with_extension("toml.bak");
        if let Err(backup_err) = std::fs::copy(&path, &backup_path) {
            eprintln!(
                "Warning: Failed to parse config and could not create backup: {} (backup error: {})",
                e, backup_err
            );
        } else {
            eprintln!(
                "Warning: Failed to parse config file. Backup saved to {}.",
                backup_path.display()
            );
        }
        SnipError::toml_error("parse snippets file", e)
    })?;

    Ok(snippets)
}

/// Saves snippets to a TOML file, creating directories as needed.
pub fn save_snippets(s: &crate::library::Snippets, config: &Option<PathBuf>) -> SnipResult<()> {
    use std::fs;

    let path = get_config_path(config)?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| SnipError::io_error("create config directory", parent, e))?;
    }

    let toml_str =
        toml::to_string_pretty(s).map_err(|e| SnipError::toml_error("serialize config", e))?;

    let toml_str = quote_strings_containing_backslashes(&toml_str);

    fs::write(&path, toml_str)
        .map_err(|e| SnipError::io_error("write config file", path.clone(), e))?;

    crate::logging::log_config_operation("save", &path, &Ok(()));
    Ok(())
}

/// Extracts snippet data arrays for TUI display.
///
/// Returns parallel arrays of descriptions, commands, tags, folders, and favorites.
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
    crate::SnippetData {
        descriptions,
        commands,
        tags,
        folders,
        favorites,
    }
}

pub fn expand_snippet_command(snippet: &crate::library::Snippet) -> SnipResult<ExpandedCommand> {
    use crate::ui;
    use crate::utils::{parse_variables, strip_escape_sequences};

    let vars = parse_variables(&snippet.command);
    if vars.is_empty() {
        return Ok(ExpandedCommand::Expanded(strip_escape_sequences(
            &snippet.command,
        )));
    }

    match ui::prompt_variables(vars)? {
        ui::VariablePromptResult::Cancel => Ok(ExpandedCommand::Cancel),
        ui::VariablePromptResult::Skip => Ok(ExpandedCommand::Skip),
        ui::VariablePromptResult::Values(values) => Ok(ExpandedCommand::Expanded(
            crate::utils::expand_command(&snippet.command, &values),
        )),
    }
}

pub fn run_snippet_selection<F>(
    filter: Option<String>,
    library: Option<String>,
    do_sync: bool,
    runtime: &tokio::runtime::Runtime,
    mut process_fn: F,
) -> crate::error::SnipResult<()>
where
    F: FnMut(
        &crate::library::Snippet,
        Option<String>,
    ) -> crate::error::SnipResult<crate::ProcessResult>,
{
    let lib_path = match get_library_path(library)? {
        Some(p) => p,
        None => {
            eprintln!("No library found. Create one with 'snp library create <name>'");
            return Ok(());
        }
    };
    let snippets = crate::library::load_library(&lib_path)?;
    let snippet_data = get_snippet_data(&snippets);

    loop {
        let result = crate::ui::select_snippet(
            &snippet_data.descriptions,
            &snippet_data.commands,
            &snippet_data.tags,
            false,
            filter.as_deref(),
            &snippet_data.folders,
            &snippet_data.favorites,
        )?;
        if let Some((idx, copy_flag)) = result {
            let snippet = &snippets.snippets[idx];
            match process_fn(snippet, copy_flag)? {
                crate::ProcessResult::Cancel => {
                    if do_sync {
                        crate::sync_commands::run_default_sync(runtime);
                    }
                    return Ok(());
                }
                crate::ProcessResult::Continue => continue,
                crate::ProcessResult::Done(_msg) => {
                    break;
                }
            }
        } else {
            break;
        }
    }
    if do_sync {
        crate::sync_commands::run_default_sync(runtime);
    }
    Ok(())
}
