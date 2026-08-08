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
    let final_command = match expand_snippet_command(snippet)? {
        crate::commands::ExpandedCommand::Cancel => return Ok(crate::ProcessResult::Cancel),
        crate::commands::ExpandedCommand::Skip => return Ok(crate::ProcessResult::Continue),
        crate::commands::ExpandedCommand::Expanded(cmd) => cmd,
    };

    crate::clipboard::copy_to_clipboard_auto(&final_command)?;
    if let Err(e) = audit_log("copy", snippet, None) {
        tracing::debug!("Audit log write failed: {}", e);
    }
    // Record usage for sorting
    let mut usage_idx = crate::usage::UsageIndex::load();
    usage_idx.record_use(&snippet.id);
    if let Err(e) = usage_idx.save() {
        tracing::debug!("Usage save failed: {}", e);
    }
    Ok(crate::ProcessResult::Done(
        "Copied to clipboard".to_string(),
    ))
}

/// Copy a specific snippet's command to clipboard, bypassing TUI selection.
pub fn run_exact(
    snippet: &crate::library::Snippet,
    do_sync: bool,
    runtime: Option<&tokio::runtime::Runtime>,
) -> SnipResult<()> {
    use crate::commands::expand_snippet_command;
    let final_command = match expand_snippet_command(snippet)? {
        crate::commands::ExpandedCommand::Cancel => return Ok(()),
        crate::commands::ExpandedCommand::Skip => return Ok(()),
        crate::commands::ExpandedCommand::Expanded(cmd) => cmd,
    };
    crate::clipboard::copy_to_clipboard_auto(&final_command)?;
    if let Err(e) = crate::logging::audit_log("copy", snippet, None) {
        tracing::debug!("Audit log write failed: {}", e);
    }
    let mut usage_idx = crate::usage::UsageIndex::load();
    usage_idx.record_use(&snippet.id);
    let _ = usage_idx.save();
    if do_sync {
        let rt = runtime.expect("run_exact: runtime required when do_sync is true");
        if let Err(e) = crate::commands::run_explicit_sync(rt) {
            tracing::warn!(error = %e, "post-clip explicit sync failed");
        }
    }
    Ok(())
}

/// Copies the selected snippet's expanded command to the clipboard.
pub fn run(
    filter: Option<String>,
    do_sync: bool,
    library: Option<String>,
    _config: Option<PathBuf>,
    sort_opts: Option<crate::sort::SortOptions>,
    runtime: Option<&tokio::runtime::Runtime>,
) -> SnipResult<()> {
    let _outcome = run_snippet_selection(
        filter,
        library,
        do_sync,
        sort_opts,
        runtime,
        process_snippet,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::library::Snippet;

    #[test]
    #[ignore = "requires live clipboard — tested via integration tests"]
    fn test_clip_run_exact_accepts_do_sync_and_runtime_params() {
        let snippet = Snippet {
            id: "test-clip-sync".to_string(),
            description: "test clip sync".to_string(),
            command: "echo hello".to_string(),
            ..Default::default()
        };
        // do_sync=false, runtime=None should succeed without attempting sync
        let result = super::run_exact(&snippet, false, None);
        assert!(result.is_ok());
    }

    #[test]
    #[ignore = "requires live clipboard — tested via integration tests"]
    fn test_clip_run_exact_without_sync_does_not_require_runtime() {
        let snippet = Snippet {
            id: "test-clip-no-sync".to_string(),
            description: "test clip no sync".to_string(),
            command: "echo world".to_string(),
            ..Default::default()
        };
        let result = super::run_exact(&snippet, false, None);
        assert!(result.is_ok(), "clip run_exact without sync should succeed");
    }
}
