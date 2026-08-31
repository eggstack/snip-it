use crate::commands::expand_snippet_command;
use crate::commands::run_snippet_selection;
use crate::error::SnipResult;
use crate::library::Snippet;
use std::path::PathBuf;

/// Copy the expanded command of a snippet to the clipboard, record the
/// audit log entry, and update the usage index.
///
/// This is the single implementation for all clipboard copy operations
/// (TUI callback, exact command path, and `snp run --copy`). Variable
/// expansion is *not* performed here — the caller expands first and
/// passes the result in.
pub(crate) fn copy_to_clipboard(snippet: &Snippet, final_command: &str) -> SnipResult<()> {
    crate::clipboard::copy_to_clipboard_auto(final_command)?;
    crate::logging::audit_log("copy", snippet, None)?;
    let mut usage_idx = crate::usage::UsageIndex::load();
    usage_idx.record_use(&snippet.id);
    if let Err(e) = usage_idx.save() {
        tracing::debug!("Usage save failed: {}", e);
    }
    Ok(())
}

fn process_snippet(
    snippet: &Snippet,
    _copy_flag: Option<String>,
) -> SnipResult<crate::ProcessResult> {
    let final_command = match expand_snippet_command(snippet)? {
        crate::commands::ExpandedCommand::Cancel => return Ok(crate::ProcessResult::Cancel),
        crate::commands::ExpandedCommand::Skip => return Ok(crate::ProcessResult::Continue),
        crate::commands::ExpandedCommand::Expanded(cmd) => cmd,
    };

    copy_to_clipboard(snippet, &final_command)?;
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
    let runtime = if do_sync {
        Some(runtime.ok_or_else(|| {
            crate::error::SnipError::runtime_error(
                "sync requested but no runtime",
                Some("run_exact called with do_sync=true and runtime=None"),
            )
        })?)
    } else {
        None
    };
    let final_command = match expand_snippet_command(snippet)? {
        crate::commands::ExpandedCommand::Cancel => return Ok(()),
        crate::commands::ExpandedCommand::Skip => return Ok(()),
        crate::commands::ExpandedCommand::Expanded(cmd) => cmd,
    };
    copy_to_clipboard(snippet, &final_command)?;
    if do_sync
        && let Some(rt) = runtime
        && let Err(e) = crate::commands::run_explicit_sync(rt)
    {
        tracing::warn!(error = %e, "post-clip explicit sync failed");
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
        true,
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

    #[test]
    fn test_clip_run_exact_with_sync_requires_runtime() {
        let snippet = Snippet {
            command: "echo hello".to_string(),
            ..Default::default()
        };
        let result = super::run_exact(&snippet, true, None);
        assert!(result.unwrap_err().to_string().contains("no runtime"));
    }
}
