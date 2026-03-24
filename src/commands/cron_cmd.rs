use crate::error::SnipResult;
use std::io::{self, Write};

pub fn run(interval: u32) -> SnipResult<()> {
    let binary_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "snp".to_string());

    let cron_entry = format!(
        "*/{} * * * * {} sync --non-interactive",
        interval, binary_path
    );

    println!("Crontab entry (every {} minutes):", interval);
    println!("{}", cron_entry);
    println!();

    #[cfg(target_os = "macos")]
    {
        println!("To add to your crontab:");
        println!("  1. Run: crontab -e");
        println!("  2. Add the line above");
        println!("  3. Save and exit");
    }

    #[cfg(target_os = "linux")]
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
        #[cfg(target_os = "macos")]
        {
            let mut child = std::process::Command::new("pbcopy")
                .spawn()
                .expect("Failed to spawn pbcopy");
            child
                .stdin
                .as_ref()
                .unwrap()
                .write_all(cron_entry.as_bytes())
                .ok();
            child.wait().ok();
        }
        #[cfg(target_os = "linux")]
        {
            let mut child = std::process::Command::new("xclip")
                .args(["-selection", "clipboard"])
                .spawn()
                .expect("Failed to spawn xclip");
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(cron_entry.as_bytes())
                .ok();
            child.wait().ok();
        }
        #[cfg(target_os = "windows")]
        {
            let mut child = std::process::Command::new("cmd")
                .args(["/C", "clip"])
                .spawn()
                .expect("Failed to spawn clip");
            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(cron_entry.as_bytes())
                .ok();
            child.wait().ok();
        }
        println!("Copied to clipboard!");
    }
    Ok(())
}
