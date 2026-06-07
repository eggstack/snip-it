//! Logging infrastructure using the `tracing` crate.
//!
//! Provides structured logging with file rotation and panic handling.
//!
//! # Log Levels
//!
//! - `trace`: Very detailed diagnostics
//! - `debug`: Debug information
//! - `info`: General information (default)
//! - `warn`: Warning messages
//! - `error`: Error messages
//!
//! # Log Locations
//!
//! - All platforms: `~/.config/snp/logs/`
//!   (or `$XDG_CONFIG_HOME/snp/logs/` if set)

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

const AUDIT_LOG_MAX_SIZE_BYTES: u64 = 10 * 1024 * 1024; // 10 MB — rotate audit log when exceeded
const AUDIT_LOG_RETENTION_DAYS: u64 = 30; // Keep 30 days of rotated audit logs
const AUDIT_LOG_CHANNEL_SIZE: usize = 1024; // Bounded channel for async audit writes
const SECS_PER_DAY: u64 = 86400;

struct AuditLogEntry {
    timestamp: u64,
    action: String,
    snippet_id: String,
    description: String,
    library_id: String,
    device_id: String,
}

static AUDIT_TX: LazyLock<std::sync::Mutex<Option<mpsc::SyncSender<AuditLogEntry>>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));

/// Holds the non-blocking log writer guard alive for the process lifetime.
/// Dropping this would stop log file writes.
static LOG_GUARD: LazyLock<std::sync::Mutex<Option<WorkerGuard>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));

/// Configuration for logging behavior.
#[allow(dead_code)]
pub struct LogConfig {
    pub log_dir: PathBuf,
    pub file_name: String,
    pub level: Level,
    pub include_target: bool,
    pub audit_max_size_bytes: u64,
    pub audit_retention_days: u64,
}

impl Default for LogConfig {
    fn default() -> Self {
        LogConfig {
            log_dir: get_default_log_dir(),
            file_name: "snp.log".to_string(),
            level: Level::INFO,
            include_target: true,
            audit_max_size_bytes: AUDIT_LOG_MAX_SIZE_BYTES,
            audit_retention_days: AUDIT_LOG_RETENTION_DAYS,
        }
    }
}

pub fn get_default_log_dir() -> PathBuf {
    crate::utils::config::get_config_dir().join("logs")
}

fn level_str(level: Level) -> &'static str {
    match level {
        Level::ERROR => "error",
        Level::WARN => "warn",
        Level::INFO => "info",
        Level::DEBUG => "debug",
        Level::TRACE => "trace",
    }
}

pub fn init_logging(config: &LogConfig) -> Result<(), Box<dyn std::error::Error>> {
    let log_dir = &config.log_dir;

    fs::create_dir_all(log_dir)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o700);
        fs::set_permissions(log_dir, perms)?;
    }

    let file_appender = tracing_appender::rolling::daily(log_dir, &config.file_name);

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = if let Ok(filter_str) = std::env::var("SNP_LOG") {
        EnvFilter::try_new(&filter_str).unwrap_or_else(|e| {
            eprintln!("Warning: Invalid SNP_LOG filter '{filter_str}': {e}");
            EnvFilter::new(format!("snp={}", level_str(config.level)))
        })
    } else {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(format!("snp={}", level_str(config.level))))
    };

    let file_layer = fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(config.include_target)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true);

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer);

    let _ = subscriber.try_init();

    *LOG_GUARD.lock().unwrap_or_else(|e| e.into_inner()) = Some(guard);

    init_async_audit_log();

    tracing::info!("Logging initialized. Log directory: {}", log_dir.display());
    tracing::info!("Log level: {:?}", config.level);

    Ok(())
}

pub fn init_default_logging() {
    let config = LogConfig::default();
    if let Err(e) = init_logging(&config) {
        eprintln!("Warning: Failed to initialize logging: {e}");
    }
    self_check();
}

fn self_check() {
    let log_dir = get_default_log_dir();
    if !log_dir.exists()
        && let Err(e) = fs::create_dir_all(&log_dir)
    {
        eprintln!(
            "Warning: Failed to create log directory {}: {}",
            log_dir.display(),
            e
        );
        return;
    }

    match fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_dir.join(".self_check"))
    {
        Ok(_) => {
            let _ = fs::remove_file(log_dir.join(".self_check"));
        }
        Err(e) => {
            eprintln!(
                "Warning: Log directory {} is not writable: {}",
                log_dir.display(),
                e
            );
        }
    }

    let config_dir = crate::utils::config::get_config_dir();
    if !config_dir.exists()
        && let Err(e) = fs::create_dir_all(&config_dir)
    {
        eprintln!(
            "Warning: Failed to create config directory {}: {}",
            config_dir.display(),
            e
        );
    }
}

fn init_async_audit_log() {
    let (tx, rx) = mpsc::sync_channel(AUDIT_LOG_CHANNEL_SIZE);

    std::thread::spawn(move || {
        let mut writer = AuditLogWriter { rx };
        writer.run();
    });

    *AUDIT_TX.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
}

struct AuditLogWriter {
    rx: mpsc::Receiver<AuditLogEntry>,
}

impl AuditLogWriter {
    fn run(&mut self) {
        while let Ok(entry) = self.rx.recv() {
            if let Err(e) = self.write_entry(&entry) {
                tracing::error!(error = %e, "Failed to write audit log entry");
            }
        }
    }

    fn write_entry(&self, entry: &AuditLogEntry) -> std::io::Result<()> {
        let log_path = match get_audit_log_path() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to get audit log path");
                return Err(e);
            }
        };

        let _ = rotate_audit_log_if_needed(
            &log_path,
            AUDIT_LOG_MAX_SIZE_BYTES,
            AUDIT_LOG_RETENTION_DAYS,
        );

        let log_entry = format!(
            "{}|{}|{}|{}|{}|{}\n",
            entry.timestamp,
            entry.action,
            escape_pipe(&entry.snippet_id),
            escape_pipe(&entry.description),
            entry.library_id,
            escape_pipe(&entry.device_id),
        );

        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            if let Err(e) = fs::set_permissions(&log_path, perms) {
                tracing::warn!(
                    path = %log_path.display(),
                    error = %e,
                    "Failed to set restrictive permissions on log file"
                );
            }
        }

        file.write_all(log_entry.as_bytes())
    }
}

pub fn shutdown_logging() {
    if let Some(tx) = AUDIT_TX.lock().unwrap_or_else(|e| e.into_inner()).take() {
        drop(tx);
    }
    if let Some(guard) = LOG_GUARD.lock().unwrap_or_else(|e| e.into_inner()).take() {
        tracing::info!("Logging shutdown complete");
        std::thread::sleep(std::time::Duration::from_millis(100));
        drop(guard);
    }
}

fn extract_panic_info(panic_info: &std::panic::PanicHookInfo) -> (String, String) {
    let location = panic_info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "unknown".to_string());

    let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "Unknown panic".to_string()
    };

    (location, message)
}

pub fn log_panic_info(panic_info: &std::panic::PanicHookInfo) {
    let (location, message) = extract_panic_info(panic_info);

    tracing::error!(
        target: "panic",
        location = %location,
        message = %message,
        "Application panicked"
    );
}

pub fn setup_panic_handler() {
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ratatui::restore();
        }));

        log_panic_info(panic_info);

        let (location, message) = extract_panic_info(panic_info);

        eprintln!("PANIC at {location}: {message}");

        previous_hook(panic_info);
    }));
}

#[tracing::instrument(level = "info", skip(result), fields(command = %redact_command(command)))]
pub fn log_command_execution(
    command: &str,
    args: &[String],
    result: &std::result::Result<(), String>,
    working_dir: Option<&std::path::Path>,
) {
    match result {
        Ok(()) => {
            tracing::info!(
                args_count = args.len(),
                working_dir = ?working_dir,
                "Command executed successfully"
            );
        }
        Err(e) => {
            tracing::error!(
                args_count = args.len(),
                error = %e,
                working_dir = ?working_dir,
                "Command execution failed"
            );
        }
    }
}

fn redact_command(command: &str) -> String {
    if command.chars().count() > 80 {
        let truncated: String = command.chars().take(77).collect();
        format!("{truncated}...")
    } else {
        command.to_string()
    }
}

/// Redacts potentially sensitive portions of a command for safe logging.
/// This truncates and is used in structured logging fields where the full
/// command is already available in the instrumented function's scope.
#[allow(dead_code)]
fn redact_sensitive(command: &str) -> String {
    let redacted_keywords = ["password", "secret", "token", "key", "api_key", "apikey"];
    let mut result = command.to_string();
    for keyword in redacted_keywords {
        // Find keyword position using case-insensitive search on the original string
        let keyword_lower = keyword.to_lowercase();
        if let Some(pos) = find_case_insensitive(&result, &keyword_lower) {
            let after_keyword = &result[pos + keyword.len()..];
            if let Some(eq_pos) = after_keyword.find('=') {
                let value_start = pos + keyword.len() + eq_pos + 1;
                let value_end = after_keyword[eq_pos + 1..]
                    .find(|c: char| c.is_whitespace())
                    .map(|i| value_start + i)
                    .unwrap_or(result.len());
                let value = &result[value_start..value_end];
                if !value.is_empty() && value != "\"\"" && value != "''" {
                    result = format!(
                        "{}{}***REDACTED***{}",
                        &result[..value_start],
                        &result[value_start..value_start + 1],
                        &result[value_end..]
                    );
                }
            }
        }
    }
    if result.chars().count() > 80 {
        let truncated: String = result.chars().take(77).collect();
        format!("{truncated}...")
    } else {
        result
    }
}

/// Case-insensitive search for `needle` in `haystack`, returning the byte offset.
fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let needle_chars: Vec<char> = needle.chars().collect();
    let needle_len = needle_chars.len();
    if needle_len == 0 {
        return None;
    }
    let haystack_chars: Vec<char> = haystack.chars().collect();
    'outer: for i in 0..haystack_chars.len() {
        if i + needle_len > haystack_chars.len() {
            break;
        }
        for (j, &nc) in needle_chars.iter().enumerate() {
            if !haystack_chars[i + j].to_lowercase().eq(nc.to_lowercase()) {
                continue 'outer;
            }
        }
        // Convert char index back to byte offset
        return haystack.char_indices().nth(i).map(|(pos, _)| pos);
    }
    None
}

pub fn log_config_operation(operation: &str, path: &Path, result: &Result<(), &str>) {
    match result {
        Ok(()) => {
            tracing::debug!(
                operation = %operation,
                path = %path.display(),
                "Config operation completed"
            );
        }
        Err(e) => {
            tracing::warn!(
                operation = %operation,
                path = %path.display(),
                error = %e,
                "Config operation failed"
            );
        }
    }
}

pub fn log_clipboard_operation(operation: &str, success: bool) {
    if success {
        tracing::debug!(operation = %operation, "Clipboard operation successful");
    } else {
        tracing::warn!(operation = %operation, "Clipboard operation failed");
    }
}

pub fn log_startup_info() {
    tracing::debug!("=== SNP Application Starting ===");
    tracing::debug!("Version: {}", env!("CARGO_PKG_VERSION"));
    tracing::debug!("Platform: {}", std::env::consts::OS);
    tracing::debug!("Architecture: {}", std::env::consts::ARCH);
    tracing::debug!(
        "Config directory: {}",
        crate::utils::config::get_config_dir().display()
    );
    tracing::debug!("Log directory: {}", get_default_log_dir().display());
}

pub fn log_shutdown_info() {
    tracing::info!("=== SNP Application Shutting Down ===");
    shutdown_logging();
}

pub fn get_audit_log_path() -> std::io::Result<std::path::PathBuf> {
    let cfg_dir = crate::utils::config::get_config_dir();
    std::fs::create_dir_all(&cfg_dir)?;
    Ok(cfg_dir.join("audit.log"))
}

#[tracing::instrument(level = "info", skip(snippet), fields(action = %action, snippet_id = %snippet.id))]
pub fn audit_log(
    action: &str,
    snippet: &crate::library::Snippet,
    library_id: Option<&str>,
) -> std::io::Result<()> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let entry = AuditLogEntry {
        timestamp,
        action: action.to_string(),
        snippet_id: snippet.id.clone(),
        description: snippet.description.clone(),
        library_id: library_id.unwrap_or("").to_string(),
        device_id: snippet.device_id.clone(),
    };

    let tx = AUDIT_TX
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .cloned();

    match tx {
        Some(tx) => tx.try_send(entry).map_err(|e| {
            tracing::warn!("Audit log channel full, dropping entry: {}", e);
            std::io::Error::new(std::io::ErrorKind::WouldBlock, e.to_string())
        }),
        None => {
            tracing::warn!("Audit log channel not initialized, writing synchronously");
            write_audit_log_entry_sync(&entry)
        }
    }
}

fn write_audit_log_entry_sync(entry: &AuditLogEntry) -> std::io::Result<()> {
    let log_path = match get_audit_log_path() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to get audit log path");
            return Err(e);
        }
    };

    let _ = rotate_audit_log_if_needed(
        &log_path,
        AUDIT_LOG_MAX_SIZE_BYTES,
        AUDIT_LOG_RETENTION_DAYS,
    );

    let log_entry = format!(
        "{}|{}|{}|{}|{}|{}\n",
        entry.timestamp,
        entry.action,
        escape_pipe(&entry.snippet_id),
        escape_pipe(&entry.description),
        entry.library_id,
        escape_pipe(&entry.device_id),
    );

    use std::io::Write;
    let mut file = match fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(f) => f,
        Err(e) => {
            tracing::error!(error = %e, path = %log_path.display(), "Failed to open audit log for writing");
            return Err(e);
        }
    };

    if let Err(e) = file.write_all(log_entry.as_bytes()) {
        tracing::error!(error = %e, path = %log_path.display(), "Failed to write to audit log");
        return Err(e);
    }
    Ok(())
}

fn escape_pipe(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => result.push_str("\\\\"),
            '|' => result.push_str("\\|"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            c if c.is_control() => {
                result.push_str(&format!("\\x{:02x}", c as u32));
            }
            _ => result.push(c),
        }
    }
    result
}

fn rotate_audit_log_if_needed(
    log_path: &Path,
    max_size_bytes: u64,
    retention_days: u64,
) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(log_path)?;
    let size = metadata.len();

    if size > max_size_bytes {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let rotated_path = log_path.with_extension(format!("{timestamp}.rotated"));
        fs::rename(log_path, rotated_path)?;
    }

    let log_dir = log_path.parent().unwrap_or(log_path);
    if let Ok(entries) = fs::read_dir(log_dir) {
        let retention_secs = retention_days * SECS_PER_DAY;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("rotated")
                && let Ok(metadata) = entry.metadata()
                && let Ok(modified) = metadata.modified()
            {
                let age = SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or_default()
                    .as_secs();
                if age > retention_secs {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_pipe_backslash() {
        assert_eq!(escape_pipe(r"foo\bar"), r"foo\\bar");
    }

    #[test]
    fn test_escape_pipe_pipe() {
        assert_eq!(escape_pipe("foo|bar"), r"foo\|bar");
    }

    #[test]
    fn test_escape_pipe_newline() {
        assert_eq!(escape_pipe("foo\nbar"), r"foo\nbar");
    }

    #[test]
    fn test_escape_pipe_carriage_return() {
        assert_eq!(escape_pipe("foo\rbar"), r"foo\rbar");
    }

    #[test]
    fn test_escape_pipe_combined() {
        assert_eq!(escape_pipe("a\\b|c\nd"), r"a\\b\|c\nd");
    }

    #[test]
    fn test_escape_pipe_empty() {
        assert_eq!(escape_pipe(""), "");
    }

    #[test]
    fn test_rotate_audit_log_creates_rotated_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let log_path = dir.path().join("audit.log");
        fs::write(&log_path, "x".repeat(100)).unwrap();
        rotate_audit_log_if_needed(&log_path, 50, 30).unwrap();
        assert!(!log_path.exists());
        let entries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("rotated"))
            .collect();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_rotate_audit_log_no_rotation_under_limit() {
        let dir = tempfile::TempDir::new().unwrap();
        let log_path = dir.path().join("audit.log");
        fs::write(&log_path, "small").unwrap();
        rotate_audit_log_if_needed(&log_path, 1024, 30).unwrap();
        assert!(log_path.exists());
    }
}
