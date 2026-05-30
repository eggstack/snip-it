//! Error types and handling for snp.
//!
//! This module defines the [`SnipError`] enum which categorizes all errors
//! that can occur during snp operations. Errors are grouped by domain:
//! I/O operations, TOML parsing, clipboard access, command execution, and runtime errors.
//!
//! # Example
//!
//! ```rust,ignore
//! use snp::error::{SnipError, SnipResult};
//!
//! fn read_config() -> SnipResult<String> {
//!     std::fs::read_to_string("config.toml")
//!         .map_err(|e| SnipError::io_error("read config", "config.toml", e))
//! }
//! ```

use std::fmt;
use std::io;
use std::path::PathBuf;

/// All possible errors that can occur in snp.
///
/// Errors are categorized by domain to make debugging and handling easier.
/// Each variant includes context about the operation that failed.
#[derive(Debug)]
pub enum SnipError {
    /// I/O operation failures.
    ///
    /// Includes file read/write errors, directory creation failures, etc.
    Io {
        operation: String,
        path: Option<PathBuf>,
        source: io::Error,
    },

    /// TOML parsing or serialization errors.
    ///
    /// Indicates malformed TOML content or serialization failures.
    Toml {
        operation: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Clipboard operation failures.
    ///
    /// Includes clipboard access errors and content transfer failures.
    Clipboard { operation: String, message: String },

    /// Command execution failures.
    ///
    /// Indicates errors when spawning or running external commands.
    Command {
        command: String,
        args: Vec<String>,
        source: io::Error,
    },

    /// Runtime errors during operation.
    ///
    /// General-purpose errors for sync failures, validation errors, etc.
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

impl From<String> for SnipError {
    fn from(error: String) -> Self {
        SnipError::Runtime {
            message: error,
            detail: None,
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
