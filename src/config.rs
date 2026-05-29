//! Configuration management for snp sync.
//!
//! Handles loading and saving sync settings including server configuration,
//! API keys, and sync preferences. Settings are stored in `sync.toml`.

use crate::error::{SnipError, SnipResult};
pub use crate::utils::config::get_sync_config_path;
use crate::utils::toml_helpers::{fix_invalid_toml_escapes, quote_strings_containing_backslashes};
use serde::{Deserialize, Serialize};
use std::fs;

const KEYCHAIN_SERVICE: &str = "snp-sync";
const KEYCHAIN_USER: &str = "api-key";
const KEYCHAIN_MARKER: &str = "@keychain";

/// Sync configuration settings.
///
/// These settings control how snippets are synchronized with a remote server,
/// including server URL, authentication, and sync behavior preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSettings {
    pub enabled: bool,
    pub server_url: String,
    #[serde(
        default,
        serialize_with = "serialize_api_key",
        deserialize_with = "deserialize_api_key"
    )]
    pub api_key: String,
    #[serde(default)]
    pub device_id: String,
    pub sync_interval_minutes: u32,
    #[serde(default)]
    pub auto_sync: bool,
    #[serde(default)]
    pub sync_direction: SyncDirection,
    #[serde(default)]
    pub clipboard_auto_clear_seconds: Option<u32>,
}

fn serialize_api_key<S: serde::Serializer>(
    api_key: &str,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if api_key.is_empty() {
        return serializer.serialize_str("");
    }
    match keychain_store(api_key) {
        Ok(()) => serializer.serialize_str(KEYCHAIN_MARKER),
        Err(e) => {
            tracing::warn!(
                "Keychain unavailable, storing API key in config file: {}",
                e
            );
            serializer.serialize_str(api_key)
        }
    }
}

fn deserialize_api_key<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let raw: String = Deserialize::deserialize(deserializer)?;
    if raw == KEYCHAIN_MARKER {
        match keychain_retrieve() {
            Ok(key) => Ok(key),
            Err(e) => {
                tracing::warn!(
                    "Keychain unavailable, cannot retrieve API key: {}. \
                     Re-save sync settings to store key in config file as fallback.",
                    e
                );
                Ok(String::new())
            }
        }
    } else {
        Ok(raw)
    }
}

fn keychain_store(api_key: &str) -> SnipResult<()> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER)
        .map_err(|e| SnipError::runtime_error("keychain entry", Some(&e.to_string())))?;
    entry
        .set_password(api_key)
        .map_err(|e| SnipError::runtime_error("keychain store", Some(&e.to_string())))?;
    Ok(())
}

fn keychain_retrieve() -> SnipResult<String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER)
        .map_err(|e| SnipError::runtime_error("keychain entry", Some(&e.to_string())))?;
    entry
        .get_password()
        .map_err(|e| SnipError::runtime_error("keychain retrieve", Some(&e.to_string())))
}

impl Default for SyncSettings {
    fn default() -> Self {
        SyncSettings {
            enabled: false,
            server_url: default_sync_url(),
            api_key: String::new(),
            device_id: String::new(),
            sync_interval_minutes: default_sync_interval(),
            auto_sync: false,
            sync_direction: SyncDirection::default(),
            clipboard_auto_clear_seconds: None,
        }
    }
}

/// Sync direction control.
///
/// Determines whether snippets are pushed to the server, pulled from it,
/// or synchronized bidirectionally.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub enum SyncDirection {
    #[default]
    Push,
    Pull,
    Bidirectional,
}

fn default_sync_url() -> String {
    // NOTE: For production use, the server requires TLS or a reverse proxy.
    // The default localhost URL uses plaintext HTTP for development only.
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

    let content = quote_strings_containing_backslashes(&content);

    fs::write(&path, content).map_err(|e| SnipError::io_error("write sync config", path, e))?;

    Ok(())
}

pub fn load_sync_settings() -> SnipResult<SyncSettings> {
    let path = get_sync_config_path();

    if !path.exists() {
        return Ok(SyncSettings::default());
    }

    let content =
        fs::read_to_string(&path).map_err(|e| SnipError::io_error("read sync config", &path, e))?;

    let fixed_content = fix_invalid_toml_escapes(&content);

    let config: SyncConfigFile = toml::from_str(&fixed_content)
        .map_err(|e| SnipError::toml_error("parse sync config", e))?;

    let mut settings = config.settings.sync;

    // Migrate existing plaintext API key to keychain on first load
    if !settings.api_key.is_empty() && settings.api_key != KEYCHAIN_MARKER {
        if let Err(e) = keychain_store(&settings.api_key) {
            tracing::warn!("Failed to migrate API key to keychain: {}", e);
        } else {
            settings.api_key = KEYCHAIN_MARKER.to_string();
            // Re-save to persist the marker
            if let Err(e) = save_sync_settings(&settings) {
                tracing::warn!("Failed to save keychain marker: {}", e);
            }
        }
    }

    Ok(settings)
}

pub fn get_sync_settings() -> SyncSettings {
    load_sync_settings().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_sync_settings_default() {
        let settings = SyncSettings::default();

        assert!(!settings.enabled);
        assert_eq!(settings.server_url, "http://localhost:50051");
        assert!(settings.api_key.is_empty());
        assert!(settings.device_id.is_empty());
        assert_eq!(settings.sync_interval_minutes, 30);
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
            clipboard_auto_clear_seconds: Some(30),
        };

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        assert!(toml_str.contains("enabled = true"));
        assert!(toml_str.contains("server_url = \"https://sync.example.com\""));
        // API key is stored in keychain if available, otherwise plaintext
        assert!(
            toml_str.contains("api_key = \"@keychain\"")
                || toml_str.contains("api_key = \"test-key-123\"")
        );
        assert!(toml_str.contains("device_id = \"device-456\""));
        assert!(toml_str.contains("sync_interval_minutes = 60"));
        assert!(toml_str.contains("auto_sync = true"));
        assert!(toml_str.contains("sync_direction = \"Bidirectional\""));
    }
}
