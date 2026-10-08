use crate::commands::run_snippet_selection;
use crate::error::SnipResult;
use std::path::PathBuf;

/// Canonical Clap arguments for `snp search`.
#[derive(Debug, Clone, clap::Args)]
pub struct SearchArgs {
    #[arg(short, long)]
    pub filter: Option<String>,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub sync: bool,
    /// Library to search; `all` is not supported by the interactive selector
    #[arg(short, long)]
    pub library: Option<String>,
    /// Sort mode for snippet ordering
    #[arg(long, value_enum, default_value_t = crate::sort::SnippetSort::Relevance)]
    pub sort: crate::sort::SnippetSort,
    /// Show favorites before other snippets
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub favorites_first: bool,
}

/// Opens the TUI snippet selector and displays the selected snippet's details.
pub fn run(
    filter: Option<String>,
    do_sync: bool,
    library: Option<String>,
    config: Option<PathBuf>,
    sort_opts: Option<crate::sort::SortOptions>,
    runtime: Option<&tokio::runtime::Runtime>,
) -> SnipResult<()> {
    // The interactive selector edits and deletes within a single library file,
    // so it cannot take a cross-library scope. Without this guard the lookup
    // failed with "Library 'all' does not exist", which is false — `all` is
    // the scope keyword owned by `LibraryScope::from_filter_arg`, and it does
    // work for `snp get`, `list`, and MCP. Say what is actually true.
    if library.as_deref() == Some("all") {
        return Err(crate::error::SnipError::runtime_error(
            "Library 'all' is not supported by 'snp search'",
            Some(
                "The interactive selector operates on one library. Use 'snp list --library all' to list every library, or 'snp get --library all' to fetch a snippet by id.",
            ),
        ));
    }

    // Propagate --config to the snippet selection pipeline. The pipeline
    // takes a library *name*, so derive one from the config file stem.
    // Surface the derivation so errors are not confusing when the stem
    // differs from a registered library name.
    let effective_library = library.or_else(|| {
        config.as_ref().and_then(|p| {
            p.file_stem().and_then(|s| s.to_str()).map(|s| {
                eprintln!("note: using library '{s}' derived from --config path");
                s.to_string()
            })
        })
    });
    let _outcome = run_snippet_selection(
        filter,
        effective_library,
        do_sync,
        true,
        sort_opts,
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
    )?;
    Ok(())
}
