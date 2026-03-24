use crate::commands::init_library_manager;
use crate::config::{load_sync_settings, SyncSettings};
use crate::error::{SnipError, SnipResult};
use crate::library::LibraryManager;
use snip_proto::Library;
use std::io::{self, Write};
use std::path::PathBuf;

fn link_server_library(
    lib: &Library,
    mgr: &mut LibraryManager,
    non_interactive: bool,
    print_linked: bool,
) -> bool {
    let filename = lib.name.to_lowercase().replace(' ', "-");
    let existing_lib_id = mgr
        .get_library_by_filename(&filename)
        .map(|l| l.library_id.clone());

    if let Some(existing_id) = &existing_lib_id {
        if !existing_id.is_empty() && existing_id != &lib.id {
            println!("  Library '{}' has different server ID, skipping", lib.name);
            return false;
        }

        let lib_path = mgr.get_libraries_dir().join(format!("{}.toml", filename));
        let local_has_content = if lib_path.exists() {
            if let Ok(snippets) = crate::library::load_library(&lib_path) {
                !snippets.snippets.is_empty()
            } else {
                false
            }
        } else {
            false
        };

        if existing_id.is_empty() && local_has_content {
            println!(
                "\n  Local library '{}' has snippets. Server also has snippets.",
                filename
            );
            match prompt_conflict(&filename, non_interactive).as_deref() {
                Some("overwrite") => {
                    println!("  Will overwrite with server version");
                }
                Some("rename") => {
                    let new_name = format!("{}_local", filename);
                    println!("  Renaming to '{}' and pulling from server", new_name);
                    if let Err(e) = mgr.create_library(&new_name) {
                        eprintln!("    Failed to create backup: {}", e);
                        return false;
                    }
                    if let Err(e) = mgr.update_library_id(&new_name, &lib.id) {
                        eprintln!("    Failed to link backup: {}", e);
                        return false;
                    }
                    println!("    Created '{}' with local content", new_name);
                    return true;
                }
                _ => {
                    println!("  Skipping, keeping local version");
                    return false;
                }
            }
        }

        if existing_id.is_empty() {
            if let Err(e) = mgr.update_library_id(&filename, &lib.id) {
                eprintln!("  Failed to link '{}': {}", lib.name, e);
                return false;
            }
            println!("  Linked '{}' to server library '{}'", filename, lib.id);
            return true;
        } else if print_linked {
            println!("  Library '{}' already linked, skipping", lib.name);
        }
        return false;
    }

    match mgr.add_server_library(&lib.name, &lib.id) {
        Ok(path) => {
            println!("  Created '{}' at {}", lib.name, path.display());
            true
        }
        Err(e) => {
            eprintln!("  Failed to create library '{}': {}", lib.name, e);
            false
        }
    }
}

pub fn prompt_conflict(lib_name: &str, non_interactive: bool) -> Option<String> {
    if non_interactive {
        println!(
            "  Conflict: '{}' has different content, keeping local (non-interactive mode)",
            lib_name
        );
        return None;
    }

    println!(
        "\nConflict: Local library '{}' has different content than server",
        lib_name
    );
    println!("  (s)kip - keep local version");
    println!("  (o)verwrite - replace with server version");
    println!("  (r)ename - rename local and pull from server");
    print!("Choice [s/o/r]: ");

    io::stdout().flush().ok();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() {
        match input.trim().to_lowercase().as_str() {
            "o" => Some("overwrite".to_string()),
            "r" => Some("rename".to_string()),
            _ => None,
        }
    } else {
        None
    }
}

fn list_and_link_server_libraries(
    runtime: &tokio::runtime::Runtime,
    sync_settings: &SyncSettings,
    non_interactive: bool,
) -> SnipResult<bool> {
    use crate::sync::SyncClient;

    let mut client = runtime
        .block_on(SyncClient::create(sync_settings.clone()))
        .map_err(|e| {
            SnipError::runtime_error("Failed to create sync client", Some(&e.to_string()))
        })?;

    match runtime.block_on(client.list_libraries()) {
        Ok(libs) => {
            let mut mgr = init_library_manager().map_err(|e| {
                SnipError::runtime_error(
                    "Failed to initialize library manager",
                    Some(&e.to_string()),
                )
            })?;

            let mut linked_any = false;

            for lib in libs {
                if link_server_library(&lib, &mut mgr, non_interactive, false) {
                    linked_any = true;
                }
            }

            Ok(linked_any)
        }
        Err(e) => Err(SnipError::runtime_error(
            "Failed to fetch server libraries",
            Some(&e.to_string()),
        )),
    }
}

pub fn run(
    _config: Option<PathBuf>,
    library: Option<String>,
    servers: bool,
    non_interactive: bool,
    push_only: bool,
    pull_only: bool,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()> {
    let sync_settings = match load_sync_settings() {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("Failed to load sync settings: {}", e);
            SyncSettings::default()
        }
    };

    if servers {
        if !sync_settings.enabled {
            eprintln!("Sync is not enabled. Configure sync settings first.");
            return Ok(());
        }

        let mut client = runtime
            .block_on(crate::sync::SyncClient::create(sync_settings.clone()))
            .map_err(|e| {
                SnipError::runtime_error("Failed to create sync client", Some(&e.to_string()))
            })?;

        match runtime.block_on(client.list_libraries()) {
            Ok(libs) => {
                println!("Server libraries:");
                for lib in libs {
                    println!("  {} ({})", lib.name, lib.id);
                }
            }
            Err(e) => eprintln!("Failed to list server libraries: {}", e),
        }
        return Ok(());
    }

    let mut client = runtime
        .block_on(crate::sync::SyncClient::create(sync_settings.clone()))
        .map_err(|e| {
            SnipError::runtime_error("Failed to create sync client", Some(&e.to_string()))
        })?;

    match runtime.block_on(client.list_libraries()) {
        Ok(libs) => {
            let mut mgr = init_library_manager().map_err(|e| {
                SnipError::runtime_error(
                    "Failed to initialize library manager",
                    Some(&e.to_string()),
                )
            })?;

            for lib in libs {
                link_server_library(&lib, &mut mgr, non_interactive, true);
            }

            println!("\nPulling snippets from server...");
            crate::sync_commands::run_sync(
                &sync_settings,
                None,
                non_interactive,
                push_only,
                pull_only,
                runtime,
            );
            return Ok(());
        }
        Err(e) => eprintln!("Failed to pull libraries: {}", e),
    }

    if !sync_settings.api_key.is_empty() && !sync_settings.device_id.is_empty() {
        let linked = list_and_link_server_libraries(runtime, &sync_settings, non_interactive)?;
        if linked {
            println!("\nSyncing libraries...");
        }
    }

    crate::sync_commands::run_sync(
        &sync_settings,
        library.as_deref(),
        non_interactive,
        push_only,
        pull_only,
        runtime,
    );
    Ok(())
}
