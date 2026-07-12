use crate::CommandOutcome;
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
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<CommandOutcome> {
    let mode = if expanded {
        OutputMode::Expanded
    } else {
        OutputMode::Raw
    };
    let cancelled = Cell::new(false);
    let selected_command = Cell::new(None);

    run_snippet_selection(filter, library, false, runtime, |snippet, _copy_flag| {
        let result = process_snippet(snippet, mode, &cancelled)?;
        if let crate::ProcessResult::Done(cmd) = &result {
            selected_command.set(Some(cmd.clone()));
        }
        Ok(result)
    })?;

    if cancelled.get() {
        if let Some(path) = &output_file
            && path.is_file()
            && !path.is_symlink()
        {
            let _ = std::fs::remove_file(path);
        }
        return Ok(CommandOutcome::Cancelled);
    }

    if let Some(command) = selected_command.take() {
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
    }

    Ok(CommandOutcome::Success)
}
