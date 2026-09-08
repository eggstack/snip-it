//! **Layer: Sync-Client** (with platform dependency: keyring)
//!
//! Sync configuration: [`SyncSettings`], [`SyncDirection`],
//! [`AutoSyncFailureMode`], keychain-backed API-key storage, and
//! save/load/get helpers. Settings are stored in `sync.toml`.
//!
//! **Known cross-layer dependency:** `save_sync_settings()` calls
//! `crate::clipboard::invalidate_clipboard_settings_cache()` — this should
//! be moved to the caller or an event bus in a future refactor.

use super::toml_cache::{
    cached_read_toml, compute_crc32, invalidate_toml_cache, strip_integrity_line, verify_integrity,
};
use crate::error::{SnipError, SnipResult};
use crate::utils::config::get_sync_config_path;
use crate::utils::toml_helpers::fix_invalid_toml_escapes;
use serde::{Deserialize, Serialize};
use std::fs;

const KEYCHAIN_SERVICE: &str = "snp-sync";
pub(crate) const KEYCHAIN_MARKER: &str = "@keychain";
const KEYCHAIN_DEFAULT_USER: &str = "api-key";

pub const DEFAULT_SERVER_URL: &str = "http://localhost:50051";

/// Minimum accepted value for `auto_sync_debounce_seconds`.
pub const AUTO_SYNC_DEBOUNCE_MIN: u64 = 0;
/// Maximum accepted value for `auto_sync_debounce_seconds`.
pub const AUTO_SYNC_DEBOUNCE_MAX: u64 = 300;
/// Maximum accepted value for `auto_sync_max_delay_seconds`.
pub const AUTO_SYNC_MAX_DELAY_MIN: u64 = 0;
/// Maximum accepted value for `auto_sync_max_delay_seconds`.
pub const AUTO_SYNC_MAX_DELAY_MAX: u64 = 600;
/// Default auto-sync network operation timeout in seconds.
pub const DEFAULT_SYNC_TIMEOUT_SECS: u64 = 30;
/// Minimum accepted value for `auto_sync_timeout_seconds`.
pub const MIN_SYNC_TIMEOUT_SECS: u64 = 5;
/// Maximum accepted value for `auto_sync_timeout_seconds`.
pub const MAX_SYNC_TIMEOUT_SECS: u64 = 120;

/// Failure behavior for post-mutation auto-sync.
///
/// Controls whether a failed auto-sync emits a warning or a hard error.
/// The `error` policy never implies rollback — the local mutation always
/// remains committed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
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
    /// Maximum latency (in seconds) before an auto-sync is forced regardless
    /// of debounce state. Clamped to [`AUTO_SYNC_MAX_DELAY_MIN`]..[`AUTO_SYNC_MAX_DELAY_MAX`].
    #[serde(default)]
    pub auto_sync_max_delay_seconds: Option<u64>,
    /// Executor sync timeout in seconds. Independent of debounce.
    /// Clamped to [`MIN_SYNC_TIMEOUT_SECS`]..[`MAX_SYNC_TIMEOUT_SECS`].
    #[serde(default)]
    pub auto_sync_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub sync_direction: SyncDirection,
    #[serde(default)]
    pub clipboard_auto_clear_seconds: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_positive_sync_limit")]
    pub sync_limit: Option<i32>,
    /// Monotonically increasing counter incremented whenever `api_key` changes.
    /// Used by the config fingerprint to detect credential replacement without
    /// persisting the key value itself.
    #[serde(default)]
    pub credential_revision: u64,
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
            .field(
                "auto_sync_max_delay_seconds",
                &self.auto_sync_max_delay_seconds,
            )
            .field("auto_sync_timeout_seconds", &self.auto_sync_timeout_seconds)
            .field("sync_direction", &self.sync_direction)
            .field(
                "clipboard_auto_clear_seconds",
                &self.clipboard_auto_clear_seconds,
            )
            .field("sync_limit", &self.sync_limit)
            .field("credential_revision", &self.credential_revision)
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
            auto_sync_max_delay_seconds: self.auto_sync_max_delay_seconds,
            auto_sync_timeout_seconds: self.auto_sync_timeout_seconds,
            sync_direction: self.sync_direction.clone(),
            clipboard_auto_clear_seconds: self.clipboard_auto_clear_seconds,
            sync_limit: self.sync_limit,
            credential_revision: self.credential_revision,
        }
    }
}

impl SyncSettings {
    /// Returns the sync limit value, defaulting to 1000 if it is not set.
    /// Non-positive values are rejected when settings are parsed or saved;
    /// the defensive fallback remains for manually constructed values.
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

    /// Returns the effective auto-sync max delay duration, clamped to
    /// [`AUTO_SYNC_MAX_DELAY_MIN`]..[`AUTO_SYNC_MAX_DELAY_MAX`].
    pub fn auto_sync_max_delay(&self) -> std::time::Duration {
        let secs = self
            .auto_sync_max_delay_seconds
            .unwrap_or(300)
            .clamp(AUTO_SYNC_MAX_DELAY_MIN, AUTO_SYNC_MAX_DELAY_MAX);
        std::time::Duration::from_secs(secs)
    }

    /// Returns the configured auto-sync timeout value, clamped to
    /// [`MIN_SYNC_TIMEOUT_SECS`]..[`MAX_SYNC_TIMEOUT_SECS`].
    /// Defaults to [`DEFAULT_SYNC_TIMEOUT_SECS`] when not configured.
    pub fn auto_sync_timeout(&self) -> std::time::Duration {
        let secs = self
            .auto_sync_timeout_seconds
            .unwrap_or(DEFAULT_SYNC_TIMEOUT_SECS)
            .clamp(MIN_SYNC_TIMEOUT_SECS, MAX_SYNC_TIMEOUT_SECS);
        std::time::Duration::from_secs(secs)
    }

    /// Returns true if the sync config file exists on disk.
    pub fn sync_config_file_exists() -> bool {
        get_sync_config_path().exists()
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
    // Test-only credential file: skip keychain, write plaintext directly.
    // This ensures the credential file and sync.toml stay in sync.
    #[cfg(feature = "test-support")]
    if std::env::var_os("SNP_TEST_CREDENTIAL_FILE").is_some() {
        return serializer.serialize_str(api_key);
    }
    // Plaintext mode is a test-only seam. Production builds must always use
    // the OS keychain or fail rather than silently persisting credentials.
    #[cfg(feature = "test-support")]
    if std::env::var_os("SNP_ALLOW_PLAINTEXT_API_KEY").is_some_and(|v| v == "true") {
        return serializer.serialize_str(api_key);
    }
    // Server URL is not available during serialization, so we use the default user
    match keychain_store(api_key, KEYCHAIN_DEFAULT_USER) {
        Ok(()) => serializer.serialize_str(KEYCHAIN_MARKER),
        Err(e) => {
            tracing::error!("Keychain unavailable, refusing to store API key in plaintext.");
            Err(serde::ser::Error::custom(format!(
                "keychain unavailable: {e}; refusing plaintext API-key storage"
            )))
        }
    }
}

fn deserialize_api_key<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<String, D::Error> {
    let raw: String = Deserialize::deserialize(deserializer)?;
    if raw == KEYCHAIN_MARKER {
        // Test-only credential file: read the actual key from the file.
        // This bypasses the keychain entirely, ensuring deterministic
        // credential availability for parent, worker, and executor.
        #[cfg(feature = "test-support")]
        if let Some(cred_path) = std::env::var_os("SNP_TEST_CREDENTIAL_FILE") {
            match std::fs::read_to_string(&cred_path) {
                Ok(key) => {
                    let key = key.trim().to_string();
                    if !key.is_empty() {
                        return Ok(key);
                    }
                    tracing::warn!(
                        "SNP_TEST_CREDENTIAL_FILE exists but is empty: {}",
                        cred_path.display()
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to read SNP_TEST_CREDENTIAL_FILE {}: {}",
                        cred_path.display(),
                        e
                    );
                }
            }
        }
        #[cfg(feature = "test-support")]
        if std::env::var_os("SNP_ALLOW_PLAINTEXT_API_KEY").is_some_and(|v| v == "true") {
            // Fail fast: returning the literal marker would authenticate
            // every subsequent sync with a bogus credential. Refuse to load
            // instead so no request is sent with "@keychain" as the key.
            tracing::error!(
                "sync.toml stores API key as `@keychain` marker but plaintext mode is enabled; \
                 refusing to use the marker as a credential. \
                 Re-save sync settings (snp sync config) to store the key in plaintext."
            );
            return Err(serde::de::Error::custom(
                "api_key is stored as the `@keychain` keychain marker, but \
                 SNP_ALLOW_PLAINTEXT_API_KEY=true forbids keychain access; \
                 re-save sync settings to store the key in plaintext",
            ));
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

pub(crate) fn migrate_plaintext_api_key<FStore, FSave>(
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
    // Skip migration when using test credential file — the file is the
    // authoritative source and migrating to keychain would overwrite it.
    #[cfg(feature = "test-support")]
    if std::env::var_os("SNP_TEST_CREDENTIAL_FILE").is_some() {
        return;
    }
    // Plaintext mode is a test-only seam; production builds migrate plaintext
    // credentials to the OS keychain.
    #[cfg(feature = "test-support")]
    if std::env::var_os("SNP_ALLOW_PLAINTEXT_API_KEY").is_some_and(|v| v == "true") {
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
            auto_sync_max_delay_seconds: None,
            auto_sync_timeout_seconds: None,
            sync_direction: SyncDirection::default(),
            clipboard_auto_clear_seconds: None,
            sync_limit: None,
            credential_revision: 0,
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

fn deserialize_positive_sync_limit<'de, D>(deserializer: D) -> Result<Option<i32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<i32>::deserialize(deserializer)?;
    match value {
        Some(limit) if limit <= 0 => Err(serde::de::Error::custom(
            "sync_limit must be greater than zero",
        )),
        _ => Ok(value),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct SyncConfigFile {
    #[serde(default)]
    pub(crate) settings: SyncConfigSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct SyncConfigSettings {
    #[serde(default)]
    pub(crate) sync: SyncSettings,
}

pub fn save_sync_settings(settings: &SyncSettings) -> SnipResult<()> {
    if settings.sync_limit.is_some_and(|limit| limit <= 0) {
        return Err(SnipError::runtime_error(
            "Invalid sync limit",
            Some("sync_limit must be greater than zero"),
        ));
    }

    let state_dir = crate::local_data::transaction_dir();
    let _local_lock = crate::local_data::acquire_local_data_lock(&state_dir)?;

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

    crate::test_failpoints::mutation_barrier("sync-config-update-before-cache-invalidate");

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
            eprintln!(
                "warning: {} failed its integrity check and may be corrupted; \
                 sync settings were reset to defaults (backup also failed: {backup_err}). \
                 Run 'snp sync config' to reconfigure.",
                path.display()
            );
        } else {
            tracing::info!(
                "Backed up corrupted sync config to {}",
                backup_path.display()
            );
            eprintln!(
                "warning: {} failed its integrity check and may be corrupted; \
                 it was backed up to {} and sync settings were reset to defaults. \
                 Run 'snp sync config' to reconfigure.",
                path.display(),
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
