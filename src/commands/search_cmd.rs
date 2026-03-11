use crate::commands::{get_library_path, get_snippet_data};
use crate::error::SnipResult;
use std::path::PathBuf;

pub fn run(
    filter: Option<String>,
    do_sync: bool,
    library: Option<String>,
    _config: Option<PathBuf>,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()> {
    let lib_path = match get_library_path(library.clone())? {
        Some(p) => p,
        None => {
            eprintln!("No library found. Create one with 'snp library create <name>'");
            return Ok(());
        }
    };
    let snippets = crate::library::load_library(&lib_path)?;
    let (descriptions, commands, tags, folders, favorites) = get_snippet_data(&snippets);
    let result = crate::ui::select_snippet(
        &descriptions,
        &commands,
        &tags,
        true,
        filter.as_deref(),
        &folders,
        &favorites,
    )?;
    if let Some((idx, _)) = result {
        let snippet = &snippets.snippets[idx];
        println!("Description: {}", snippet.description);
        println!("Command: {}", snippet.command);
        println!("Output: {}", snippet.output);
        println!("Tags: {:?}", snippet.tags);
        println!("Folders: {:?}", snippet.folders);
        println!("Favorite: {}", snippet.favorite);
    }
    if do_sync {
        crate::sync_commands::run_sync(
            &crate::config::SyncSettings::default(),
            None,
            false,
            false,
            false,
            runtime,
        );
    }
    Ok(())
}
