use crate::error::{SnipError, SnipResult};
use std::io::{self, Write};

fn shell_escape_path(path: &str) -> String {
    if path.is_empty() {
        return "''".to_string();
    }
    let needs_escape = path
        .chars()
        .any(|c| c == ' ' || c == '\'' || c == '"' || c == '\\' || c == '$' || c == '`');
    if !needs_escape {
        return path.to_string();
    }
    // Wrap in single quotes, escaping any existing single quotes
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// Resolves the executable path for the running `snp` binary, falling back to
/// the bare name `snp` if the path cannot be determined.
fn binary_path() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "snp".to_string())
}

/// Validates the interval and builds the crontab entry string.
///
/// This is the pure portion of [`run`]: it performs no blocking I/O (in
/// particular, no `stdin` reads) so it can be unit tested directly.
pub fn make_cron_entry(interval: u32) -> SnipResult<String> {
    if interval == 0 {
        return Err(SnipError::runtime_error(
            "Invalid interval",
            Some("Interval must be at least 1 minute"),
        ));
    }

    Ok(format!(
        "*/{} * * * * {} sync",
        interval,
        shell_escape_path(&binary_path())
    ))
}

/// Displays a crontab entry for periodic sync at the given interval (in minutes).
pub fn run(interval: u32) -> SnipResult<()> {
    // The resolved `binary_path` is only surfaced in the Windows instructions
    // (the crontab entry itself is built by `make_cron_entry`); gate the binding
    // so it is not flagged as unused on Unix.
    #[cfg(target_os = "windows")]
    let binary_path = binary_path();
    let cron_entry = make_cron_entry(interval)?;

    println!("Crontab entry (every {interval} minutes):");
    println!("{cron_entry}");
    println!();

    #[cfg(not(target_os = "windows"))]
    {
        println!("To add to your crontab:");
        println!("  1. Run: crontab -e");
        println!("  2. Add the line above");
        println!("  3. Save and exit");
    }

    #[cfg(target_os = "windows")]
    {
        println!("On Windows, use Task Scheduler instead:");
        println!("  1. Open Task Scheduler (taskschd.msc)");
        println!("  2. Create Basic Task");
        println!("  3. Set trigger: Daily, repeat every {} minutes", interval);
        println!("  4. Action: Start a program");
        println!("  5. Program: {}", binary_path);
        println!("  6. Arguments: sync");
    }

    println!();
    print!("Copy to clipboard? [y/N]: ");
    io::stdout().flush().ok();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() && input.trim().to_lowercase() == "y" {
        match crate::clipboard::copy_to_clipboard_auto(&cron_entry) {
            Ok(()) => println!("Copied to clipboard!"),
            Err(e) => eprintln!("Failed to copy to clipboard: {e}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_interval_zero_invalid() {
        let result = make_cron_entry(0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Interval must be at least 1 minute")
        );
    }

    #[test]
    fn test_run_interval_valid() {
        let result = make_cron_entry(30);
        assert!(result.is_ok());
    }

    #[test]
    fn test_shell_escape_path_empty() {
        assert_eq!(shell_escape_path(""), "''");
    }

    #[test]
    fn test_shell_escape_path_simple() {
        assert_eq!(shell_escape_path("/usr/bin/snp"), "/usr/bin/snp");
    }

    #[test]
    fn test_shell_escape_path_with_spaces() {
        assert_eq!(
            shell_escape_path("/usr/local/bin/my app"),
            "'/usr/local/bin/my app'"
        );
    }

    #[test]
    fn test_shell_escape_path_with_single_quotes() {
        // single quotes inside single-quoted strings are escaped as '\''
        assert_eq!(
            shell_escape_path("/path/with'quote"),
            "'/path/with'\\''quote'"
        );
    }

    #[test]
    fn test_shell_escape_path_with_dollar() {
        assert_eq!(shell_escape_path("/path/$HOME"), "'/path/$HOME'");
    }

    #[test]
    fn test_shell_escape_path_with_backtick() {
        assert_eq!(shell_escape_path("/path/`cmd`"), "'/path/`cmd`'");
    }

    #[test]
    fn test_shell_escape_path_with_backslash() {
        assert_eq!(
            shell_escape_path("/path\\with\\backslash"),
            "'/path\\with\\backslash'"
        );
    }
}
