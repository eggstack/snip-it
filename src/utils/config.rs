use std::path::PathBuf;

pub fn get_config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::config_dir().unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".config")
            })
        })
        .join("snp")
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
