#[cfg(windows)]
use clipboard_win::{formats, Clipboard};

#[cfg(not(windows))]
use copypasta::{ClipboardContext, ClipboardProvider};

use crate::error::{SnipError, SnipResult};
use crate::logging::log_clipboard_operation;

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
