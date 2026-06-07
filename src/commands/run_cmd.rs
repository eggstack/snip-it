use crate::commands::expand_snippet_command;
use crate::commands::run_snippet_selection;
use crate::error::{SnipError, SnipResult};
use crate::library::Snippet;
use crate::logging::{audit_log, log_command_execution};
use std::fs;
use std::process::Command;
use std::time::Duration;

const DEFAULT_TIMEOUT_SECONDS: u64 = 300;

fn get_timeout() -> Duration {
    let secs = std::env::var("SNP_COMMAND_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|v| v.max(1))
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    Duration::from_secs(secs)
}

fn run_command_with_timeout(
    shell: &str,
    command: &str,
    timeout: Duration,
) -> SnipResult<std::process::ExitStatus> {
    let mut child = Command::new(shell)
        .arg("-c")
        .arg(command)
        .spawn()
        .map_err(|e| SnipError::command_error(shell, vec![command.to_string()], e))?;

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(e) => {
                return Err(SnipError::runtime_error(
                    "Failed to check command status",
                    Some(&e.to_string()),
                ));
            }
        }

        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SnipError::runtime_error(
                "Command timed out",
                Some(&format!(
                    "Command exceeded timeout of {} seconds",
                    timeout.as_secs()
                )),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn get_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn handle_command_result(
    command: &str,
    result: std::process::ExitStatus,
    snippet: &Snippet,
    working_dir: Option<&std::path::Path>,
) -> crate::ProcessResult {
    let result_str: Result<(), String> = if result.success() {
        if let Err(e) = audit_log("execute", snippet, None) {
            tracing::debug!("Audit log write failed: {}", e);
        }
        Ok(())
    } else {
        Err(format!("exit code: {result}"))
    };
    log_command_execution(command, &[], &result_str, working_dir);

    if result.success() {
        crate::ProcessResult::Done("Executed".to_string())
    } else {
        crate::ProcessResult::Done(format!("Executed with exit code: {result}"))
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
        if let Err(e) = audit_log("copy", snippet, None) {
            tracing::debug!("Audit log write failed: {}", e);
        }
        let ok_result: std::result::Result<(), String> = Ok(());
        log_command_execution(&final_command, &[], &ok_result, None);
        Ok(crate::ProcessResult::Done(
            "Copied to clipboard".to_string(),
        ))
    } else if !snippet.output.is_empty() {
        let cwd = std::env::current_dir().map_err(|e| {
            SnipError::runtime_error(
                "Failed to get current directory",
                Some(&format!("Cannot create output file: {e}")),
            )
        })?;

        let output_path = cwd.join(&snippet.output);

        let canonical_cwd = cwd.canonicalize().map_err(|e| {
            SnipError::runtime_error(
                "Failed to verify current directory",
                Some(&format!("Cannot canonicalize CWD: {e}")),
            )
        })?;

        if output_path.exists() {
            let canonical_path = output_path.canonicalize().map_err(|e| {
                SnipError::runtime_error(
                    "Failed to verify output path",
                    Some(&format!("Cannot canonicalize output path: {e}")),
                )
            })?;
            if !canonical_path.starts_with(&canonical_cwd) {
                return Err(SnipError::runtime_error(
                    "Invalid output path",
                    Some(
                        "Output path resolves outside of working directory (possible symlink attack)",
                    ),
                ));
            }
        } else {
            let parent = output_path
                .parent()
                .unwrap_or(&output_path)
                .canonicalize()
                .map_err(|e| {
                    SnipError::runtime_error(
                        "Failed to verify output directory",
                        Some(&format!("Cannot canonicalize parent directory: {e}")),
                    )
                })?;
            if !parent.starts_with(&canonical_cwd) {
                return Err(SnipError::runtime_error(
                    "Invalid output path",
                    Some(
                        "Output path resolves outside of working directory (possible symlink attack)",
                    ),
                ));
            }
        }

        let output_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output_path)
            .map_err(|e| SnipError::io_error("create output file", snippet.output.clone(), e))?;

        let shell = get_shell();
        let timeout = get_timeout();
        let mut child = Command::new(&shell)
            .arg("-c")
            .arg(&final_command)
            .stdout(output_file)
            .spawn()
            .map_err(|e| SnipError::command_error(&shell, vec![final_command.clone()], e))?;

        let start = std::time::Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(SnipError::runtime_error(
                            "Command timed out",
                            Some(&format!(
                                "Command exceeded timeout of {} seconds",
                                timeout.as_secs()
                            )),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    return Err(SnipError::runtime_error(
                        "Failed to check command status",
                        Some(&e.to_string()),
                    ));
                }
            }
        };

        Ok(handle_command_result(
            &final_command,
            status,
            snippet,
            Some(&cwd),
        ))
    } else {
        let shell = get_shell();
        let timeout = get_timeout();
        let status = run_command_with_timeout(&shell, &final_command, timeout)?;

        Ok(handle_command_result(&final_command, status, snippet, None))
    }
}

/// Executes the selected snippet's command in the user's shell.
pub fn run(
    filter: Option<String>,
    do_sync: bool,
    library: Option<String>,
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
        if cfg!(windows) {
            assert!(
                shell.ends_with("cmd.exe") || shell.ends_with("CMD.EXE"),
                "Expected cmd.exe on Windows, got: {shell}"
            );
        } else {
            assert!(
                shell.contains("/bin/sh") || std::env::var("SHELL").is_ok(),
                "Expected /bin/sh or $SHELL on Unix, got: {shell}"
            );
        }
    }
}
