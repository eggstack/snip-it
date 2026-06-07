//! Configuration directory and path management.
//!
//! Handles platform-specific config directory resolution (XDG on Linux,
//! Application Support on macOS, AppData on Windows) and macOS legacy
//! config directory migration.

use std::path::{Path, PathBuf};

/// Returns the path to the user's snp config directory without touching
/// the filesystem. Callers that need the directory to exist (and to have
/// restrictive permissions on Unix) should call [`ensure_config_dir`]
/// once at startup.
pub fn get_config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| {
                    tracing::warn!("Home directory not found, using current directory for config");
                    PathBuf::from(".")
                })
                .join(".config")
        })
        .join("snp")
}

/// Creates the config directory (if missing) and tightens its permissions
/// to `0o700` on Unix. Idempotent and safe to call multiple times.
///
/// Callers should invoke this once during startup before performing any
/// I/O inside the config directory. Individual I/O helpers (logging
/// initialization, library creation, premade downloads, audit logging)
/// also call this defensively in case the startup hook was skipped.
pub fn ensure_config_dir() -> std::io::Result<PathBuf> {
    let dir = get_config_dir();

    if dir.exists() {
        // Tighten permissions on existing directory if needed
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = std::fs::metadata(&dir) {
                let mode = metadata.permissions().mode();
                if mode & 0o077 != 0 {
                    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
                }
            }
        }
    } else {
        std::fs::create_dir_all(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }

    Ok(dir)
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
    tracing::info!(
        from = %legacy_dir.display(),
        to = %new_dir.display(),
        "Migrating config directory"
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

    tracing::info!("Config migration complete");
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

    #[test]
    fn test_get_config_dir_is_deterministic() {
        // Calling get_config_dir() must return the same path on repeated
        // calls. This is the contract that lets callers use it as a cheap
        // path builder without worrying about I/O.
        let a = get_config_dir();
        let b = get_config_dir();
        assert_eq!(a, b);
    }

    #[test]
    fn test_ensure_config_dir_is_idempotent() {
        // Call twice — both should succeed without error and return the
        // same path. The second call exercises the "dir already exists"
        // branch (including the permission-tightening check on Unix).
        let a = ensure_config_dir().expect("first ensure_config_dir");
        let b = ensure_config_dir().expect("second ensure_config_dir");
        assert_eq!(a, b);
        assert!(
            a.exists(),
            "config dir should exist after ensure_config_dir"
        );
    }
}
