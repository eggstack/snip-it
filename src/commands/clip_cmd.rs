use crate::commands::expand_snippet_command;
use crate::commands::run_snippet_selection;
use crate::error::SnipResult;
use crate::library::Snippet;
use crate::logging::audit_log;
use std::path::PathBuf;

fn process_snippet(
    snippet: &Snippet,
    _copy_flag: Option<String>,
) -> SnipResult<crate::ProcessResult> {
    use crate::logging::log_command_execution;

    let final_command = match expand_snippet_command(snippet)? {
        crate::commands::ExpandedCommand::Cancel => return Ok(crate::ProcessResult::Cancel),
        crate::commands::ExpandedCommand::Skip => return Ok(crate::ProcessResult::Continue),
        crate::commands::ExpandedCommand::Expanded(cmd) => cmd,
    };

    crate::clipboard::copy_to_clipboard_auto(&final_command)?;
    if let Err(e) = audit_log("copy", snippet, None) {
        tracing::debug!("Audit log write failed: {}", e);
    }
    let ok_result: std::result::Result<(), String> = Ok(());
    log_command_execution(&final_command, &[], &ok_result, None);
    Ok(crate::ProcessResult::Done(
        "Copied to clipboard".to_string(),
    ))
}

/// Copies the selected snippet's expanded command to the clipboard.
pub fn run(
    filter: Option<String>,
    do_sync: bool,
    library: Option<String>,
    _config: Option<PathBuf>,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()> {
    run_snippet_selection(filter, library, do_sync, runtime, |snippet, copy_flag| {
        process_snippet(snippet, copy_flag)
    })
}
