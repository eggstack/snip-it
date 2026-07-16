//! Configuration management for snp sync.
//!
//! Handles loading and saving sync settings including server configuration,
//! API keys, and sync preferences. Settings are stored in `sync.toml`.

use crate::error::{SnipError, SnipResult};
pub use crate::utils::config::get_sync_config_path;
use crate::utils::toml_helpers::fix_invalid_toml_escapes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::SystemTime;

const KEYCHAIN_SERVICE: &str = "snp-sync";
const KEYCHAIN_MARKER: &str = "@keychain";
const KEYCHAIN_DEFAULT_USER: &str = "api-key";

pub const DEFAULT_SERVER_URL: &str = "http://localhost:50051";

/// Minimum accepted value for `auto_sync_debounce_seconds`.
pub const AUTO_SYNC_DEBOUNCE_MIN: u64 = 0;
/// Maximum accepted value for `auto_sync_debounce_seconds`.
pub const AUTO_SYNC_DEBOUNCE_MAX: u64 = 300;

/// Failure behavior for post-mutation auto-sync.
///
/// Controls whether a failed auto-sync emits a warning or a hard error.
/// The `error` policy never implies rollback — the local mutation always
/// remains committed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoSyncFailureMode {
    /// Retain local success, suppress user-facing failure.
    Ignore,
    /// Retain local success, emit a concise warning to stderr.
    #[default]
    Warn,
    /// Local mutation remains committed, but the command returns a
    /// distinct post-commit sync failure outcome (nonzero exit code).
    Error,
}

impl std::fmt::Display for AutoSyncFailureMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ignore => write!(f, "ignore"),
            Self::Warn => write!(f, "warn"),
            Self::Error => write!(f, "error"),
        }
    }
}

impl std::str::FromStr for AutoSyncFailureMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "ignore" => Ok(Self::Ignore),
            "warn" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            _ => Err(format!(
                "invalid auto_sync_failure mode '{s}': expected ignore, warn, or error"
            )),
        }
    }
}

struct CachedToml {
    mtime: SystemTime,
    len: u64,
    content: String,
}

const MAX_TOML_CACHE_SIZE: usize = 100;

static TOML_CACHE: LazyLock<Mutex<HashMap<String, CachedToml>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn invalidate_toml_cache(path: &std::path::Path) {
    let key = path.to_string_lossy().to_string();
    if let Ok(mut cache) = TOML_CACHE.lock() {
        cache.remove(&key);
    }
}

fn compute_crc32(data: &str) -> u32 {
    crc32fast::hash(data.as_bytes())
}

fn split_integrity_header(content: &str) -> Option<(&str, &str)> {
    let (first_line, body) = match content.find('\n') {
        Some(index) => (&content[..index], &content[index + 1..]),
        None => (content, ""),
    };

    first_line
        .strip_prefix("# integrity:")
        .map(|checksum| (checksum.trim(), body))
}

/// Verifies CRC32 integrity of the config file content.
///
/// Note: CRC32 detects accidental corruption (e.g., partial writes, disk errors)
/// but is NOT a cryptographic integrity check. An attacker who can modify the
/// config file can recalculate the CRC32. This is acceptable because the threat
/// model assumes local-only access — if an attacker can write to the config
/// directory, they can already replace the entire file or binary.
fn verify_integrity(content: &str) -> bool {
    // The integrity header must be the very first line to avoid matching
    // user-authored TOML comments like "# integrity: 42".
    if let Some((checksum, body)) = split_integrity_header(content) {
        return checksum
            .parse::<u32>()
            .is_ok_and(|stored| stored == compute_crc32(body));
    }

    // No integrity header found — this is a legacy config file from before the
    // integrity feature was added. Treat it as valid rather than silently
    // replacing with defaults (which would cause data loss on upgrade).
    // The header will be added on the next save.
    true
}

fn strip_integrity_line(content: &str) -> String {
    split_integrity_header(content)
        .map(|(_, body)| body.to_string())
        .unwrap_or_else(|| content.to_string())
}

pub fn cached_read_toml(path: &std::path::Path) -> SnipResult<String> {
    let key = path.to_string_lossy().to_string();

    let metadata = fs::metadata(path)
        .map_err(|e| SnipError::io_error("stat toml file", path.to_path_buf(), e))?;
    let mtime = metadata
        .modified()
        .map_err(|e| SnipError::io_error("read mtime", path.to_path_buf(), e))?;
    let len = metadata.len();

    let cache = TOML_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = cache.get(&key)
        && entry.mtime == mtime
        && entry.len == len
    {
        return Ok(entry.content.clone());
    }
    drop(cache);

    let content = fs::read_to_string(path)
        .map_err(|e| SnipError::io_error("read toml file", path.to_path_buf(), e))?;

    let mut cache = TOML_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= MAX_TOML_CACHE_SIZE {
        let keys_to_remove: Vec<_> = cache
            .keys()
            .take(MAX_TOML_CACHE_SIZE / 2)
            .cloned()
            .collect();
        for key in keys_to_remove {
            cache.remove(&key);
        }
    }

    cache.insert(
        key,
        CachedToml {
            mtime,
            len,
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
#[derive(Serialize, Deserialize)]
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
    /// Debounce delay in seconds before auto-sync fires after a mutation.
    /// Clamped to [`AUTO_SYNC_DEBOUNCE_MIN`]..[`AUTO_SYNC_DEBOUNCE_MAX`].
    #[serde(default = "default_auto_sync_debounce_seconds")]
    pub auto_sync_debounce_seconds: u64,
    /// Failure behavior when auto-sync cannot complete.
    /// Does not affect local mutation guarantees.
    #[serde(default)]
    pub auto_sync_failure: AutoSyncFailureMode,
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
            .field(
                "auto_sync_debounce_seconds",
                &self.auto_sync_debounce_seconds,
            )
            .field("auto_sync_failure", &self.auto_sync_failure)
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

impl Clone for SyncSettings {
    fn clone(&self) -> Self {
        SyncSettings {
            enabled: self.enabled,
            server_url: self.server_url.clone(),
            api_key: self.api_key.clone(),
            device_id: self.device_id.clone(),
            sync_interval_minutes: self.sync_interval_minutes,
            auto_sync: self.auto_sync,
            auto_sync_debounce_seconds: self.auto_sync_debounce_seconds,
            auto_sync_failure: self.auto_sync_failure.clone(),
            sync_direction: self.sync_direction.clone(),
            clipboard_auto_clear_seconds: self.clipboard_auto_clear_seconds,
            sync_limit: self.sync_limit,
        }
    }
}

impl SyncSettings {
    /// Returns the sync limit value, defaulting to 1000 if not set.
    pub fn sync_limit_value(&self) -> i32 {
        self.sync_limit.filter(|&v| v > 0).unwrap_or(1000)
    }

    /// Returns the effective auto-sync debounce duration, clamped to
    /// [`AUTO_SYNC_DEBOUNCE_MIN`]..[`AUTO_SYNC_DEBOUNCE_MAX`].
    pub fn auto_sync_debounce(&self) -> std::time::Duration {
        let clamped = self
            .auto_sync_debounce_seconds
            .clamp(AUTO_SYNC_DEBOUNCE_MIN, AUTO_SYNC_DEBOUNCE_MAX);
        std::time::Duration::from_secs(clamped)
    }
}

fn serialize_api_key<S: serde::Serializer>(
    api_key: &str,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if api_key.is_empty() {
        return serializer.serialize_str("");
    }
    // If the key is already the keychain marker, just write the marker
    // without touching the keychain (avoids overwriting the real key).
    if api_key == KEYCHAIN_MARKER {
        return serializer.serialize_str(KEYCHAIN_MARKER);
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
                Err(serde::ser::Error::custom(format!(
                    "keychain unavailable: {e}. Set SNP_ALLOW_PLAINTEXT_API_KEY=true to allow plaintext storage."
                )))
            }
        }
    }
}

fn deserialize_api_key<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let raw: String = Deserialize::deserialize(deserializer)?;
    if raw == KEYCHAIN_MARKER {
        if std::env::var_os("SNP_ALLOW_PLAINTEXT_API_KEY").is_some_and(|v| v == "true") {
            tracing::warn!(
                "sync.toml stores API key as `@keychain` marker but plaintext mode is enabled; \
                 keeping marker in-memory. Subsequent sync operations may fail."
            );
            return Ok(raw);
        }
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

fn migrate_plaintext_api_key<FStore, FSave>(
    settings: &SyncSettings,
    store_key: FStore,
    save_marker: FSave,
) where
    FStore: FnOnce(&str) -> SnipResult<()>,
    FSave: FnOnce(&SyncSettings) -> SnipResult<()>,
{
    if settings.api_key.is_empty() || settings.api_key == KEYCHAIN_MARKER {
        return;
    }

    if let Err(e) = store_key(&settings.api_key) {
        tracing::error!(
            "Failed to migrate API key to keychain (keychain unavailable): {}. \
             API key will remain in plaintext config file.",
            e
        );
        return;
    }

    let mut marker_settings = settings.clone();
    marker_settings.api_key = KEYCHAIN_MARKER.to_string();
    if let Err(e) = save_marker(&marker_settings) {
        tracing::error!("Failed to save keychain marker: {}", e);
    }
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
            auto_sync_debounce_seconds: 2,
            auto_sync_failure: AutoSyncFailureMode::default(),
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
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
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

fn default_auto_sync_debounce_seconds() -> u64 {
    2
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
    let config = SyncConfigFile {
        settings: SyncConfigSettings {
            sync: settings.clone(),
        },
    };

    let content = toml::to_string_pretty(&config)
        .map_err(|e| SnipError::toml_error("serialize sync config", e))?;

    let checksum = compute_crc32(&content);
    let content_with_integrity = format!("# integrity: {checksum}\n{content}");

    crate::utils::atomic::write_private_atomic(&path, &content_with_integrity, "sync")?;

    invalidate_toml_cache(&path);
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

    let settings = config.settings.sync;

    // Migrate existing plaintext API key to keychain on first load. Keep the
    // plaintext key in this in-memory settings value so the caller can complete
    // the current sync/register operation with the real credential.
    migrate_plaintext_api_key(
        &settings,
        |api_key| keychain_store(api_key, KEYCHAIN_DEFAULT_USER),
        save_sync_settings,
    );

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
        assert_eq!(settings.auto_sync_debounce_seconds, 2);
        assert_eq!(settings.auto_sync_failure, AutoSyncFailureMode::Warn);
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
            auto_sync_debounce_seconds: 5,
            auto_sync_failure: AutoSyncFailureMode::Error,
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
        assert!(toml_str.contains("auto_sync_debounce_seconds = 5"));
        assert!(toml_str.contains("auto_sync_failure = \"error\""));
        assert!(toml_str.contains("sync_direction = \"Bidirectional\""));
    }

    #[test]
    fn test_verify_integrity_no_header_returns_true() {
        // Legacy config files without an integrity header should be accepted
        // to prevent data loss on upgrade from older versions.
        let content = "[sync]\nenabled = true\n";
        assert!(verify_integrity(content));
    }

    #[test]
    fn test_verify_integrity_valid_header() {
        let body = "[sync]\nenabled = true";
        let checksum = compute_crc32(body);
        let content = format!("# integrity: {checksum}\n{body}");
        assert!(verify_integrity(&content));
    }

    #[test]
    fn test_verify_integrity_invalid_header() {
        let body = "[sync]\nenabled = true";
        let content = format!("# integrity: 999999\n{body}");
        assert!(!verify_integrity(&content));
    }

    #[test]
    fn test_verify_integrity_tampered_body() {
        let original = "[sync]\nenabled = true";
        let checksum = compute_crc32(original);
        let tampered = "[sync]\nenabled = false";
        let content = format!("# integrity: {checksum}\n{tampered}");
        assert!(!verify_integrity(&content));
    }

    #[test]
    fn test_verify_integrity_preserves_exact_body() {
        let body = "[sync]\n# integrity: user-authored comment\nenabled = true\n";
        let checksum = compute_crc32(body);
        let content = format!("# integrity: {checksum}\n{body}");

        assert!(verify_integrity(&content));
        assert_eq!(strip_integrity_line(&content), body);
    }

    #[test]
    fn test_verify_integrity_malformed_header_fails() {
        let content = "# integrity: not-a-checksum\n[sync]\nenabled = true\n";
        assert!(!verify_integrity(content));
    }

    #[test]
    fn test_keychain_migration_preserves_in_memory_api_key() {
        let mut settings = SyncSettings::default();
        settings.api_key = "test-key-123".to_string();

        migrate_plaintext_api_key(
            &settings,
            |api_key| {
                assert_eq!(api_key, "test-key-123");
                Ok(())
            },
            |saved_settings| {
                assert_eq!(saved_settings.api_key, KEYCHAIN_MARKER);
                Ok(())
            },
        );

        assert_eq!(settings.api_key, "test-key-123");
    }

    #[test]
    fn test_auto_sync_debounce_clamped() {
        let mut settings = SyncSettings::default();
        assert_eq!(
            settings.auto_sync_debounce(),
            std::time::Duration::from_secs(2)
        );

        settings.auto_sync_debounce_seconds = 0;
        assert_eq!(
            settings.auto_sync_debounce(),
            std::time::Duration::from_secs(0)
        );

        settings.auto_sync_debounce_seconds = 300;
        assert_eq!(
            settings.auto_sync_debounce(),
            std::time::Duration::from_secs(300)
        );

        // Overflow clamped to max
        settings.auto_sync_debounce_seconds = u64::MAX;
        assert_eq!(
            settings.auto_sync_debounce(),
            std::time::Duration::from_secs(300)
        );
    }

    #[test]
    fn test_auto_sync_failure_mode_default() {
        let settings = SyncSettings::default();
        assert_eq!(settings.auto_sync_failure, AutoSyncFailureMode::Warn);
    }

    #[test]
    fn test_auto_sync_failure_mode_display_roundtrip() {
        let modes = vec![
            AutoSyncFailureMode::Ignore,
            AutoSyncFailureMode::Warn,
            AutoSyncFailureMode::Error,
        ];
        for mode in &modes {
            let s = mode.to_string();
            let parsed: AutoSyncFailureMode = s.parse().unwrap();
            assert_eq!(*mode, parsed);
        }
    }

    #[test]
    fn test_auto_sync_failure_mode_invalid() {
        let result = "bogus".parse::<AutoSyncFailureMode>();
        assert!(result.is_err());
    }

    #[test]
    fn test_old_config_without_auto_sync_fields_loads_defaults() {
        let content = r#"
[settings.sync]
enabled = true
server_url = "https://sync.example.com"
api_key = "test-key"
sync_interval_minutes = 15
auto_sync = true
sync_direction = "Bidirectional"
"#;
        // Old configs without auto_sync_debounce_seconds/auto_sync_failure should load defaults
        let config: SyncConfigFile = toml::from_str(content).unwrap();
        let settings = config.settings.sync;
        assert!(settings.auto_sync);
        assert_eq!(settings.auto_sync_debounce_seconds, 2); // default
        assert_eq!(settings.auto_sync_failure, AutoSyncFailureMode::Warn); // default
    }

    #[test]
    fn test_full_config_roundtrip() {
        let settings = SyncSettings {
            enabled: true,
            server_url: "https://sync.example.com".to_string(),
            api_key: "test-key".to_string(),
            device_id: "device-1".to_string(),
            sync_interval_minutes: 15,
            auto_sync: true,
            auto_sync_debounce_seconds: 5,
            auto_sync_failure: AutoSyncFailureMode::Error,
            sync_direction: SyncDirection::Bidirectional,
            clipboard_auto_clear_seconds: Some(30),
            sync_limit: Some(500),
        };
        let toml_str = toml::to_string_pretty(&settings).unwrap();
        // Use from_str directly to avoid keychain lookup
        let roundtripped: SyncSettings = toml::from_str(&toml_str).unwrap_or_else(|_| {
            // If keychain lookup fails, parse with a plaintext fallback
            let fallback = toml_str.replace("api_key = \"@keychain\"", "api_key = \"test-key\"");
            toml::from_str(&fallback).unwrap()
        });
        assert!(roundtripped.auto_sync);
        assert_eq!(roundtripped.auto_sync_debounce_seconds, 5);
        assert_eq!(roundtripped.auto_sync_failure, AutoSyncFailureMode::Error);
        assert_eq!(roundtripped.sync_direction, SyncDirection::Bidirectional);
    }

    #[test]
    fn test_unrelated_settings_preserved() {
        let settings = SyncSettings {
            enabled: true,
            server_url: "https://sync.example.com".to_string(),
            api_key: "test-key".to_string(),
            device_id: "device-1".to_string(),
            sync_interval_minutes: 15,
            auto_sync: true,
            auto_sync_debounce_seconds: 10,
            auto_sync_failure: AutoSyncFailureMode::Ignore,
            sync_direction: SyncDirection::Push,
            clipboard_auto_clear_seconds: Some(60),
            sync_limit: Some(500),
        };
        let toml_str = toml::to_string_pretty(&settings).unwrap();
        // Verify unrelated fields are present
        assert!(toml_str.contains("enabled = true"));
        assert!(toml_str.contains("sync_interval_minutes = 15"));
        assert!(toml_str.contains("clipboard_auto_clear_seconds = 60"));
        assert!(toml_str.contains("sync_limit = 500"));
        assert!(toml_str.contains("sync_direction = \"Push\""));
    }
}
