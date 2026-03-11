use crate::commands::{get_library_path, get_snippet_data};
use crate::error::{SnipError, SnipResult};
use crate::library::Snippet;
use std::fs;
use std::io::{self};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn process_snippet(snippet: &Snippet, copy: bool) -> SnipResult<crate::ProcessResult> {
    use crate::logging::log_command_execution;
    use crate::ui;
    use crate::utils::parse_variables;

    let vars = parse_variables(&snippet.command);
    let final_command = if vars.is_empty() {
        snippet.command.clone()
    } else {
        match ui::prompt_variables(vars)? {
            None => return Ok(crate::ProcessResult::Cancel),
            Some(None) => return Ok(crate::ProcessResult::Continue),
            Some(Some(values)) => ui::expand_command(&snippet.command, &values),
        }
    };

    if copy {
        crate::clipboard::copy_to_clipboard(&final_command)?;
        let _ = audit_log("copy", snippet);
        let ok_result: std::result::Result<(), String> = Ok(());
        log_command_execution(&final_command, &[], &ok_result);
        Ok(crate::ProcessResult::Done(
            "Copied to clipboard".to_string(),
        ))
    } else if !snippet.output.is_empty() {
        let output_file = fs::File::create(&snippet.output)
            .map_err(|e| SnipError::io_error("create output file", snippet.output.clone(), e))?;

        let status = Command::new("sh")
            .arg("-c")
            .arg(&final_command)
            .stdout(output_file)
            .status();

        match status {
            Ok(s) if s.success() => {
                let _ = audit_log("execute", snippet);
                let ok_result: std::result::Result<(), String> = Ok(());
                log_command_execution(&final_command, &[], &ok_result);
                Ok(crate::ProcessResult::Done("Executed".to_string()))
            }
            Ok(s) => {
                let err_result: std::result::Result<(), String> = Err("non-zero exit".to_string());
                log_command_execution(&final_command, &[], &err_result);
                Ok(crate::ProcessResult::Done(format!(
                    "Executed with exit code: {}",
                    s
                )))
            }
            Err(e) => {
                let err_result: std::result::Result<(), String> = Err(e.to_string());
                log_command_execution(&final_command, &[], &err_result);
                Err(SnipError::command_error(&final_command, vec![], e))
            }
        }
    } else {
        let status = Command::new("sh").arg("-c").arg(&final_command).status();

        match status {
            Ok(s) if s.success() => {
                let _ = audit_log("execute", snippet);
                let ok_result: std::result::Result<(), String> = Ok(());
                log_command_execution(&final_command, &[], &ok_result);
                Ok(crate::ProcessResult::Done("Executed".to_string()))
            }
            Ok(s) => {
                let err_result: std::result::Result<(), String> = Err("non-zero exit".to_string());
                log_command_execution(&final_command, &[], &err_result);
                Ok(crate::ProcessResult::Done(format!(
                    "Executed with exit code: {}",
                    s
                )))
            }
            Err(e) => {
                let err_result: std::result::Result<(), String> = Err(e.to_string());
                log_command_execution(&final_command, &[], &err_result);
                Err(SnipError::command_error(&final_command, vec![], e))
            }
        }
    }
}

fn audit_log(action: &str, snippet: &Snippet) -> io::Result<()> {
    let cfg_dir = std::env::var("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from(".local/share"))
                .join("snp")
        });

    std::fs::create_dir_all(&cfg_dir)?;
    let log_path = cfg_dir.join("audit.log");

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let log_entry = format!(
        "{}|{}|{}|{}|{}\n",
        timestamp, action, snippet.description, snippet.command, snippet.output
    );

    fs::write(log_path, log_entry)?;
    Ok(())
}

pub fn run(
    filter: Option<String>,
    do_sync: bool,
    library: Option<String>,
    _config: Option<PathBuf>,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()> {
    let lib_path = match get_library_path(library.clone())? {
        Some(p) => p,
        None => {
            eprintln!("No library found. Create one with 'snp library create <name>'");
            return Ok(());
        }
    };
    let snippets = crate::library::load_library(&lib_path)?;
    let (descriptions, commands, tags, folders, favorites) = get_snippet_data(&snippets);

    loop {
        let result = crate::ui::select_snippet(
            &descriptions,
            &commands,
            &tags,
            false,
            filter.as_deref(),
            &folders,
            &favorites,
        )?;
        if let Some((idx, copy)) = result {
            let snippet = &snippets.snippets[idx];
            match process_snippet(snippet, copy.is_some())? {
                crate::ProcessResult::Cancel => {
                    if do_sync {
                        crate::sync_commands::run_sync(
                            &crate::config::SyncSettings::default(),
                            None,
                            false,
                            false,
                            false,
                            runtime,
                        );
                    }
                    return Ok(());
                }
                crate::ProcessResult::Continue => continue,
                crate::ProcessResult::Done(_msg) => {
                    break;
                }
            }
        } else {
            break;
        }
    }
    if do_sync {
        crate::sync_commands::run_sync(
            &crate::config::SyncSettings::default(),
            None,
            false,
            false,
            false,
            runtime,
        );
    }
    Ok(())
}
