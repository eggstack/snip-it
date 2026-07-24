use crate::commands::expand_snippet_command;
use crate::commands::run_snippet_selection;
use crate::error::{SnipError, SnipResult};
use crate::library::Snippet;
use crate::logging::{audit_log, log_command_execution};
use std::fs;
use std::process::Command;
use std::time::Duration;

const DEFAULT_TIMEOUT_SECONDS: u64 = 300;

#[derive(Clone, Copy)]
enum TimeoutPolicy {
    NoDefault,
    Default(Duration),
}

fn get_timeout(policy: TimeoutPolicy) -> Option<Duration> {
    if let Some(secs) = std::env::var("SNP_COMMAND_TIMEOUT")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
    {
        return (secs > 0).then(|| Duration::from_secs(secs));
    }

    match policy {
        TimeoutPolicy::NoDefault => None,
        TimeoutPolicy::Default(timeout) => Some(timeout),
    }
}

fn wait_for_command(
    child: &mut std::process::Child,
    timeout: Option<Duration>,
) -> SnipResult<std::process::ExitStatus> {
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

        if let Some(timeout) = timeout
            && start.elapsed() >= timeout
        {
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

fn shell_arg_flag() -> &'static str {
    if cfg!(windows) { "/C" } else { "-c" }
}

fn get_shell() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

/// Spawn a shell command and wait for it, returning a unified `ProcessResult`.
///
/// This helper is used by both the output-file and ordinary execution branches
/// to ensure consistent outcome mapping. It returns:
/// - `Done` for success (exit code 0);
/// - `Failed { exit_code: Some(code) }` for normal child nonzero exit;
/// - `Failed { exit_code: None }` for spawn failure, timeout, or
///   signal/no-code termination (maps to execution-failure exit code 8).
fn spawn_and_wait_execution(
    shell: &str,
    command: &str,
    timeout: Option<Duration>,
    stdout_file: Option<std::fs::File>,
) -> crate::ProcessResult {
    let mut cmd = Command::new(shell);
    cmd.arg(shell_arg_flag()).arg(command);
    if let Some(file) = stdout_file {
        cmd.stdout(file);
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            return crate::ProcessResult::Failed {
                exit_code: None,
                message: format!("Failed to spawn shell: {e}"),
            };
        }
    };

    match wait_for_command(&mut child, timeout) {
        Ok(status) => {
            if status.success() {
                crate::ProcessResult::Done("Executed".to_string())
            } else {
                crate::ProcessResult::Failed {
                    exit_code: status.code(),
                    message: format!("Executed with exit code: {status}"),
                }
            }
        }
        Err(e) => {
            // Timeout or wait failure: no child exit code.
            crate::ProcessResult::Failed {
                exit_code: None,
                message: e.to_string(),
            }
        }
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
        let timeout = get_timeout(TimeoutPolicy::Default(Duration::from_secs(
            DEFAULT_TIMEOUT_SECONDS,
        )));

        // Use unified spawn_and_wait_execution so spawn failures map to
        // ProcessResult::Failed { exit_code: None } (exit code 8), not
        // generic SnipError (exit code 1).
        let result = spawn_and_wait_execution(
            &shell,
            &final_command,
            timeout,
            Some(output_file),
        );

        // Record audit/usage on success
        if result.is_done() {
            if let Err(e) = audit_log("execute", snippet, None) {
                tracing::debug!("Audit log write failed: {}", e);
            }
            let mut usage_idx = crate::usage::UsageIndex::load();
            usage_idx.record_use(&snippet.id);
            if let Err(e) = usage_idx.save() {
                tracing::debug!("Usage save failed: {}", e);
            }
        }

        let result_str: Result<(), String> = if result.is_done() {
            Ok(())
        } else {
            Err(format!("{:?}", result))
        };
        log_command_execution(&final_command, &[], &result_str, Some(&cwd));

        Ok(result)
    } else {
        let shell = get_shell();
        let timeout = get_timeout(TimeoutPolicy::NoDefault);

        // Use unified spawn_and_wait_execution for consistent outcome mapping.
        let result = spawn_and_wait_execution(&shell, &final_command, timeout, None);

        // Record audit/usage on success
        if result.is_done() {
            if let Err(e) = audit_log("execute", snippet, None) {
                tracing::debug!("Audit log write failed: {}", e);
            }
            let mut usage_idx = crate::usage::UsageIndex::load();
            usage_idx.record_use(&snippet.id);
            if let Err(e) = usage_idx.save() {
                tracing::debug!("Usage save failed: {}", e);
            }
        }

        let result_str: Result<(), String> = if result.is_done() {
            Ok(())
        } else {
            Err(format!("{:?}", result))
        };
        log_command_execution(&final_command, &[], &result_str, None);

        Ok(result)
    }
}

/// Executes the selected snippet's command in the user's shell.
pub fn run(
    filter: Option<String>,
    do_sync: bool,
    library: Option<String>,
    sort_opts: Option<crate::sort::SortOptions>,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<crate::CommandOutcome> {
    let outcome = run_snippet_selection(
        filter,
        library,
        do_sync,
        sort_opts,
        runtime,
        |snippet, copy_flag| process_snippet(snippet, copy_flag.is_some()),
    )?;
    match outcome {
        crate::SelectionOutcome::ExecutionFailed { exit_code } => {
            Ok(crate::CommandOutcome::ExecutionFailed {
                child_code: exit_code,
            })
        }
        // run treats cancellation as exit 0 (per documented contract)
        crate::SelectionOutcome::Cancelled => Ok(crate::CommandOutcome::Success),
        crate::SelectionOutcome::Selected => Ok(crate::CommandOutcome::Success),
    }
}

/// Execute a specific snippet directly, bypassing TUI selection.
pub fn run_exact(
    snippet: &Snippet,
    do_sync: bool,
    _runtime: &tokio::runtime::Runtime,
) -> SnipResult<crate::CommandOutcome> {
    let result = process_snippet(snippet, false)?;
    if let crate::ProcessResult::Failed { exit_code, .. } = result {
        return Ok(crate::CommandOutcome::ExecutionFailed {
            child_code: exit_code,
        });
    }
    if do_sync {
        crate::auto_sync::notify_mutation(
            crate::auto_sync::MutationKind::SnippetRun,
            crate::auto_sync::MutationOrigin::User,
        );
    }
    Ok(crate::CommandOutcome::Success)
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

    #[test]
    fn test_shell_arg_flag_matches_platform() {
        let flag = shell_arg_flag();
        if cfg!(windows) {
            assert_eq!(flag, "/C", "cmd.exe expects /C, not the unix-style {flag}");
        } else {
            assert_eq!(flag, "-c", "Unix shells expect -c, not {flag}");
        }
    }
}
