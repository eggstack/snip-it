use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tracing::Level;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[allow(dead_code)]
static LOG_GUARD: LazyLock<std::sync::Mutex<Option<WorkerGuard>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));

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
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("snp")
        .join("logs")
}

pub fn get_default_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("snp")
}

pub fn init_logging(config: &LogConfig) -> Result<(), Box<dyn std::error::Error>> {
    let log_dir = &config.log_dir;

    fs::create_dir_all(log_dir)?;

    let file_appender = tracing_appender::rolling::daily(log_dir, &config.file_name);

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("snp=info,warn"));

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
        drop(guard);
        tracing::info!("Logging shutdown complete");
    }
}

pub fn log_panic_info(panic_info: &std::panic::PanicHookInfo) {
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
    tracing::info!("Config directory: {}", get_default_config_dir().display());
    tracing::info!("Log directory: {}", get_default_log_dir().display());
}

pub fn log_shutdown_info() {
    tracing::info!("=== SNP Application Shutting Down ===");
    shutdown_logging();
}
