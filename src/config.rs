//! **Layer: Sync-Client** (with platform dependency: keyring)
//!
//! Configuration management for snp sync.
//!
//! Handles loading and saving sync settings including server configuration,
//! API keys, and sync preferences. Settings are stored in `sync.toml`.
//!
//! **Known cross-layer dependency:** `save_sync_settings()` calls
//! `crate::clipboard::invalidate_clipboard_settings_cache()` — this should
//! be moved to the caller or an event bus in a future refactor.

use crate::error::{SnipError, SnipResult};
pub use crate::utils::config::derive_sync_state_dir;
pub use crate::utils::config::get_sync_config_path;
use crate::utils::toml_helpers::fix_invalid_toml_escapes;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::Read;
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

struct CachedToml {
    mtime: SystemTime,
    len: u64,
    content: String,
    mtime_nanos: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct TomlMetadata {
    mtime: SystemTime,
    len: u64,
    mtime_nanos: u32,
}

struct TomlCache {
    entries: HashMap<String, CachedToml>,
    insertion_order: VecDeque<String>,
}

const MAX_TOML_CACHE_SIZE: usize = 100;

static TOML_CACHE: LazyLock<Mutex<TomlCache>> = LazyLock::new(|| {
    Mutex::new(TomlCache {
        entries: HashMap::new(),
        insertion_order: VecDeque::new(),
    })
});

/// Lock the TOML cache. A poisoned mutex means a previous holder panicked
/// mid-update, so the cached contents may be inconsistent — recover by
/// clearing the cache rather than trusting it.
fn lock_toml_cache() -> std::sync::MutexGuard<'static, TomlCache> {
    match TOML_CACHE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = TomlCache {
                entries: HashMap::new(),
                insertion_order: VecDeque::new(),
            };
            guard
        }
    }
}

pub fn invalidate_toml_cache(path: &std::path::Path) {
    let key = toml_cache_key(path);
    if let Ok(mut cache) = TOML_CACHE.lock() {
        cache.entries.remove(&key);
        cache.insertion_order.retain(|k| k != &key);
    }
}

fn toml_cache_key(path: &std::path::Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| {
        path.parent()
            .and_then(|parent| Some(parent.canonicalize().ok()?.join(path.file_name()?)))
            .unwrap_or_else(|| path.to_path_buf())
    });
    canonical.to_string_lossy().into_owned()
}

fn toml_metadata(file: &fs::File, path: &std::path::Path) -> SnipResult<TomlMetadata> {
    let metadata = file
        .metadata()
        .map_err(|e| SnipError::io_error("stat toml file", path.to_path_buf(), e))?;
    let mtime = metadata
        .modified()
        .map_err(|e| SnipError::io_error("read mtime", path.to_path_buf(), e))?;
    let mtime_nanos = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    Ok(TomlMetadata {
        mtime,
        len: metadata.len(),
        mtime_nanos,
    })
}

fn toml_path_metadata(path: &std::path::Path) -> SnipResult<TomlMetadata> {
    let file = fs::File::open(path)
        .map_err(|e| SnipError::io_error("open toml file", path.to_path_buf(), e))?;
    toml_metadata(&file, path)
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
    let key = toml_cache_key(path);

    let path_metadata = toml_path_metadata(path)?;

    let cache = lock_toml_cache();
    if let Some(entry) = cache.entries.get(&key)
        && entry.mtime == path_metadata.mtime
        && entry.mtime_nanos == path_metadata.mtime_nanos
        && entry.len == path_metadata.len
    {
        return Ok(entry.content.clone());
    }
    drop(cache);

    // Read through the opened file and verify that its metadata did not change
    // during the read. This keeps content and cache metadata tied to one file
    // snapshot, even if the path is atomically replaced concurrently.
    for _ in 0..2 {
        let mut file = fs::File::open(path)
            .map_err(|e| SnipError::io_error("open toml file", path.to_path_buf(), e))?;
        let before = toml_metadata(&file, path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| SnipError::io_error("read toml file", path.to_path_buf(), e))?;
        let after = toml_metadata(&file, path)?;
        if before != after {
            continue;
        }

        let mut cache = lock_toml_cache();
        while cache.entries.len() >= MAX_TOML_CACHE_SIZE {
            let Some(oldest) = cache.insertion_order.pop_front() else {
                break;
            };
            if cache.entries.remove(&oldest).is_some() {
                break;
            }
        }

        if !cache.entries.contains_key(&key) {
            cache.insertion_order.push_back(key.clone());
        }
        cache.entries.insert(
            key.clone(),
            CachedToml {
                mtime: after.mtime,
                len: after.len,
                content: content.clone(),
                mtime_nanos: after.mtime_nanos,
            },
        );
        return Ok(content);
    }

    Err(SnipError::runtime_error(
        "read toml file",
        Some("file changed while being read"),
    ))
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
    fn test_sync_limit_rejects_non_positive_values() {
        for limit in ["0", "-1"] {
            let content = format!(
                "enabled = false\nserver_url = \"https://sync.example.com\"\nsync_interval_minutes = 30\nsync_limit = {limit}\n"
            );
            assert!(
                toml::from_str::<SyncSettings>(&content).is_err(),
                "sync_limit = {limit} should be rejected"
            );
        }
    }

    #[test]
    fn test_sync_settings_serialization() {
        // Ensure keychain is bypassed in CI environments without a keychain
        #[cfg(feature = "test-support")]
        unsafe {
            std::env::set_var("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
        }
        let settings = SyncSettings {
            enabled: true,
            server_url: "https://sync.example.com".to_string(),
            api_key: if cfg!(feature = "test-support") {
                "test-key-123"
            } else {
                ""
            }
            .to_string(),
            device_id: "device-456".to_string(),
            sync_interval_minutes: 60,
            auto_sync: true,
            auto_sync_debounce_seconds: 5,
            auto_sync_failure: AutoSyncFailureMode::Error,
            auto_sync_max_delay_seconds: Some(60),
            auto_sync_timeout_seconds: None,
            sync_direction: SyncDirection::Bidirectional,
            clipboard_auto_clear_seconds: Some(30),
            sync_limit: Some(2000),
            credential_revision: 0,
        };

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        assert!(toml_str.contains("enabled = true"));
        assert!(toml_str.contains("server_url = \"https://sync.example.com\""));
        #[cfg(feature = "test-support")]
        assert!(
            toml_str.contains("api_key = \"@keychain\"")
                || toml_str.contains("api_key = \"test-key-123\"")
        );
        #[cfg(not(feature = "test-support"))]
        assert!(toml_str.contains("api_key = \"\""));
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
    fn test_invalidate_toml_cache_does_not_duplicate_insertion_order() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = temp_dir.path().join("cache-churn.toml");
        std::fs::write(&path, "value = 1\n").unwrap();

        let key = path.to_string_lossy().to_string();
        for _ in 0..10 {
            invalidate_toml_cache(&path);
            let _ = cached_read_toml(&path).unwrap();
        }

        let cache = TOML_CACHE.lock().unwrap();
        let occurrences = cache.insertion_order.iter().filter(|k| **k == key).count();
        assert_eq!(occurrences, 1);
        assert_eq!(cache.entries.get(&key).map(|e| e.len), Some(10));
    }

    #[cfg(unix)]
    #[test]
    fn test_toml_cache_key_canonicalizes_symlink_aliases() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let path = temp_dir.path().join("real.toml");
        let alias = temp_dir.path().join("alias.toml");
        std::fs::write(&path, "value = 1\n").unwrap();
        std::os::unix::fs::symlink(&path, &alias).unwrap();

        assert_eq!(toml_cache_key(&path), toml_cache_key(&alias));
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
        // Ensure keychain is bypassed in CI environments without a keychain
        #[cfg(feature = "test-support")]
        unsafe {
            std::env::set_var("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
        }
        let settings = SyncSettings {
            enabled: true,
            server_url: "https://sync.example.com".to_string(),
            api_key: if cfg!(feature = "test-support") {
                "test-key"
            } else {
                ""
            }
            .to_string(),
            device_id: "device-1".to_string(),
            sync_interval_minutes: 15,
            auto_sync: true,
            auto_sync_debounce_seconds: 5,
            auto_sync_failure: AutoSyncFailureMode::Error,
            auto_sync_max_delay_seconds: Some(120),
            auto_sync_timeout_seconds: None,
            sync_direction: SyncDirection::Bidirectional,
            clipboard_auto_clear_seconds: Some(30),
            sync_limit: Some(500),
            credential_revision: 0,
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
        // Ensure keychain is bypassed in CI environments without a keychain
        #[cfg(feature = "test-support")]
        unsafe {
            std::env::set_var("SNP_ALLOW_PLAINTEXT_API_KEY", "true");
        }
        let settings = SyncSettings {
            enabled: true,
            server_url: "https://sync.example.com".to_string(),
            api_key: if cfg!(feature = "test-support") {
                "test-key"
            } else {
                ""
            }
            .to_string(),
            device_id: "device-1".to_string(),
            sync_interval_minutes: 15,
            auto_sync: true,
            auto_sync_debounce_seconds: 10,
            auto_sync_failure: AutoSyncFailureMode::Ignore,
            auto_sync_max_delay_seconds: None,
            auto_sync_timeout_seconds: None,
            sync_direction: SyncDirection::Push,
            clipboard_auto_clear_seconds: Some(60),
            sync_limit: Some(500),
            credential_revision: 0,
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
