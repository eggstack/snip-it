use crate::config::{DEFAULT_SERVER_URL, SyncSettings, load_sync_settings, save_sync_settings};
use crate::error::SnipResult;

/// Canonical Clap arguments for `snp register`.
#[derive(Debug, Clone, clap::Args)]
pub struct RegisterArgs {
    #[arg(long, default_value = crate::config::DEFAULT_SERVER_URL)]
    pub server: String,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub force: bool,
}

/// Registers this device with a sync server and saves the API key to the OS keychain.
pub fn run(server: String, force: bool, runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
    // Migrate a legacy plaintext API key before the settings are used. This
    // rewrites `sync.toml`, so it must run outside any lock-holding path.
    crate::config::migrate_plaintext_api_key_to_keychain();
    if !force
        && let Ok(settings) = load_sync_settings()
        && !settings.device_id.is_empty()
    {
        eprintln!("Already registered! Device ID: {}", settings.device_id);
        eprintln!(
            "Config file: {}",
            crate::config::get_sync_config_path().display()
        );
        eprintln!("Use --force to re-register.");
        return Ok(());
    }

    let server_url = if server != DEFAULT_SERVER_URL {
        server.clone()
    } else if let Ok(settings) = load_sync_settings() {
        if !settings.server_url.is_empty() {
            settings.server_url.clone()
        } else {
            server.clone()
        }
    } else {
        server.clone()
    };

    match runtime.block_on(crate::sync::SyncClient::register(server_url.clone())) {
        Ok((api_key, device_id)) => {
            let mut sync_settings = SyncSettings::default();
            sync_settings.enabled = true;
            sync_settings.server_url = server_url.clone();
            sync_settings.api_key = api_key.clone();
            sync_settings.device_id = device_id.clone();
            sync_settings.credential_revision = 1;

            if let Err(e) = save_sync_settings(&sync_settings) {
                eprintln!("Failed to save sync settings: {e}");
                return Err(e);
            }

            println!("Registration successful!");
            let masked_key = mask_api_key(&api_key);
            println!("API key: {masked_key}");
            println!("(Note: API key is stored in the OS keychain when available.)");
            println!("Device ID: {device_id}");
            println!(
                "Saved to: {}",
                crate::config::get_sync_config_path().display()
            );
        }
        Err(e) => {
            return Err(crate::error::SnipError::runtime_error(
                "Registration failed",
                Some(&e.to_string()),
            ));
        }
    }
    Ok(())
}

/// Masks an API key for display: the first 4 and last 4 characters.
///
/// The comparison and the arithmetic both use **character** counts, never
/// byte counts. The API key is server-supplied and therefore arbitrary
/// (`snp register --server <url>` accepts any server), so a short multibyte
/// key used to slip past a byte-length guard while the character arithmetic
/// printed the whole secret — or underflowed `chars.len() - 4` and panicked
/// in debug builds.
fn mask_api_key(api_key: &str) -> String {
    let chars: Vec<char> = api_key.chars().collect();
    if chars.len() > 8 {
        let prefix: String = chars.iter().take(4).collect();
        // `saturating_sub` keeps the arithmetic non-underflowing even if the
        // guard above is ever relaxed.
        let suffix: String = chars.iter().skip(chars.len().saturating_sub(4)).collect();
        format!("{prefix}...{suffix}")
    } else {
        "****".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_api_key_keeps_shape_for_normal_keys() {
        assert_eq!(mask_api_key("0123456789abcdef"), "0123...cdef");
        // A real (UUID-shaped) key is unaffected by the fix.
        let uuid = "6f1c2f0e-6b1a-4a6e-9a2f-1c2d3e4f5a6b";
        assert_eq!(mask_api_key(uuid), "6f1c...5a6b");
    }

    #[test]
    fn mask_api_key_never_reveals_a_short_multibyte_key() {
        // 4 characters but 16 bytes: the byte-length guard used to pass while
        // the character arithmetic printed the complete key.
        let key = "🔑🔑🔑🔑";
        let masked = mask_api_key(key);
        assert_eq!(masked, "****");
        assert!(!masked.contains(key), "mask leaked the full key: {masked}");

        // 3 characters: the old code underflowed `chars.len() - 4`.
        let key = "🔑🔑🔑";
        let masked = mask_api_key(key);
        assert_eq!(masked, "****");
        assert!(!masked.contains(key), "mask leaked the full key: {masked}");
    }

    #[test]
    fn mask_api_key_never_reveals_a_long_multibyte_key() {
        // 9 characters: masked, but the full key must not appear.
        let key = "🔑🔑🔑🔑🔑🔑🔑🔑🔑";
        let masked = mask_api_key(key);
        assert_eq!(masked, "🔑🔑🔑🔑...🔑🔑🔑🔑");
        assert!(!masked.contains(key), "mask leaked the full key: {masked}");
    }

    #[test]
    fn mask_api_key_masks_short_ascii_keys() {
        assert_eq!(mask_api_key("short"), "****");
        assert_eq!(mask_api_key(""), "****");
        assert_eq!(mask_api_key("12345678"), "****");
    }
}
