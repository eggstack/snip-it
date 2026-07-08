use crate::commands::get_library_path;
use crate::error::{SnipError, SnipResult};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Opens the snippets library file in the user's `$EDITOR`.
pub fn run(library: Option<String>, _config: Option<PathBuf>) -> SnipResult<()> {
    let path = if let Some(ref lib_name) = library {
        match get_library_path(library.clone())? {
            Some(p) => p,
            None => {
                eprintln!(
                    "Library '{lib_name}' not found. Use 'snp library list' to see available libraries."
                );
                return Err(crate::error::SnipError::runtime_error(
                    "Library not found",
                    Some(&format!("Library '{lib_name}' does not exist")),
                ));
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
                    "EDITOR '{editor}' does not exist. Set EDITOR to a valid editor path."
                )),
            ));
        }
        if !editor_path.is_file() {
            return Err(SnipError::runtime_error(
                "Editor is not a file",
                Some(&format!(
                    "EDITOR '{editor}' exists but is not a file (it may be a directory). \
                     Set EDITOR to a valid editor executable."
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
                Some(&format!("Cannot resolve relative editor path: {e}")),
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
                Some(&format!("Cannot canonicalize CWD: {e}")),
            )
        })?;

        if !canonical.starts_with(&canonical_cwd) {
            return Err(SnipError::runtime_error(
                "Editor path unsafe",
                Some(&format!(
                    "EDITOR '{editor}' resolves outside of current directory (possible symlink attack). Use an absolute path."
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
            "EDITOR '{editor}' is not an absolute path and could not be found in PATH. \
             Set EDITOR to an absolute path (e.g., /usr/bin/vim)."
        )),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_directory_component_slash() {
        assert!(has_directory_component("/usr/bin/vim"));
        assert!(has_directory_component("./vim"));
        assert!(has_directory_component("../vim"));
        assert!(has_directory_component("path/to/vim"));
    }

    #[test]
    fn test_has_directory_component_bare_name() {
        assert!(!has_directory_component("vim"));
        assert!(!has_directory_component("nano"));
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_editor_absolute_path_exists() {
        let result = resolve_editor("/bin/sh");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "/bin/sh");
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_editor_absolute_path_not_exists() {
        let result = resolve_editor("/nonexistent/editor");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_editor_absolute_path_is_directory() {
        let result = resolve_editor("/usr/bin");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("is not a file"));
    }

    #[test]
    #[cfg(not(windows))]
    fn test_resolve_editor_bare_name_in_path() {
        let result = resolve_editor("sh");
        assert!(result.is_ok());
        assert!(result.unwrap().ends_with("sh"));
    }

    #[test]
    fn test_resolve_editor_bare_name_not_in_path() {
        let result = resolve_editor("nonexistent-editor-xyz");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("could not be found in PATH"));
    }
}
