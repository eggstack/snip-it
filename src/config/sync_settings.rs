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

    // Refuse the write while an interrupted transaction awaits recovery:
    // `sync.toml` must not land in a config tree that a pending restore is
    // about to roll back.
    crate::transaction::gate_mutation_on_interrupted_transactions(
        &crate::config::derive_sync_state_dir(),
        &crate::local_data::transaction_dir(),
    )?;

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

/// Returns `true` the first time a corruption warning is emitted for `path`
/// in this process, and `false` afterwards.
///
/// `load_sync_settings` is called more than once per command (the
/// plaintext→keychain migration loads the settings before the command does),
/// and the corruption warning must reach the user exactly once — not once per
/// call.
fn claim_corrupt_config_warning(path: &std::path::Path) -> bool {
    static WARNED: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);
    let mut warned = WARNED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if warned.as_deref() == Some(path) {
        return false;
    }
    *warned = Some(path.to_path_buf());
    true
}

/// Report a corrupt `sync.toml` and fail closed.
///
/// The file is never modified: it is only copied aside so the user can
/// inspect it. Returning the defaults would be a *writable* state, and the
/// next writer (`snp sync config --auto-sync on`, for instance) would persist
/// those defaults over the user's configuration — destroying the very
/// settings the backup copy was taken to preserve.
fn fail_closed_on_corrupt_config(
    path: &std::path::Path,
    reason: &str,
    detail: Option<&str>,
) -> SnipError {
    let backup_path = path.with_extension("toml.corrupt.bak");
    let backup_error = fs::copy(path, &backup_path).err();

    if claim_corrupt_config_warning(path) {
        match &backup_error {
            Some(backup_err) => {
                tracing::error!("Failed to backup {reason} sync config: {backup_err}");
                eprintln!(
                    "warning: {} {reason} and may be corrupted; \
                     it was left unchanged and sync settings could not be loaded \
                     (the backup copy also failed: {backup_err}). \
                     Repair or remove the file, then run 'snp register --force' to reconfigure.",
                    path.display()
                );
            }
            None => {
                tracing::info!(
                    "Backed up {reason} sync config to {}",
                    backup_path.display()
                );
                eprintln!(
                    "warning: {} {reason} and may be corrupted; \
                     it was backed up to {} and left unchanged, and sync settings could not be loaded. \
                     Repair or remove the file, then run 'snp register --force' to reconfigure.",
                    path.display(),
                    backup_path.display()
                );
            }
        }
    }

    let mut message = format!(
        "{} {reason} and may be corrupted; it was left unchanged",
        path.display()
    );
    if let Some(detail) = detail {
        message.push_str(&format!(": {detail}"));
    }
    SnipError::runtime_error("sync config could not be loaded", Some(&message))
}

pub fn load_sync_settings() -> SnipResult<SyncSettings> {
    let path = get_sync_config_path();

    if !path.exists() {
        return Ok(SyncSettings::default());
    }

    let content = cached_read_toml(&path)?;

    if !verify_integrity(&content) {
        // Fail closed: a corrupt config must never be silently replaced by a
        // writable default.
        return Err(fail_closed_on_corrupt_config(
            &path,
            "failed its integrity check",
            None,
        ));
    }

    let content = strip_integrity_line(&content);
    let fixed_content = fix_invalid_toml_escapes(&content);

    let config: SyncConfigFile = match toml::from_str(&fixed_content) {
        Ok(config) => config,
        Err(e) => {
            // A parse failure means the file cannot be understood at all (for
            // example it was written by a different schema version). The CRC
            // gate only catches bit corruption, so this path needs the same
            // treatment: back the file up and say so on stderr. Without it the
            // caller silently falls back to the defaults and the user sees
            // "sync disabled" with no explanation and no file to inspect.
            return Err(fail_closed_on_corrupt_config(
                &path,
                "could not be parsed",
                Some(&e.to_string()),
            ));
        }
    };
    let settings = config.settings.sync;

    // NOTE: loading the settings is a pure read. The plaintext→keychain
    // migration is an explicit step (see
    // `migrate_plaintext_api_key_to_keychain`) because it rewrites
    // `sync.toml`, which takes the local-data lock. Performing it here made
    // every read path mutate the config, and a caller that already held the
    // lock (e.g. `snp restore`) deadlocked on the nested acquire.

    Ok(settings)
}

/// Migrates a plaintext API key in `sync.toml` to the OS keychain.
///
/// This is a *mutating* operation: it rewrites `sync.toml`, which acquires the
/// local-data lock. The lock protocol is not reentrant, so this must not be
/// called from a read path, nor from a caller that already holds the lock —
/// the nested acquire would spin until it times out.
///
/// Call it from the credential-consuming paths (sync, register, the auto-sync
/// worker) before the settings are loaded. The plaintext key stays in the
/// in-memory settings for the current operation, so the caller can still
/// complete the sync/register with the real credential.
pub fn migrate_plaintext_api_key_to_keychain() {
    let Ok(settings) = load_sync_settings() else {
        return;
    };
    migrate_plaintext_api_key(
        &settings,
        |api_key| keychain_store(api_key, KEYCHAIN_DEFAULT_USER),
        save_sync_settings,
    );
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
