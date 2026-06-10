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
    println!("Downloaded premade library '{name}' to {}", path.display());
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

/// Searches premade libraries on the server by query string.
pub fn run_search(query: String, runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
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
        .block_on(client.search_premade_libraries(&query))
        .map_err(|e| {
            crate::error::SnipError::runtime_error(
                "Failed to search premade libraries",
                Some(&e.to_string()),
            )
        })?;

    if libs.is_empty() {
        println!("No premade libraries found matching '{query}'.");
    } else {
        println!("Found {} premade libraries matching '{query}':", libs.len(),);
        for lib in &libs {
            let tags_display = if lib.tags.is_empty() {
                String::new()
            } else {
                let tags = lib.tags.join(", ");
                format!(" [{tags}]")
            };
            println!(
                "  {}: {} ({} snippets){tags_display}",
                lib.filename, lib.description, lib.snippet_count
            );
        }
    }
    Ok(())
}

/// Updates a premade library by re-downloading it and showing the diff.
pub fn run_update(name: String, runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
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

    let mgr = crate::library::LibraryManager::new()?;

    let old_content = if mgr.premade_exists(&name) {
        let premade_path = mgr.get_premade_dir().join(format!("{name}.toml"));
        std::fs::read_to_string(&premade_path).unwrap_or_default()
    } else {
        String::new()
    };

    let new_content = runtime
        .block_on(client.get_premade_library(&name))
        .map_err(|e| {
            crate::error::SnipError::runtime_error(
                "Failed to get premade library",
                Some(&e.to_string()),
            )
        })?;

    if old_content == new_content {
        println!("Premade library '{name}' is already up to date.");
        return Ok(());
    }

    if old_content.is_empty() {
        println!("Installing new premade library '{name}':");
        println!("  {} new lines", new_content.lines().count());
    } else {
        let old_lines: Vec<&str> = old_content.lines().collect();
        let new_lines: Vec<&str> = new_content.lines().collect();
        let added = new_lines.iter().filter(|l| !old_lines.contains(l)).count();
        let removed = old_lines.iter().filter(|l| !new_lines.contains(l)).count();
        println!("Updating premade library '{name}': +{added} / -{removed} lines",);
    }

    let path = mgr.save_premade_library(&name, &new_content)?;
    println!("Updated '{name}' → {}", path.display());
    Ok(())
}
