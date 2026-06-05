use crate::commands::run_snippet_selection;
use crate::error::SnipResult;
use std::path::PathBuf;

/// Opens the TUI snippet selector and displays the selected snippet's details.
pub fn run(
    filter: Option<String>,
    do_sync: bool,
    library: Option<String>,
    config: Option<PathBuf>,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()> {
    // Propagate --config to the snippet selection pipeline
    let effective_library = library.or_else(|| {
        config.as_ref().and_then(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
    });
    run_snippet_selection(
        filter,
        effective_library,
        do_sync,
        runtime,
        |snippet, _copy_flag| {
            println!("Description: {}", snippet.description);
            println!("Command: {}", snippet.command);
            println!("Output: {}", snippet.output);
            println!("Tags: {}", snippet.tags.join(", "));
            println!("Folders: {}", snippet.folders.join(", "));
            println!("Favorite: {}", snippet.favorite);
            Ok(crate::ProcessResult::Done(String::new()))
        },
    )
}
