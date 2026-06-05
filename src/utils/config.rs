//! Configuration directory and path management.
//!
//! Handles platform-specific config directory resolution (XDG on Linux,
//! Application Support on macOS, AppData on Windows) and macOS legacy
//! config directory migration.

use std::path::{Path, PathBuf};

pub fn get_config_dir() -> PathBuf {
    let dir = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        })
        .join("snp");

    #[cfg(unix)]
    {
        if !dir.exists() {
            let _ = std::fs::create_dir_all(&dir);
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
    }

    dir
}

/// Returns the old macOS platform-specific config directory
/// (`~/Library/Application Support/snp/`) if it exists and the
/// canonical config directory (`~/.config/snp/`) does not.
/// This is used to detect data that needs migration.
pub fn get_legacy_macos_config_dir() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let new_dir = get_config_dir();
    if new_dir.exists() {
        return None;
    }
    let legacy_dir = dirs::config_dir()?.join("snp");
    if legacy_dir.exists() && legacy_dir != new_dir {
        Some(legacy_dir)
    } else {
        None
    }
}

/// Recursively copies a file or directory from `src` to `dst`.
fn copy_recursively(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursively(&entry.path(), &dst.join(entry.file_name()))?;
        }
    } else {
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

/// Migrates config data from the old macOS path
/// (`~/Library/Application Support/snp/`) to the canonical path
/// (`~/.config/snp/`). Moves all files and directories.
pub fn migrate_macos_config_dir() -> std::io::Result<()> {
    let legacy_dir = match get_legacy_macos_config_dir() {
        Some(d) => d,
        None => return Ok(()),
    };

    let new_dir = get_config_dir();
    eprintln!(
        "Migrating config from {} to {}",
        legacy_dir.display(),
        new_dir.display()
    );

    std::fs::create_dir_all(&new_dir)?;

    for entry in std::fs::read_dir(&legacy_dir)? {
        let entry = entry?;
        let src = entry.path();
        let dst = new_dir.join(entry.file_name());
        if std::fs::rename(&src, &dst).is_ok() {
            continue;
        }
        copy_recursively(&src, &dst)?;
        if src.is_dir() {
            let _ = std::fs::remove_dir_all(&src);
        } else {
            let _ = std::fs::remove_file(&src);
        }
    }

    // Remove legacy dir if it's now empty
    if std::fs::read_dir(&legacy_dir)?.next().is_none() {
        let _ = std::fs::remove_dir(&legacy_dir);
    }

    eprintln!("Migration complete.");
    Ok(())
}

pub fn get_config_path(filename: &str) -> PathBuf {
    get_config_dir().join(filename)
}

pub fn get_snippets_path() -> PathBuf {
    get_config_path("snippets.toml")
}

pub fn get_sync_config_path() -> PathBuf {
    get_config_path("sync.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_config_dir_contains_snp() {
        let dir = get_config_dir();
        assert!(dir.to_string_lossy().ends_with("snp"));
    }

    #[test]
    fn test_get_config_path_adds_filename() {
        let path = get_config_path("test.toml");
        assert!(path.to_string_lossy().ends_with("test.toml"));
    }

    #[test]
    fn test_get_snippets_path_ends_with_snippets_toml() {
        let path = get_snippets_path();
        assert!(path.to_string_lossy().ends_with("snippets.toml"));
    }

    #[test]
    fn test_get_sync_config_path_ends_with_sync_toml() {
        let path = get_sync_config_path();
        assert!(path.to_string_lossy().ends_with("sync.toml"));
    }
}
