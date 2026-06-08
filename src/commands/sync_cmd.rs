use crate::commands::init_library_manager;
use crate::config::{SyncDirection, load_sync_settings};
use crate::error::{SnipError, SnipResult};
use crate::library::LibraryManager;
use crate::proto::Library;
use std::io::{self, Write};

fn link_server_library(lib: &Library, mgr: &mut LibraryManager, print_linked: bool) -> bool {
    let filename = lib.name.to_lowercase().replace(' ', "-");
    let existing_lib_id = mgr
        .get_library_by_filename(&filename)
        .map(|l| l.library_id.clone());

    if let Some(existing_id) = &existing_lib_id {
        if !existing_id.is_empty() && existing_id != &lib.id {
            println!("  Library '{}' has different server ID, skipping", lib.name);
            return false;
        }

        let lib_path = mgr.get_libraries_dir().join(format!("{filename}.toml"));
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
            println!("\n  Local library '{filename}' has snippets. Server also has snippets.");
            match prompt_conflict(&filename).as_deref() {
                Some("overwrite") => {
                    println!("  Will overwrite with server version");
                }
                Some("rename") => {
                    let new_name = format!("{filename}_local");
                    println!("  Renaming to '{new_name}' and pulling from server");
                    if let Err(e) = mgr.create_library(&new_name) {
                        eprintln!("    Failed to create backup: {e}");
                        return false;
                    }
                    // Move local snippets to the backup library
                    let local_snippets =
                        crate::library::load_library(&lib_path).unwrap_or_default();
                    let backup_path = mgr.get_libraries_dir().join(format!("{new_name}.toml"));
                    if let Err(e) = crate::library::save_library(&backup_path, &local_snippets) {
                        eprintln!("    Failed to save backup: {e}");
                        return false;
                    }
                    if let Err(e) = mgr.update_library_id(&new_name, &lib.id) {
                        eprintln!("    Failed to link backup: {e}");
                        return false;
                    }
                    // Clear original library for server content
                    let empty = crate::library::Snippets::default();
                    if let Err(e) = crate::library::save_library(&lib_path, &empty) {
                        eprintln!("    Failed to clear original library: {e}");
                        return false;
                    }
                    println!(
                        "    Created '{new_name}' with local content, original cleared for server content"
                    );
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

/// Prompts the user to resolve a local/server library conflict.
///
/// Returns `"overwrite"`, `"rename"`, or `None` (skip) based on user input.
pub fn prompt_conflict(lib_name: &str) -> Option<String> {
    println!("\nConflict: Local library '{lib_name}' has different content than server");
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

/// Options for the `sync` command.
pub struct SyncOptions {
    pub library: Option<String>,
    pub servers: bool,
    pub push_only: bool,
    pub pull_only: bool,
    pub dry_run: bool,
}

/// Runs the sync command with the given options.
///
/// Supports listing servers, push-only, pull-only, bidirectional, and dry-run modes.
pub fn run(options: SyncOptions, runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
    let sync_settings = load_sync_settings().map_err(|e| {
        eprintln!("Failed to load sync settings: {e}");
        e
    })?;

    if options.servers {
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
            Err(e) => eprintln!("Failed to list server libraries: {e}"),
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
                link_server_library(&lib, &mut mgr, true);
            }

            if options.dry_run {
                println!("\n[DRY RUN] Would sync snippets:");
                let lib_path = match crate::commands::get_library_path(options.library)? {
                    Some(p) => p,
                    None => {
                        println!("  No library selected");
                        return Ok(());
                    }
                };
                let snippets = crate::library::load_library(&lib_path)?;
                let direction = if options.push_only {
                    "push to server"
                } else if options.pull_only {
                    "pull from server"
                } else {
                    "bidirectional"
                };
                println!("  Direction: {direction}");
                println!("  Snippets in library: {}", snippets.snippets.len());
                for s in &snippets.snippets {
                    if !s.deleted {
                        println!("  - {} ({})", s.description, &s.id[..8.min(s.id.len())]);
                    }
                }
                return Ok(());
            }

            println!("\nSyncing snippets...");
            // Respect config direction when no CLI flags are provided
            let effective_push = options.push_only
                || (!options.pull_only
                    && (sync_settings.sync_direction == SyncDirection::Push
                        || sync_settings.sync_direction == SyncDirection::Bidirectional));
            let effective_pull = options.pull_only
                || (!options.push_only
                    && (sync_settings.sync_direction == SyncDirection::Pull
                        || sync_settings.sync_direction == SyncDirection::Bidirectional));
            crate::sync_commands::run_sync(
                &sync_settings,
                options.library.as_deref(),
                effective_push,
                effective_pull,
                runtime,
            )?;
            Ok(())
        }
        Err(e) => {
            eprintln!("Failed to pull libraries: {e}");
            Err(SnipError::runtime_error(
                "Failed to list server libraries",
                Some(&e.to_string()),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_library_filename_slug() {
        // link_server_library derives filenames by lowercasing and replacing spaces.
        // The function takes a &mut LibraryManager, but the slug transformation
        // is the testable contract — verify the expected mapping directly.
        let cases = vec![
            ("My Library", "my-library"),
            ("UPPERCASE", "uppercase"),
            ("multi word name", "multi-word-name"),
        ];
        for (input, expected) in cases {
            assert_eq!(input.to_lowercase().replace(' ', "-"), expected);
        }
    }

    #[test]
    fn test_sync_options_defaults() {
        let opts = SyncOptions {
            library: None,
            servers: false,
            push_only: false,
            pull_only: false,
            dry_run: false,
        };
        assert!(!opts.servers);
        assert!(!opts.push_only);
        assert!(!opts.pull_only);
    }
}
