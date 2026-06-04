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
use std::sync::mpsc;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

const AUDIT_LOG_MAX_SIZE_BYTES: u64 = 10 * 1024 * 1024; // 10 MB — rotate audit log when exceeded
const AUDIT_LOG_RETENTION_DAYS: u64 = 30; // Keep 30 days of rotated audit logs
const AUDIT_LOG_CHANNEL_SIZE: usize = 1024; // Bounded channel for async audit writes

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

#[allow(dead_code)]
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

#[tracing::instrument(level = "info", skip(config))]
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
            eprintln!("Warning: Invalid SNP_LOG filter '{}': {}", filter_str, e);
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

    subscriber.init();

    *LOG_GUARD.lock().unwrap_or_else(|e| e.into_inner()) = Some(guard);

    init_async_audit_log();

    tracing::info!("Logging initialized. Log directory: {}", log_dir.display());
    tracing::info!("Log level: {:?}", config.level);

    Ok(())
}

pub fn init_default_logging() {
    let config = LogConfig::default();
    if let Err(e) = init_logging(&config) {
        eprintln!("Warning: Failed to initialize logging: {}", e);
    }
    self_check();
}

/// Logs an error with its full cause chain.
///
/// This utility is available for future use but is not currently called by the CLI.
#[allow(dead_code)]
pub fn log_any_error(context: &str, error: &dyn std::error::Error) {
    tracing::error!(error = %error, context = %context, "Error occurred");
    let mut source = error.source();
    while let Some(cause) = source {
        tracing::debug!(error = %cause, context = %context, "Caused by");
        source = cause.source();
    }
}

fn self_check() {
    let log_dir = get_default_log_dir();
    if !log_dir.exists() {
        if let Err(e) = fs::create_dir_all(&log_dir) {
            eprintln!(
                "Warning: Failed to create log directory {}: {}",
                log_dir.display(),
                e
            );
            return;
        }
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
    if !config_dir.exists() {
        if let Err(e) = fs::create_dir_all(&config_dir) {
            eprintln!(
                "Warning: Failed to create config directory {}: {}",
                config_dir.display(),
                e
            );
        }
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

        eprintln!("PANIC at {}: {}", location, message);

        previous_hook(panic_info);
    }));
}

#[tracing::instrument(level = "info", skip(result), fields(command = %command))]
pub fn log_command_execution(
    command: &str,
    args: &[String],
    result: &std::result::Result<(), String>,
    working_dir: Option<&std::path::Path>,
) {
    match result {
        Ok(()) => {
            tracing::info!(
                args = ?args,
                working_dir = ?working_dir,
                "Command executed successfully"
            );
        }
        Err(e) => {
            tracing::error!(
                args = ?args,
                error = %e,
                working_dir = ?working_dir,
                "Command execution failed"
            );
        }
    }
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
    tracing::info!("=== SNP Application Starting ===");
    tracing::info!("Version: {}", env!("CARGO_PKG_VERSION"));
    tracing::info!("Platform: {}", std::env::consts::OS);
    tracing::info!("Architecture: {}", std::env::consts::ARCH);
    tracing::info!(
        "Config directory: {}",
        crate::utils::config::get_config_dir().display()
    );
    tracing::info!("Log directory: {}", get_default_log_dir().display());
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
        Some(tx) => tx
            .try_send(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::WouldBlock, e.to_string())),
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
    s.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn rotate_audit_log_if_needed(
    log_path: &Path,
    max_size_bytes: u64,
    retention_days: u64,
) -> std::io::Result<()> {
    let metadata = fs::metadata(log_path)?;
    let size = metadata.len();

    if size > max_size_bytes {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let rotated_path = log_path.with_extension(format!("{}.rotated", timestamp));
        fs::rename(log_path, rotated_path)?;
    }

    let log_dir = log_path.parent().unwrap_or(log_path);
    if let Ok(entries) = fs::read_dir(log_dir) {
        let retention_secs = retention_days * 86400;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("rotated") {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(modified) = metadata.modified() {
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
        }
    }

    Ok(())
}

/// Types of sync operations for audit logging.
///
/// This enum is available for future use but is not currently used by the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SyncOperationType {
    Connect,
    Disconnect,
    Merge,
    Push,
    Pull,
    ConflictResolved,
    SyncFailed,
}

impl std::fmt::Display for SyncOperationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncOperationType::Connect => write!(f, "connect"),
            SyncOperationType::Disconnect => write!(f, "disconnect"),
            SyncOperationType::Merge => write!(f, "merge"),
            SyncOperationType::Push => write!(f, "push"),
            SyncOperationType::Pull => write!(f, "pull"),
            SyncOperationType::ConflictResolved => write!(f, "conflict_resolved"),
            SyncOperationType::SyncFailed => write!(f, "sync_failed"),
        }
    }
}

/// Logs the result of a sync operation.
///
/// This function is available for future use but is not currently called by the CLI.
#[allow(dead_code)]
pub fn log_sync_operation(
    operation: SyncOperationType,
    library_id: Option<&str>,
    result: &Result<(), String>,
) {
    match result {
        Ok(()) => {
            tracing::info!(
                operation = %operation,
                library_id = ?library_id,
                "Sync operation completed"
            );
        }
        Err(e) => {
            tracing::error!(
                operation = %operation,
                library_id = ?library_id,
                error = %e,
                "Sync operation failed"
            );
        }
    }
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
