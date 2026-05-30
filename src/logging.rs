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
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

const AUDIT_LOG_MAX_SIZE_BYTES: u64 = 10 * 1024 * 1024;
const AUDIT_LOG_RETENTION_DAYS: u64 = 30;

#[allow(dead_code)]
static LOG_GUARD: LazyLock<std::sync::Mutex<Option<WorkerGuard>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));

/// Configuration for logging behavior.
pub struct LogConfig {
    pub log_dir: PathBuf,
    pub file_name: String,
    pub level: Level,
    pub include_target: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        LogConfig {
            log_dir: get_default_log_dir(),
            file_name: "snp.log".to_string(),
            level: Level::INFO,
            include_target: true,
        }
    }
}

pub fn get_default_log_dir() -> PathBuf {
    crate::utils::config::get_config_dir().join("logs")
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

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        let level = match config.level {
            Level::ERROR => "error",
            Level::WARN => "warn",
            Level::INFO => "info",
            Level::DEBUG => "debug",
            Level::TRACE => "trace",
        };
        EnvFilter::new(format!("snp={}", level))
    });

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

    *LOG_GUARD.lock().unwrap() = Some(guard);

    tracing::info!("Logging initialized. Log directory: {}", log_dir.display());
    tracing::info!("Log level: {:?}", config.level);

    Ok(())
}

pub fn init_default_logging() {
    let config = LogConfig::default();
    if let Err(e) = init_logging(&config) {
        eprintln!("Warning: Failed to initialize logging: {}", e);
    }
}

pub fn shutdown_logging() {
    if let Some(guard) = LOG_GUARD.lock().unwrap().take() {
        tracing::info!("Logging shutdown complete");
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
    std::panic::set_hook(Box::new(|panic_info| {
        ratatui::restore();

        log_panic_info(panic_info);

        let (location, message) = extract_panic_info(panic_info);

        eprintln!("PANIC at {}: {}", location, message);
    }));
}

pub fn log_command_execution(
    command: &str,
    args: &[String],
    result: &std::result::Result<(), String>,
) {
    match result {
        Ok(()) => {
            tracing::info!(
                command = %command,
                args = ?args,
                "Command executed successfully"
            );
        }
        Err(e) => {
            tracing::error!(
                command = %command,
                args = ?args,
                error = %e,
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

pub fn audit_log(action: &str, snippet: &crate::library::Snippet) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;

    let log_path = match get_audit_log_path() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to get audit log path");
            return Err(e);
        }
    };

    if let Err(e) = rotate_audit_log_if_needed(&log_path) {
        tracing::warn!(error = %e, "Failed to rotate audit log");
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let log_entry = format!(
        "{}|{}|{}|{}\n",
        timestamp,
        action,
        escape_pipe(&snippet.id),
        escape_pipe(&snippet.description)
    );

    let mut file = match OpenOptions::new().create(true).append(true).open(&log_path) {
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

fn rotate_audit_log_if_needed(log_path: &Path) -> std::io::Result<()> {
    let metadata = fs::metadata(log_path)?;
    let size = metadata.len();

    if size > AUDIT_LOG_MAX_SIZE_BYTES {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let rotated_path = log_path.with_extension(format!("{}.rotated", timestamp));
        fs::rename(log_path, rotated_path)?;
    }

    let log_dir = log_path.parent().unwrap_or(log_path);
    if let Ok(entries) = fs::read_dir(log_dir) {
        let retention_secs = AUDIT_LOG_RETENTION_DAYS * 86400;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("rotated") {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(modified) = metadata.modified() {
                        let age = SystemTime::now()
                            .duration_since(modified)
                            .unwrap()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
