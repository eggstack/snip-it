use crate::CommandOutcome;
use crate::SelectionOutcome;
use crate::commands::run_snippet_selection;
use crate::error::SnipResult;
use crate::library::Snippet;
use std::cell::Cell;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq)]
enum OutputMode {
    Raw,
    Expanded,
}

/// Writes `content` to `target` via a private temp file in the same directory
/// and atomically renames it into place.
///
/// This avoids two distinct failure modes:
/// - A pre-existing caller-owned file is never deleted or truncated by this
///   command; the temp file is freshly created with `O_EXCL` and any failure
///   before the final rename removes only that temp file.
/// - A racing symlink swap at `target` cannot redirect the write, because the
///   write happens to a fresh, `create_new`-created temp file (which fails on
///   a symlink at the temp path) and the final `rename` replaces a symlink at
///   `target` rather than following it.
fn write_selection_atomically(target: &Path, content: &[u8]) -> SnipResult<()> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|e| {
        crate::error::SnipError::io_error("create output file parent", parent.to_path_buf(), e)
    })?;

    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp_path = parent.join(format!(".snp-select-{pid}-{nanos}.tmp"));

    let write_result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .truncate(false)
            .open(&temp_path)?;
        file.write_all(content)?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(crate::error::SnipError::io_error(
            "write selection to temp file",
            temp_path,
            e,
        ));
    }

    if let Err(e) = std::fs::rename(&temp_path, target) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(crate::error::SnipError::io_error(
            "install selection to target",
            target.to_path_buf(),
            e,
        ));
    }
    Ok(())
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
///
/// The output file is written via an atomic temp-file rename so a
/// pre-existing caller-owned file is preserved if the selection is
/// cancelled, and a racing symlink at the target cannot redirect the write.
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
        (SelectionOutcome::Cancelled, _, _) | (_, true, _) => Ok(CommandOutcome::Cancelled),
        (SelectionOutcome::ExecutionFailed { .. }, _, _) => {
            Err(crate::error::SnipError::runtime_error(
                "Internal contract error",
                Some("select command should never produce an execution failure"),
            ))
        }
        (SelectionOutcome::Selected, false, Some(command)) => {
            if let Some(path) = output_file {
                write_selection_atomically(&path, command.as_bytes())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_write_selection_creates_file_with_content() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("out.txt");
        write_selection_atomically(&target, b"selected-command").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"selected-command");
    }

    #[test]
    fn test_write_selection_overwrites_existing_file() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("out.txt");
        std::fs::write(&target, "old content").unwrap();
        write_selection_atomically(&target, b"new content").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new content");
    }

    #[test]
    #[cfg(unix)]
    fn test_write_selection_replaces_symlink_with_regular_file() {
        // A racing symlink at the target path must not redirect the write.
        // rename(2) does not follow symlinks at the destination, so the
        // symlink itself is replaced by our newly-written regular file.
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("out.txt");
        let real = dir.path().join("real.txt");
        std::fs::write(&real, b"sensitive data").unwrap();
        std::os::unix::fs::symlink(&real, &target).unwrap();

        write_selection_atomically(&target, b"new content").unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new content");
        assert_eq!(
            std::fs::read(&real).unwrap(),
            b"sensitive data",
            "the symlink target must not be modified"
        );
        assert!(
            !target.is_symlink(),
            "target must be a regular file after the write, not a symlink"
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_write_selection_does_not_follow_symlink_at_target() {
        // If the target is a symlink to a file, rename(2) replaces the
        // symlink itself rather than following it. Verify the destination
        // file's content is preserved and only the symlink is replaced.
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("link.txt");
        let real = dir.path().join("real.txt");
        std::fs::write(&real, b"preserved-target-content").unwrap();
        std::os::unix::fs::symlink(&real, &target).unwrap();

        write_selection_atomically(&target, b"selection").unwrap();

        // The target symlink is gone (replaced by a regular file).
        assert!(!target.is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), b"selection");
        // The destination the symlink used to point at is untouched.
        assert_eq!(std::fs::read(&real).unwrap(), b"preserved-target-content");
    }

    #[test]
    #[cfg(unix)]
    fn test_write_selection_creates_parent_dirs() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("nested").join("sub").join("out.txt");
        write_selection_atomically(&target, b"content").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"content");
    }
}
