//! Cross-platform clipboard access.
//!
//! Provides a unified interface for clipboard operations across Windows, macOS, and Linux.
//!
//! # Platform Support
//!
//! - **Windows**: Uses `clipboard-win` crate
//! - **macOS/Linux**: Uses `copypasta` crate

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

#[cfg(windows)]
use clipboard_win::{formats, Clipboard};

#[cfg(not(windows))]
use copypasta::{ClipboardContext, ClipboardProvider};

use crate::error::{SnipError, SnipResult};
use crate::logging::log_clipboard_operation;

static CLIPBOARD_CLEAR_SCHEDULED: AtomicBool = AtomicBool::new(false);

pub fn schedule_clipboard_clear(seconds: u32) {
    if seconds == 0 {
        return;
    }

    if CLIPBOARD_CLEAR_SCHEDULED.swap(true, Ordering::SeqCst) {
        return;
    }

    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_secs(seconds as u64));
        if let Err(e) = clear_clipboard() {
            tracing::debug!("Auto-clear clipboard failed: {}", e);
        }
        CLIPBOARD_CLEAR_SCHEDULED.store(false, Ordering::SeqCst);
    });

    std::mem::drop(handle);
}

fn clear_clipboard_impl() -> SnipResult<()> {
    #[cfg(windows)]
    {
        let mut clipboard = Clipboard::new().map_err(|e| {
            SnipError::clipboard_error("open clipboard for clear", format!("{}", e))
        })?;
        clipboard
            .set_text("")
            .map_err(|e| SnipError::clipboard_error("clear clipboard", format!("{}", e)))?;
    }

    #[cfg(not(windows))]
    {
        let mut ctx = ClipboardContext::new().map_err(|e| {
            SnipError::clipboard_error("create clipboard context for clear", format!("{}", e))
        })?;
        ctx.set_contents(String::new())
            .map_err(|e| SnipError::clipboard_error("clear clipboard", format!("{}", e)))?;
    }

    log_clipboard_operation("clear_clipboard", true);
    Ok(())
}

pub fn clear_clipboard() -> SnipResult<()> {
    clear_clipboard_impl()
}

pub fn copy_to_clipboard_with_auto_clear(
    text: &str,
    auto_clear_seconds: Option<u32>,
) -> SnipResult<()> {
    copy_to_clipboard(text)?;
    if let Some(seconds) = auto_clear_seconds {
        schedule_clipboard_clear(seconds);
    }
    Ok(())
}

pub fn copy_to_clipboard_auto(text: &str) -> SnipResult<()> {
    let settings = crate::config::get_sync_settings();
    copy_to_clipboard_with_auto_clear(text, settings.clipboard_auto_clear_seconds)
}

/// Copy text to the system clipboard.
#[cfg(windows)]
pub fn copy_to_clipboard(text: &str) -> SnipResult<()> {
    let mut clipboard = Clipboard::new().map_err(|e| {
        log_clipboard_operation("open clipboard", false);
        SnipError::clipboard_error("open clipboard", format!("{}", e))
    })?;

    clipboard.set_text(text).map_err(|e| {
        log_clipboard_operation("set_text", false);
        SnipError::clipboard_error("set text", format!("{}", e))
    })?;

    log_clipboard_operation("set_text", true);
    Ok(())
}

#[cfg(not(windows))]
pub fn copy_to_clipboard(text: &str) -> SnipResult<()> {
    let mut ctx = ClipboardContext::new().map_err(|e| {
        log_clipboard_operation("create context", false);
        SnipError::clipboard_error("create clipboard context", format!("{}", e))
    })?;

    ctx.set_contents(text.to_owned()).map_err(|e| {
        log_clipboard_operation("set_contents", false);
        SnipError::clipboard_error("set contents", format!("{}", e))
    })?;

    log_clipboard_operation("set_contents", true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clipboard_copy_empty_string() {
        let result = copy_to_clipboard("");
        assert!(result.is_ok());
    }

    #[test]
    fn test_clipboard_copy_normal_text() {
        let result = copy_to_clipboard("test content");
        assert!(result.is_ok());
    }

    #[test]
    fn test_clipboard_copy_unicode() {
        let result = copy_to_clipboard("héllo wörld 🎉");
        assert!(result.is_ok());
    }

    #[test]
    fn test_clipboard_copy_multiline() {
        let result = copy_to_clipboard("line1\nline2\nline3");
        assert!(result.is_ok());
    }

    #[test]
    fn test_clipboard_copy_special_chars() {
        let result = copy_to_clipboard("echo 'hello' | grep 'world'");
        assert!(result.is_ok());
    }

    #[test]
    fn test_clipboard_copy_long_content() {
        let long_text = "x".repeat(100000);
        let result = copy_to_clipboard(&long_text);
        assert!(result.is_ok());
    }
}
