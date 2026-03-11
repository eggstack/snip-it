use std::fmt;
use std::io;
use std::path::PathBuf;

#[allow(dead_code)]
#[derive(Debug)]
pub enum SnipError {
    /// I/O operation failures
    Io {
        operation: String,
        path: Option<PathBuf>,
        source: io::Error,
    },

    /// TOML parsing/serialization errors
    Toml {
        operation: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Clipboard operation failures
    Clipboard { operation: String, message: String },

    /// Command execution failures
    Command {
        command: String,
        args: Vec<String>,
        source: io::Error,
    },

    /// Runtime errors during operation
    Runtime {
        message: String,
        detail: Option<String>,
    },
}

impl fmt::Display for SnipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnipError::Io {
                operation,
                path,
                source,
            } => {
                let path_info = match path {
                    Some(p) => format!(" (path: {})", p.display()),
                    None => String::new(),
                };
                write!(f, "I/O error during {}: {}{}", operation, source, path_info)
            }
            SnipError::Toml { operation, source } => {
                write!(f, "TOML error during {}: {}", operation, source)
            }
            SnipError::Clipboard { operation, message } => {
                write!(f, "Clipboard error during {}: {}", operation, message)
            }
            SnipError::Command {
                command,
                args,
                source,
            } => {
                let args_str = args.join(" ");
                write!(
                    f,
                    "Command execution error: '{}' {} - {}",
                    command, args_str, source
                )
            }
            SnipError::Runtime { message, detail } => {
                let detail_str = detail
                    .as_ref()
                    .map(|d| format!(": {}", d))
                    .unwrap_or_default();
                write!(f, "Runtime error: {}{}", message, detail_str)
            }
        }
    }
}

impl std::error::Error for SnipError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SnipError::Io { source, .. } => Some(source),
            SnipError::Toml { source, .. } => Some(&**source),
            SnipError::Command { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<io::Error> for SnipError {
    fn from(error: io::Error) -> Self {
        SnipError::Io {
            operation: "I/O operation".to_string(),
            path: None,
            source: error,
        }
    }
}

// Convenient error constructors
impl SnipError {
    pub fn io_error(operation: &str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        SnipError::Io {
            operation: operation.to_string(),
            path: Some(path.into()),
            source,
        }
    }

    pub fn toml_error(
        operation: &str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        SnipError::Toml {
            operation: operation.to_string(),
            source: Box::new(source),
        }
    }

    pub fn clipboard_error(operation: &str, message: impl Into<String>) -> Self {
        SnipError::Clipboard {
            operation: operation.to_string(),
            message: message.into(),
        }
    }

    pub fn command_error(command: &str, args: Vec<String>, source: io::Error) -> Self {
        SnipError::Command {
            command: command.to_string(),
            args,
            source,
        }
    }

    pub fn runtime_error(message: &str, detail: Option<&str>) -> Self {
        SnipError::Runtime {
            message: message.to_string(),
            detail: detail.map(ToString::to_string),
        }
    }
}

// Convenient Result type
pub type SnipResult<T> = Result<T, SnipError>;

// Helper macros for error handling
#[macro_export]
macro_rules! snip_err {
    ($variant:ident $(, $arg:expr)*) => {
        Err(SnipError::$variant($( $arg ),*))
    };
}

#[macro_export]
macro_rules! snip_io_err {
    ($op:expr, $path:expr, $source:expr) => {
        Err(SnipError::io_error($op, $path, $source))
    };
}

#[macro_export]
macro_rules! snip_clipboard_err {
    ($op:expr, $source:expr) => {
        Err(SnipError::clipboard_error($op, $source))
    };
}

#[macro_export]
macro_rules! snip_toml_err {
    ($op:expr, $source:expr) => {
        Err(SnipError::toml_error($op, $source))
    };
}

#[macro_export]
macro_rules! snip_runtime_err {
    ($msg:expr $(, $detail:expr)*) => {
        Err(SnipError::runtime_error($msg, $( Some($detail) ),*))
    };
}
