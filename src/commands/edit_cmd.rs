use crate::commands::get_library_path;
use crate::error::{SnipError, SnipResult};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn run(library: Option<String>, _config: Option<PathBuf>) -> SnipResult<()> {
    let path = if let Some(ref lib_name) = library {
        match get_library_path(library.clone())? {
            Some(p) => p,
            None => {
                eprintln!("Library '{}' not found", lib_name);
                return Ok(());
            }
        }
    } else {
        get_library_path(None)?
            .unwrap_or_else(crate::library::LibraryManager::get_default_snippets_path)
    };
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        File::create(&path)?;
    }
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());

    let resolved_editor = resolve_editor(&editor)?;

    Command::new(&resolved_editor)
        .arg(&path)
        .status()
        .map_err(|e| {
            SnipError::command_error(&resolved_editor, vec![path.display().to_string()], e)
        })?;
    Ok(())
}

fn has_directory_component(editor: &str) -> bool {
    editor.contains('/') || (cfg!(windows) && editor.contains('\\')) || editor.starts_with('.')
}

fn resolve_editor(editor: &str) -> SnipResult<String> {
    let editor_path = Path::new(editor);

    if editor_path.is_absolute() {
        if !editor_path.exists() {
            return Err(SnipError::runtime_error(
                "Editor not found",
                Some(&format!(
                    "EDITOR '{}' does not exist. Set EDITOR to a valid editor path.",
                    editor
                )),
            ));
        }
        if !editor_path.is_file() {
            return Err(SnipError::runtime_error(
                "Editor is not a file",
                Some(&format!(
                    "EDITOR '{}' exists but is not a file (it may be a directory). \
                     Set EDITOR to a valid editor executable.",
                    editor
                )),
            ));
        }
        return Ok(editor.to_string());
    }

    // Relative path with directory components: resolve against CWD
    if has_directory_component(editor) {
        let cwd = std::env::current_dir().map_err(|e| {
            SnipError::runtime_error(
                "Failed to get current directory",
                Some(&format!("Cannot resolve relative editor path: {}", e)),
            )
        })?;
        let candidate = cwd.join(editor);
        if !candidate.exists() {
            return Err(SnipError::runtime_error(
                "Editor not found",
                Some(&format!(
                    "EDITOR '{}' does not exist relative to {}.",
                    editor,
                    cwd.display()
                )),
            ));
        }
        if !candidate.is_file() {
            return Err(SnipError::runtime_error(
                "Editor is not a file",
                Some(&format!(
                    "EDITOR '{}' exists but is not a file.",
                    candidate.display()
                )),
            ));
        }

        let canonical = candidate.canonicalize().map_err(|e| {
            SnipError::runtime_error(
                "Editor path resolution failed",
                Some(&format!(
                    "Cannot resolve editor path '{}': {}",
                    candidate.display(),
                    e
                )),
            )
        })?;

        let canonical_cwd = cwd.canonicalize().map_err(|e| {
            SnipError::runtime_error(
                "Current directory resolution failed",
                Some(&format!("Cannot canonicalize CWD: {}", e)),
            )
        })?;

        if !canonical.starts_with(&canonical_cwd) {
            return Err(SnipError::runtime_error(
                "Editor path unsafe",
                Some(&format!(
                    "EDITOR '{}' resolves outside of current directory (possible symlink attack). Use an absolute path.",
                    editor
                )),
            ));
        }

        return Ok(candidate.to_string_lossy().into_owned());
    }

    // Bare name: search PATH
    let path_var = std::env::var("PATH").unwrap_or_default();

    for dir in path_var.split(if cfg!(windows) { ';' } else { ':' }) {
        if dir.is_empty() {
            continue;
        }
        let candidate = Path::new(dir).join(editor);
        if candidate.exists() && candidate.is_file() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
        #[cfg(windows)]
        {
            for ext in &[".exe", ".cmd", ".bat"] {
                let with_ext = Path::new(dir).join(format!("{}{}", editor, ext));
                if with_ext.exists() && with_ext.is_file() {
                    return Ok(with_ext.to_string_lossy().into_owned());
                }
            }
        }
    }

    Err(SnipError::runtime_error(
        "Editor not found",
        Some(&format!(
            "EDITOR '{}' is not an absolute path and could not be found in PATH. \
             Set EDITOR to an absolute path (e.g., /usr/bin/vim).",
            editor
        )),
    ))
}
