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
//!
//! ## Layout
//!
//! - [`toml_cache`]: pure TOML cache + CRC32 integrity helpers (no keyring,
//!   no sync). Core modules use this via the documented carve-out.
//! - [`sync_settings`]: sync-client configuration, keychain integration,
//!   save/load/get.
//!
//! Root re-exports preserve the pre-split `crate::config::*` paths.

pub mod sync_settings;
pub mod toml_cache;

pub use crate::utils::config::{derive_sync_state_dir, get_sync_config_path};
pub use sync_settings::*;
pub use toml_cache::{cached_read_toml, invalidate_toml_cache};
#[cfg(test)]
mod tests;
