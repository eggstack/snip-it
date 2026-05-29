use crate::commands::init_library_manager;
use crate::config::{load_sync_settings, save_sync_settings, SyncSettings};
use crate::error::SnipResult;
use crate::library::LibraryManager;

pub fn run(server: String, runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
    if let Ok(settings) = load_sync_settings() {
        if !settings.device_id.is_empty() {
            eprintln!("Already registered! Device ID: {}", settings.device_id);
            eprintln!(
                "Config file: {}",
                crate::config::get_sync_config_path().display()
            );
            eprintln!("If you want to re-register, remove this file.");
            return Ok(());
        }
    }

    let _config_path = match init_library_manager() {
        Ok(mgr) => match mgr.get_primary_library() {
            Some(primary) => mgr
                .get_libraries_dir()
                .join(format!("{}.toml", primary.filename)),
            None => LibraryManager::get_default_snippets_path(),
        },
        Err(_) => LibraryManager::get_default_snippets_path(),
    };

    let server_url = if server != "http://localhost:50051" {
        server.clone()
    } else if let Ok(settings) = load_sync_settings() {
        if !settings.server_url.is_empty() {
            settings.server_url
        } else {
            server.clone()
        }
    } else {
        server.clone()
    };

    match runtime.block_on(crate::sync::SyncClient::register(server_url.clone())) {
        Ok((api_key, device_id)) => {
            let sync_settings = SyncSettings {
                enabled: true,
                server_url: server_url.clone(),
                api_key: api_key.clone(),
                device_id: device_id.clone(),
                ..Default::default()
            };

            if let Err(e) = save_sync_settings(&sync_settings) {
                eprintln!("Failed to save sync settings: {}", e);
                return Err(e);
            }

            println!("Registration successful!");
            let masked_key = if api_key.len() > 8 {
                format!("{}...{}", &api_key[..4], &api_key[api_key.len() - 4..])
            } else {
                "****".to_string()
            };
            println!("API key: {}", masked_key);
            println!("Device ID: {}", device_id);
            println!(
                "Saved to: {}",
                crate::config::get_sync_config_path().display()
            );
        }
        Err(e) => {
            eprintln!("Registration failed: {}", e);
        }
    }
    Ok(())
}
