use crate::config::get_sync_settings;
use crate::error::SnipResult;
use crate::library::LibraryManager;
use crate::sync::SyncClient;

/// Lists all premade libraries available on the sync server.
pub fn run_list(runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
    let sync_settings = get_sync_settings();

    if !sync_settings.enabled {
        eprintln!("Sync is not enabled. Configure sync settings first.");
        return Ok(());
    }

    let mut client = runtime
        .block_on(SyncClient::create(sync_settings.clone()))
        .map_err(|e| {
            crate::error::SnipError::runtime_error(
                "Failed to create sync client",
                Some(&e.to_string()),
            )
        })?;

    let libs = runtime
        .block_on(client.list_premade_libraries())
        .map_err(|e| {
            crate::error::SnipError::runtime_error(
                "Failed to list premade libraries",
                Some(&e.to_string()),
            )
        })?;

    if libs.is_empty() {
        println!("No premade libraries available.");
    } else {
        println!("Available premade libraries:");
        for lib in libs {
            println!("  {}: {}", lib.filename, lib.description);
        }
    }
    Ok(())
}

pub fn run_get(
    name: Option<String>,
    all: bool,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()> {
    let sync_settings = get_sync_settings();

    if !sync_settings.enabled {
        eprintln!("Sync is not enabled. Configure sync settings first.");
        return Ok(());
    }

    let mut client = runtime
        .block_on(SyncClient::create(sync_settings.clone()))
        .map_err(|e| {
            crate::error::SnipError::runtime_error(
                "Failed to create sync client",
                Some(&e.to_string()),
            )
        })?;

    if all {
        let libs = runtime
            .block_on(client.list_premade_libraries())
            .map_err(|e| {
                crate::error::SnipError::runtime_error(
                    "Failed to list premade libraries",
                    Some(&e.to_string()),
                )
            })?;

        if libs.is_empty() {
            println!("No premade libraries available.");
            return Ok(());
        }

        let mgr = LibraryManager::new()?;
        let mut results: Vec<(String, bool, String)> = Vec::new();

        for lib in libs {
            if mgr.premade_exists(&lib.filename) {
                continue;
            }

            match runtime.block_on(client.get_premade_library(&lib.filename)) {
                Ok(content) => match mgr.save_premade_library(&lib.filename, &content) {
                    Ok(path) => {
                        results.push((lib.filename, true, path.display().to_string()));
                    }
                    Err(e) => {
                        results.push((lib.filename, false, e.to_string()));
                    }
                },
                Err(e) => {
                    results.push((lib.filename, false, e.to_string()));
                }
            }
        }

        if results.is_empty() {
            println!("All premade libraries already downloaded.");
        } else {
            for (name, success, msg) in results {
                if success {
                    println!("  + {name} → {msg}");
                } else {
                    println!("  ✗ {name}: {msg}");
                }
            }
        }
        return Ok(());
    }

    let name = match name {
        Some(n) if n != "all" => n,
        _ => {
            eprintln!("Usage: snp premade get <name>   or   snp premade get all");
            eprintln!("Use 'snp premade list' to see available libraries.");
            return Err(crate::error::SnipError::runtime_error(
                "Invalid usage",
                Some("Expected: snp premade get <name> or snp premade get all"),
            ));
        }
    };

    let content = runtime
        .block_on(client.get_premade_library(&name))
        .map_err(|e| {
            crate::error::SnipError::runtime_error(
                "Failed to get premade library",
                Some(&e.to_string()),
            )
        })?;

    let mgr = LibraryManager::new()?;
    let path = mgr.save_premade_library(&name, &content)?;
    println!(
        "Downloaded premade library '{}' to {}",
        name,
        path.display()
    );
    Ok(())
}

/// Downloads all missing premade libraries from the server.
pub fn run_sync(runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
    let sync_settings = get_sync_settings();

    if !sync_settings.enabled {
        eprintln!("Sync is not enabled. Configure sync settings first.");
        return Ok(());
    }

    crate::sync_commands::run_premade_sync(&sync_settings, runtime)?;
    Ok(())
}
