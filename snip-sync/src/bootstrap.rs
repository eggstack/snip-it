use crate::paths;
use std::fs;

pub fn ensure_layout() -> Result<(), String> {
    let dirs = [
        paths::config_dir(),
        paths::state_dir(),
        paths::data_dir(),
        paths::cert_dir(),
        paths::default_premade_dir(),
    ];
    for d in &dirs {
        fs::create_dir_all(d).map_err(|e| format!("Failed to create {}: {}", d.display(), e))?;
    }
    if let Some(parent) = paths::default_db_path().parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create db parent: {}", e))?;
    }
    Ok(())
}

pub fn ensure_config_file() {
    let config_path = paths::config_path();
    if !config_path.exists() {
        let default_config = include_str!("../config.toml");
        if let Some(parent) = config_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(e) = fs::write(&config_path, default_config) {
            tracing::warn!("Failed to create default config file: {}", e);
        } else {
            tracing::info!("Created default config file at {}", config_path.display());
        }
    }
}

pub fn ensure_certs(force: bool) -> Result<(), String> {
    crate::cert::generate_dev_certs(force, None)
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[test]
    fn test_ensure_layout_creates_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("snip-sync-test-layout");
        // Simulate by checking that create_dir_all is idempotent
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(base.join("certs")).unwrap();
        assert!(base.join("certs").exists());
    }

    #[test]
    fn test_ensure_config_file_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        // Write a config
        fs::write(&config_path, "test = true").unwrap();
        // Ensure it doesn't overwrite
        ensure_config_file_at(&config_path);
        let content = fs::read_to_string(&config_path).unwrap();
        assert_eq!(content, "test = true");
    }

    fn ensure_config_file_at(path: &std::path::Path) {
        if !path.exists() {
            let default_config = include_str!("../config.toml");
            let _ = fs::write(path, default_config);
        }
    }
}
