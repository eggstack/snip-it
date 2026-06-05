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
        let operation = match error.kind() {
            io::ErrorKind::NotFound => "file not found",
            io::ErrorKind::PermissionDenied => "permission denied",
            io::ErrorKind::AlreadyExists => "file already exists",
            io::ErrorKind::InvalidInput => "invalid input",
            io::ErrorKind::InvalidData => "invalid data",
            io::ErrorKind::UnexpectedEof => "unexpected end of file",
            _ => "I/O operation",
        };
        SnipError::Io {
            operation: operation.to_string(),
            path: None,
            source: error,
        }
    }
}

// Convenient error constructors
impl SnipError {
    /// Create an I/O error with operation context and optional file path.
    pub fn io_error(operation: &str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        SnipError::Io {
            operation: operation.to_string(),
            path: Some(path.into()),
            source,
        }
    }

    /// Create a TOML parsing or serialization error with operation context.
    pub fn toml_error(
        operation: &str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        SnipError::Toml {
            operation: operation.to_string(),
            source: Box::new(source),
        }
    }

    /// Create a clipboard error with operation context and message.
    pub fn clipboard_error(operation: &str, message: impl Into<String>) -> Self {
        SnipError::Clipboard {
            operation: operation.to_string(),
            message: message.into(),
        }
    }

    /// Create a command execution error with command name, arguments, and source error.
    pub fn command_error(command: &str, args: Vec<String>, source: io::Error) -> Self {
        SnipError::Command {
            command: command.to_string(),
            args,
            source,
        }
    }

    /// Create a runtime error with a message and optional detail string.
    pub fn runtime_error(message: &str, detail: Option<&str>) -> Self {
        SnipError::Runtime {
            message: message.to_string(),
            detail: detail.map(ToString::to_string),
        }
    }
}

/// Convenient Result type
pub type SnipResult<T> = Result<T, SnipError>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn test_io_error_display() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err = SnipError::io_error("read config", "/etc/config.toml", io_err);
        let msg = err.to_string();
        assert!(msg.contains("read config"));
        assert!(msg.contains("/etc/config.toml"));
    }

    #[test]
    fn test_toml_error_display() {
        let toml_err = toml::from_str::<toml::Value>("invalid = [toml").unwrap_err();
        let err = SnipError::toml_error("parse config", toml_err);
        let msg = err.to_string();
        assert!(msg.contains("parse config"));
    }

    #[test]
    fn test_clipboard_error_display() {
        let err = SnipError::clipboard_error("copy to clipboard", "no clipboard available");
        let msg = err.to_string();
        assert!(msg.contains("copy to clipboard"));
        assert!(msg.contains("no clipboard available"));
    }

    #[test]
    fn test_command_error_display() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "command not found");
        let err = SnipError::command_error("git", vec!["status".to_string()], io_err);
        let msg = err.to_string();
        assert!(msg.contains("git"));
        assert!(msg.contains("status"));
    }

    #[test]
    fn test_runtime_error_display_with_detail() {
        let err = SnipError::runtime_error("sync failed", Some("server unavailable"));
        let msg = err.to_string();
        assert!(msg.contains("sync failed"));
        assert!(msg.contains("server unavailable"));
    }

    #[test]
    fn test_runtime_error_display_without_detail() {
        let err = SnipError::runtime_error("sync failed", None);
        let msg = err.to_string();
        assert!(msg.contains("sync failed"));
        assert!(!msg.contains("server unavailable"));
    }

    #[test]
    fn test_from_io_error() {
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        let err: SnipError = io_err.into();
        let msg = err.to_string();
        assert!(msg.contains("permission denied"));
    }

    #[test]
    fn test_from_io_error_not_found() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "no such file");
        let err: SnipError = io_err.into();
        let msg = err.to_string();
        assert!(msg.contains("file not found"));
    }

    #[test]
    fn test_error_source_io() {
        let io_err = io::Error::other("test");
        let err = SnipError::io_error("op", "path", io_err);
        assert!(err.source().is_some());
    }

    #[test]
    fn test_error_source_toml() {
        let toml_err = toml::from_str::<toml::Value>("invalid = [toml").unwrap_err();
        let err = SnipError::toml_error("op", toml_err);
        assert!(err.source().is_some());
    }

    #[test]
    fn test_error_source_runtime() {
        let err = SnipError::runtime_error("msg", None);
        assert!(err.source().is_none());
    }
}
