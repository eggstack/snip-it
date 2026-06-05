use crate::error::{SnipError, SnipResult};
use std::io::{self, Write};

/// Displays a crontab entry for periodic sync at the given interval (in minutes).
pub fn run(interval: u32) -> SnipResult<()> {
    if interval == 0 {
        return Err(SnipError::runtime_error(
            "Invalid interval",
            Some("Interval must be at least 1 minute"),
        ));
    }

    let binary_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "snp".to_string());

    let cron_entry = format!(
        "*/{} * * * * '{}' sync --non-interactive",
        interval, binary_path
    );

    println!("Crontab entry (every {} minutes):", interval);
    println!("{}", cron_entry);
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
        println!("  6. Arguments: sync --non-interactive");
    }

    println!();
    print!("Copy to clipboard? [y/N]: ");
    io::stdout().flush().ok();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() && input.trim().to_lowercase() == "y" {
        match crate::clipboard::copy_to_clipboard_auto(&cron_entry) {
            Ok(()) => println!("Copied to clipboard!"),
            Err(e) => eprintln!("Failed to copy to clipboard: {}", e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_interval_zero_invalid() {
        let result = run(0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("Interval must be at least 1 minute")
        );
    }

    #[test]
    fn test_run_interval_valid() {
        let result = run(30);
        assert!(result.is_ok());
    }
}
