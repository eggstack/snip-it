use crate::CommandOutcome;
use crate::SelectionOutcome;
use crate::commands::run_snippet_selection;
use crate::error::SnipResult;
use crate::library::Snippet;
use std::cell::Cell;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq)]
enum OutputMode {
    Raw,
    Expanded,
}

/// Removes the output file on cancellation if it is a regular non-symlink file.
/// Ignores cleanup errors — cancellation must not become a deletion primitive.
fn cleanup_output_file_on_cancel(path: &PathBuf) {
    if path.is_file() && !path.is_symlink() {
        let _ = std::fs::remove_file(path);
    }
}

fn process_snippet(
    snippet: &Snippet,
    mode: OutputMode,
    cancelled: &Cell<bool>,
) -> SnipResult<crate::ProcessResult> {
    match mode {
        OutputMode::Raw => {
            let command = snippet.command.clone();
            Ok(crate::ProcessResult::Done(command))
        }
        OutputMode::Expanded => match crate::commands::expand_snippet_command(snippet)? {
            crate::commands::ExpandedCommand::Cancel => {
                cancelled.set(true);
                Ok(crate::ProcessResult::Cancel)
            }
            crate::commands::ExpandedCommand::Skip => Ok(crate::ProcessResult::Continue),
            crate::commands::ExpandedCommand::Expanded(cmd) => Ok(crate::ProcessResult::Done(cmd)),
        },
    }
}

/// Select a snippet and print its command to stdout (no execution).
///
/// When `output_file` is provided, writes the selection to that file instead
/// of stdout. Used by shell integration functions for lossless transport.
pub fn run(
    filter: Option<String>,
    library: Option<String>,
    _raw: bool,
    expanded: bool,
    output_file: Option<PathBuf>,
    sort_opts: Option<crate::sort::SortOptions>,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<CommandOutcome> {
    let mode = if expanded {
        OutputMode::Expanded
    } else {
        OutputMode::Raw
    };
    let cancelled = Cell::new(false);
    let selected_command = Cell::new(None);

    let selection_outcome = run_snippet_selection(
        filter,
        library,
        false,
        sort_opts,
        runtime,
        |snippet, _copy_flag| {
            let result = process_snippet(snippet, mode, &cancelled)?;
            if let crate::ProcessResult::Done(cmd) = &result {
                selected_command.set(Some(cmd.clone()));
            }
            Ok(result)
        },
    )?;

    match (selection_outcome, cancelled.get(), selected_command.take()) {
        (SelectionOutcome::Cancelled, _, _) | (_, true, _) => {
            if let Some(path) = &output_file {
                cleanup_output_file_on_cancel(path);
            }
            Ok(CommandOutcome::Cancelled)
        }
        (SelectionOutcome::Selected, false, Some(command)) => {
            if let Some(path) = output_file {
                if path.is_symlink() {
                    return Err(crate::error::SnipError::io_error(
                        "write selection to file",
                        path.clone(),
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "output file is a symlink; refusing to follow",
                        ),
                    ));
                }
                if path.is_dir() {
                    return Err(crate::error::SnipError::io_error(
                        "write selection to file",
                        path.clone(),
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "output file path is a directory",
                        ),
                    ));
                }
                std::fs::write(&path, command).map_err(|e| {
                    crate::error::SnipError::io_error("write selection to file", path.clone(), e)
                })?;
            } else {
                println!("{command}");
            }
            Ok(CommandOutcome::Success)
        }
        (SelectionOutcome::Selected, false, None) => Err(crate::error::SnipError::runtime_error(
            "Internal contract error",
            Some("SelectionOutcome::Selected but no command produced — this is a bug"),
        )),
    }
}
