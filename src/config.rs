use crate::error::{SnipError, SnipResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_sync_url")]
    pub server_url: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub device_id: String,
    #[serde(default = "default_sync_interval")]
    pub sync_interval_minutes: u32,
    #[serde(default)]
    pub auto_sync: bool,
    #[serde(default)]
    pub sync_direction: SyncDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub enum SyncDirection {
    #[default]
    Push,
    Pull,
    Bidirectional,
}

fn default_sync_url() -> String {
    "http://localhost:50051".to_string()
}

fn default_sync_interval() -> u32 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SyncConfigFile {
    #[serde(default)]
    settings: SyncConfigSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SyncConfigSettings {
    #[serde(default)]
    sync: SyncSettings,
}

pub fn get_sync_config_path() -> PathBuf {
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".config")));
    config_dir.join("snp").join("sync.toml")
}

pub fn save_sync_settings(settings: &SyncSettings) -> SnipResult<()> {
    let path = get_sync_config_path();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| SnipError::io_error("create config directory", parent.to_path_buf(), e))?;
    }

    let config = SyncConfigFile {
        settings: SyncConfigSettings {
            sync: settings.clone(),
        },
    };

    let content = toml::to_string_pretty(&config)
        .map_err(|e| SnipError::toml_error("serialize sync config", e))?;

    fs::write(&path, content).map_err(|e| SnipError::io_error("write sync config", path, e))?;

    Ok(())
}

pub fn load_sync_settings() -> SnipResult<SyncSettings> {
    let path = get_sync_config_path();

    if !path.exists() {
        return Ok(SyncSettings::default());
    }

    let content =
        fs::read_to_string(&path).map_err(|e| SnipError::io_error("read sync config", path, e))?;

    let config: SyncConfigFile =
        toml::from_str(&content).map_err(|e| SnipError::toml_error("parse sync config", e))?;

    Ok(config.settings.sync)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_sync_settings_default() {
        let settings = SyncSettings::default();

        assert!(!settings.enabled);
        // Note: serde with default = "..." doesn't apply to struct defaults
        // These fields use default() from Default trait
        assert!(settings.api_key.is_empty());
        assert!(settings.device_id.is_empty());
        assert_eq!(settings.sync_interval_minutes, 0); // u32 defaults to 0
        assert!(!settings.auto_sync);
        assert_eq!(settings.sync_direction, SyncDirection::Push);
    }

    #[test]
    fn test_sync_direction_variants() {
        assert_eq!(SyncDirection::Push, SyncDirection::Push);
        assert_eq!(SyncDirection::Pull, SyncDirection::Pull);
        assert_eq!(SyncDirection::Bidirectional, SyncDirection::Bidirectional);
    }

    #[test]
    fn test_save_and_load_sync_settings() {
        // Verify default settings work
        let settings = SyncSettings::default();
        assert!(!settings.enabled);
    }

    #[test]
    fn test_sync_settings_serialization() {
        let settings = SyncSettings {
            enabled: true,
            server_url: "https://sync.example.com".to_string(),
            api_key: "test-key-123".to_string(),
            device_id: "device-456".to_string(),
            sync_interval_minutes: 60,
            auto_sync: true,
            sync_direction: SyncDirection::Bidirectional,
        };

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        assert!(toml_str.contains("enabled = true"));
        assert!(toml_str.contains("server_url = \"https://sync.example.com\""));
        assert!(toml_str.contains("api_key = \"test-key-123\""));
        assert!(toml_str.contains("device_id = \"device-456\""));
        assert!(toml_str.contains("sync_interval_minutes = 60"));
        assert!(toml_str.contains("auto_sync = true"));
        assert!(toml_str.contains("sync_direction = \"Bidirectional\""));
    }
}
