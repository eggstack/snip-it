use crate::commands::expand_snippet_command;
use crate::commands::run_snippet_selection;
use crate::error::{SnipError, SnipResult};
use crate::library::Snippet;
use crate::logging::{audit_log, log_command_execution};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn get_shell() -> String {
    "/bin/sh".to_string()
}

fn handle_command_result(
    command: &str,
    result: std::process::ExitStatus,
    snippet: &Snippet,
) -> crate::ProcessResult {
    let result_str: Result<(), String> = if result.success() {
        if let Err(e) = audit_log("execute", snippet) {
            tracing::debug!("Audit log write failed: {}", e);
        }
        Ok(())
    } else {
        Err(format!("exit code: {}", result))
    };
    log_command_execution(command, &[], &result_str);

    if result.success() {
        crate::ProcessResult::Done("Executed".to_string())
    } else {
        crate::ProcessResult::Done(format!("Executed with exit code: {}", result))
    }
}

fn process_snippet(snippet: &Snippet, copy: bool) -> SnipResult<crate::ProcessResult> {
    let final_command = match expand_snippet_command(snippet)? {
        crate::commands::ExpandedCommand::Cancel => return Ok(crate::ProcessResult::Cancel),
        crate::commands::ExpandedCommand::Skip => return Ok(crate::ProcessResult::Continue),
        crate::commands::ExpandedCommand::Expanded(cmd) => cmd,
    };

    if copy {
        crate::clipboard::copy_to_clipboard_auto(&final_command)?;
        if let Err(e) = audit_log("copy", snippet) {
            tracing::debug!("Audit log write failed: {}", e);
        }
        let ok_result: std::result::Result<(), String> = Ok(());
        log_command_execution(&final_command, &[], &ok_result);
        Ok(crate::ProcessResult::Done(
            "Copied to clipboard".to_string(),
        ))
    } else if !snippet.output.is_empty() {
        let cwd = std::env::current_dir().map_err(|e| {
            SnipError::runtime_error(
                "Failed to get current directory",
                Some(&format!("Cannot create output file: {}", e)),
            )
        })?;

        let output_path = cwd.join(&snippet.output);

        let output_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output_path)
            .map_err(|e| SnipError::io_error("create output file", snippet.output.clone(), e))?;

        let canonical_path = output_path.canonicalize().map_err(|e| {
            SnipError::runtime_error(
                "Failed to verify output path",
                Some(&format!("Cannot canonicalize path: {}", e)),
            )
        })?;

        let canonical_cwd = cwd.canonicalize().map_err(|e| {
            SnipError::runtime_error(
                "Failed to verify current directory",
                Some(&format!("Cannot canonicalize CWD: {}", e)),
            )
        })?;

        if !canonical_path.starts_with(&canonical_cwd) {
            return Err(SnipError::runtime_error(
                "Invalid output path",
                Some("Output path resolves outside of working directory after creation (possible symlink attack)"),
            ));
        }

        let shell = get_shell();
        let output = Command::new(&shell)
            .arg("-c")
            .arg(&final_command)
            .stdout(output_file)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.is_empty() {
                eprintln!("Error: {}", stderr);
            }
        }

        Ok(handle_command_result(
            &final_command,
            output.status,
            snippet,
        ))
    } else {
        let shell = get_shell();
        let output = Command::new(&shell)
            .arg("-c")
            .arg(&final_command)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.is_empty() {
                eprintln!("Error: {}", stderr);
            }
        }

        Ok(handle_command_result(
            &final_command,
            output.status,
            snippet,
        ))
    }
}

pub fn run(
    filter: Option<String>,
    do_sync: bool,
    library: Option<String>,
    _config: Option<PathBuf>,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()> {
    run_snippet_selection(filter, library, do_sync, runtime, |snippet, copy_flag| {
        process_snippet(snippet, copy_flag.is_some())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_shell() {
        let shell = get_shell();
        assert_eq!(shell, "/bin/sh");
    }
}
