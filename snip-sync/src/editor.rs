use std::path::Path;

pub fn resolve_editor(editor: &str) -> Result<String, String> {
    let editor_path = Path::new(editor);

    if editor_path.is_absolute() {
        if !editor_path.exists() {
            return Err(format!(
                "EDITOR '{}' does not exist. Set EDITOR to a valid editor path.",
                editor
            ));
        }
        if !editor_path.is_file() {
            return Err(format!(
                "EDITOR '{}' exists but is not a file (it may be a directory). \
                 Set EDITOR to a valid editor executable.",
                editor
            ));
        }
        return Ok(editor.to_string());
    }

    if has_directory_component(editor) {
        let cwd = std::env::current_dir()
            .map_err(|e| format!("Cannot resolve relative editor path: {}", e))?;
        let candidate = cwd.join(editor);
        if !candidate.exists() {
            return Err(format!(
                "EDITOR '{}' does not exist relative to {}.",
                editor,
                cwd.display()
            ));
        }
        if !candidate.is_file() {
            return Err(format!(
                "EDITOR '{}' exists but is not a file.",
                candidate.display()
            ));
        }
        let canonical = candidate.canonicalize().map_err(|e| {
            format!(
                "Cannot resolve editor path '{}': {}",
                candidate.display(),
                e
            )
        })?;
        let canonical_cwd = cwd
            .canonicalize()
            .map_err(|e| format!("Cannot canonicalize CWD: {}", e))?;
        if !canonical.starts_with(&canonical_cwd) {
            return Err(format!(
                "EDITOR '{}' resolves outside of current directory (possible symlink attack). Use an absolute path.",
                editor
            ));
        }
        return Ok(candidate.to_string_lossy().into_owned());
    }

    let path_var = std::env::var("PATH").unwrap_or_default();
    let path_sep = if cfg!(windows) { ';' } else { ':' };
    for dir in path_var.split(path_sep) {
        if dir.is_empty() {
            continue;
        }
        let candidate = Path::new(dir).join(editor);
        if candidate.exists() && candidate.is_file() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
    }

    Err(format!(
        "EDITOR '{}' is not an absolute path and could not be found in PATH. \
         Set EDITOR to an absolute path (e.g., /usr/bin/vim).",
        editor
    ))
}

fn has_directory_component(editor: &str) -> bool {
    editor.contains('/') || (cfg!(windows) && editor.contains('\\')) || editor.starts_with('.')
}

pub fn get_editor() -> Result<String, String> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
    resolve_editor(&editor)
}

pub fn open_in_editor(path: &Path) -> Result<(), String> {
    let editor = get_editor()?;
    let status = std::process::Command::new(&editor)
        .arg(path)
        .status()
        .map_err(|e| format!("Failed to run editor '{}': {}", editor, e))?;
    if !status.success() {
        return Err(format!(
            "Editor '{}' exited with status: {}",
            editor, status
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_directory_component() {
        assert!(has_directory_component("/usr/bin/vim"));
        assert!(has_directory_component("./vim"));
        assert!(has_directory_component("../vim"));
        assert!(has_directory_component("path/to/vim"));
        assert!(!has_directory_component("vim"));
        assert!(!has_directory_component("nano"));
    }

    #[test]
    fn test_resolve_editor_absolute_exists() {
        let editor = if cfg!(windows) {
            "C:\\Windows\\System32\\cmd.exe"
        } else {
            "/bin/sh"
        };
        let result = resolve_editor(editor);
        assert!(result.is_ok(), "failed: {:?}", result.err());
        assert_eq!(result.unwrap(), editor);
    }

    #[test]
    fn test_resolve_editor_absolute_not_exists() {
        let editor = if cfg!(windows) {
            "C:\\nonexistent\\editor.exe"
        } else {
            "/nonexistent/editor"
        };
        let result = resolve_editor(editor);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[test]
    fn test_resolve_editor_absolute_is_dir() {
        let editor = if cfg!(windows) {
            "C:\\Windows"
        } else {
            "/usr/bin"
        };
        let result = resolve_editor(editor);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("is not a file"));
    }

    #[test]
    fn test_resolve_editor_bare_in_path() {
        let editor = if cfg!(windows) { "cmd.exe" } else { "sh" };
        let result = resolve_editor(editor);
        assert!(result.is_ok(), "failed: {:?}", result.err());
        assert!(result.unwrap().ends_with(editor));
    }

    #[test]
    fn test_resolve_editor_bare_not_in_path() {
        let result = resolve_editor("nonexistent-editor-xyz-abc");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("could not be found in PATH"));
    }
}
