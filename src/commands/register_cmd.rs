use crate::config::{DEFAULT_SERVER_URL, SyncSettings, load_sync_settings, save_sync_settings};
use crate::error::SnipResult;

/// Registers this device with a sync server and saves the API key to the OS keychain.
pub fn run(server: String, force: bool, runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
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

            if let Err(e) = save_sync_settings(&sync_settings) {
                eprintln!("Failed to save sync settings: {e}");
                return Err(e);
            }

            println!("Registration successful!");
            let masked_key = if api_key.len() > 8 {
                let chars: Vec<char> = api_key.chars().collect();
                let prefix: String = chars.iter().take(4).collect();
                let suffix: String = chars.iter().rev().take(4).collect();
                format!("{prefix}...{suffix}")
            } else {
                "****".to_string()
            };
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
