use crate::commands::run_snippet_selection;
use crate::error::SnipResult;
use crate::library::Snippet;
use std::cell::Cell;

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
pub fn run(
    filter: Option<String>,
    library: Option<String>,
    _raw: bool,
    expanded: bool,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()> {
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
        std::process::exit(4);
    }

    if let Some(command) = selected_command.take() {
        println!("{command}");
    }

    Ok(())
}
