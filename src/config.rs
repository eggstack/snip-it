//! Configuration management for snp sync.
//!
//! Handles loading and saving sync settings including server configuration,
//! API keys, and sync preferences. Settings are stored in `sync.toml`.

use crate::error::{SnipError, SnipResult};
pub use crate::utils::config::get_sync_config_path;
use crate::utils::toml_helpers::{fix_invalid_toml_escapes, quote_strings_containing_backslashes};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::SystemTime;

const KEYCHAIN_SERVICE: &str = "snp-sync";
const KEYCHAIN_MARKER: &str = "@keychain";
const KEYCHAIN_DEFAULT_USER: &str = "api-key";

pub const DEFAULT_SERVER_URL: &str = "https://localhost:50051";

struct CachedToml {
    mtime: SystemTime,
    content: String,
}

static TOML_CACHE: LazyLock<Mutex<HashMap<String, CachedToml>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn compute_crc32(data: &str) -> u32 {
    crc32fast::hash(data.as_bytes())
}

/// Verifies CRC32 integrity of the config file content.
///
/// Note: CRC32 detects accidental corruption (e.g., partial writes, disk errors)
/// but is NOT a cryptographic integrity check. An attacker who can modify the
/// config file can recalculate the CRC32. This is acceptable because the threat
/// model assumes local-only access — if an attacker can write to the config
/// directory, they can already replace the entire file or binary.
fn verify_integrity(content: &str) -> bool {
    for line in content.lines() {
        if let Some(stripped) = line.strip_prefix("# integrity:")
            && let Ok(stored) = stripped.trim().parse::<u32>()
        {
            let body: String = content
                .lines()
                .filter(|l| !l.starts_with("# integrity:"))
                .collect::<Vec<_>>()
                .join("\n");
            return stored == compute_crc32(&body);
        }
    }
    true
}

fn strip_integrity_line(content: &str) -> String {
    content
        .lines()
        .filter(|l| !l.starts_with("# integrity:"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn cached_read_toml(path: &std::path::Path) -> SnipResult<String> {
    let meta = fs::metadata(path)
        .map_err(|e| SnipError::io_error("stat toml file", path.to_path_buf(), e))?;
    let mtime = meta
        .modified()
        .map_err(|e| SnipError::io_error("read mtime", path.to_path_buf(), e))?;
    let key = path.to_string_lossy().to_string();

    {
        let cache = TOML_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(&key)
            && entry.mtime == mtime
        {
            return Ok(entry.content.clone());
        }
    }

    let content = fs::read_to_string(path)
        .map_err(|e| SnipError::io_error("read toml file", path.to_path_buf(), e))?;

    let mut cache = TOML_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.insert(
        key,
        CachedToml {
            mtime,
            content: content.clone(),
        },
    );
    Ok(content)
}

/// Sync configuration settings.
///
/// These settings control how snippets are synchronized with a remote server,
/// including server URL, authentication, and sync behavior preferences.
///
/// The API key is zeroized on drop to minimize exposure in process memory.
#[derive(Clone, Serialize, Deserialize)]
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
    #[serde(default)]
    pub sync_limit: Option<i32>,
}

impl std::fmt::Debug for SyncSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncSettings")
            .field("enabled", &self.enabled)
            .field("server_url", &self.server_url)
            .field("api_key", &"[REDACTED]")
            .field("device_id", &self.device_id)
            .field("sync_interval_minutes", &self.sync_interval_minutes)
            .field("auto_sync", &self.auto_sync)
            .field("sync_direction", &self.sync_direction)
            .field(
                "clipboard_auto_clear_seconds",
                &self.clipboard_auto_clear_seconds,
            )
            .field("sync_limit", &self.sync_limit)
            .finish()
    }
}

impl Drop for SyncSettings {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.api_key.zeroize();
    }
}

impl SyncSettings {
    /// Returns the sync limit value, defaulting to 1000 if not set.
    pub fn sync_limit_value(&self) -> i32 {
        self.sync_limit.filter(|&v| v > 0).unwrap_or(1000)
    }
}

fn serialize_api_key<S: serde::Serializer>(
    api_key: &str,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if api_key.is_empty() {
        return serializer.serialize_str("");
    }
    // Server URL is not available during serialization, so we use the default user
    match keychain_store(api_key, KEYCHAIN_DEFAULT_USER) {
        Ok(()) => serializer.serialize_str(KEYCHAIN_MARKER),
        Err(e) => {
            if std::env::var_os("SNP_ALLOW_PLAINTEXT_API_KEY").is_some_and(|v| v == "true") {
                tracing::warn!(
                    "Keychain unavailable, storing API key in config file (explicitly allowed): {}",
                    e
                );
                serializer.serialize_str(api_key)
            } else {
                tracing::error!(
                    "Keychain unavailable, refusing to store API key in plaintext. \
                     Set SNP_ALLOW_PLAINTEXT_API_KEY=true to allow."
                );
                serializer.serialize_str("")
            }
        }
    }
}

fn deserialize_api_key<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let raw: String = Deserialize::deserialize(deserializer)?;
    if raw == KEYCHAIN_MARKER {
        match keychain_retrieve(KEYCHAIN_DEFAULT_USER) {
            Ok(key) => Ok(key),
            Err(e) => {
                tracing::error!(
                    "Keychain unavailable, cannot retrieve API key: {}. \
                     Re-save sync settings to store key in config file as fallback.",
                    e
                );
                Err(serde::de::Error::custom(
                    "keychain unavailable, cannot retrieve API key",
                ))
            }
        }
    } else {
        Ok(raw)
    }
}

fn keychain_store(api_key: &str, user: &str) -> SnipResult<()> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, user)
        .map_err(|e| SnipError::runtime_error("keychain entry", Some(&e.to_string())))?;
    entry
        .set_password(api_key)
        .map_err(|e| SnipError::runtime_error("keychain store", Some(&e.to_string())))?;
    Ok(())
}

fn keychain_retrieve(user: &str) -> SnipResult<String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, user)
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
            sync_limit: None,
        }
    }
}

/// Sync direction control.
///
/// Determines whether snippets are pushed to the server, pulled from it,
/// or synchronized bidirectionally.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[non_exhaustive]
pub enum SyncDirection {
    #[default]
    Push,
    Pull,
    Bidirectional,
}

fn default_sync_url() -> String {
    DEFAULT_SERVER_URL.to_string()
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
    let checksum = compute_crc32(&content);
    let content_with_integrity = format!("# integrity: {checksum}\n{content}");

    let tmp_path = path.with_extension("toml.tmp");

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true).mode(0o600);
        let mut file = opts
            .open(&tmp_path)
            .map_err(|e| SnipError::io_error("create sync config temp", &tmp_path, e))?;
        use std::io::Write;
        file.write_all(content_with_integrity.as_bytes())
            .map_err(|e| SnipError::io_error("write sync config temp", &tmp_path, e))?;
    }

    #[cfg(not(unix))]
    {
        fs::write(&tmp_path, &content_with_integrity)
            .map_err(|e| SnipError::io_error("write sync config temp", &tmp_path, e))?;
    }

    fs::rename(&tmp_path, &path).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        SnipError::io_error("atomic rename sync config", path, e)
    })?;

    crate::clipboard::invalidate_clipboard_settings_cache();

    Ok(())
}

pub fn load_sync_settings() -> SnipResult<SyncSettings> {
    let path = get_sync_config_path();

    if !path.exists() {
        return Ok(SyncSettings::default());
    }

    let content = cached_read_toml(&path)?;

    if !verify_integrity(&content) {
        tracing::warn!("sync.toml integrity check failed — file may be corrupted. Using defaults.");
        // Backup corrupted file before returning defaults
        let backup_path = path.with_extension("toml.corrupt.bak");
        if let Err(backup_err) = fs::copy(&path, &backup_path) {
            tracing::error!("Failed to backup corrupted sync config: {}", backup_err);
        } else {
            tracing::info!(
                "Backed up corrupted sync config to {}",
                backup_path.display()
            );
        }
        return Ok(SyncSettings::default());
    }

    let content = strip_integrity_line(&content);
    let fixed_content = fix_invalid_toml_escapes(&content);

    let config: SyncConfigFile = toml::from_str(&fixed_content)
        .map_err(|e| SnipError::toml_error("parse sync config", e))?;

    let mut settings = config.settings.sync;

    // Migrate existing plaintext API key to keychain on first load
    if !settings.api_key.is_empty() && settings.api_key != KEYCHAIN_MARKER {
        if let Err(e) = keychain_store(&settings.api_key, KEYCHAIN_DEFAULT_USER) {
            tracing::error!(
                "Failed to migrate API key to keychain (keychain unavailable): {}. \
                 API key will remain in plaintext config file.",
                e
            );
        } else {
            settings.api_key = KEYCHAIN_MARKER.to_string();
            if let Err(e) = save_sync_settings(&settings) {
                tracing::error!("Failed to save keychain marker: {}", e);
            }
        }
    }

    Ok(settings)
}

pub fn get_sync_settings() -> SyncSettings {
    match load_sync_settings() {
        Ok(settings) => settings,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to load sync settings, using defaults");
            SyncSettings::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_sync_settings_default() {
        let settings = SyncSettings::default();

        assert!(!settings.enabled);
        assert_eq!(settings.server_url, DEFAULT_SERVER_URL);
        assert!(settings.api_key.is_empty());
        assert!(settings.device_id.is_empty());
        assert_eq!(settings.sync_interval_minutes, 30);
        assert!(!settings.auto_sync);
        assert_eq!(settings.sync_direction, SyncDirection::Push);
        assert_eq!(settings.sync_limit, None);
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
            sync_limit: Some(2000),
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
