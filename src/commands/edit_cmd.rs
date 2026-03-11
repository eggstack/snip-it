use crate::commands::get_library_path;
use crate::error::SnipResult;
use std::fs::{self, File};
use std::path::PathBuf;
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
    let editor_path = std::path::Path::new(&editor);

    if !editor_path.is_absolute() {
        eprintln!(
            "Warning: EDITOR is not an absolute path. This may be insecure. Using '{}'.",
            editor
        );
    }

    Command::new(&editor).arg(path.to_str().unwrap()).status()?;
    Ok(())
}
