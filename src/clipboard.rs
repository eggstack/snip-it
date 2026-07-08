//! Cross-platform clipboard access.
//!
//! Provides a unified interface for clipboard operations across Windows, macOS, and Linux.
//!
//! # Platform Support
//!
//! - **Windows**: Uses `clipboard-win` crate
//! - **macOS/Linux**: Uses `arboard` crate

use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[cfg(not(windows))]
use arboard::Clipboard;

use crate::error::{SnipError, SnipResult};
use crate::logging::log_clipboard_operation;

static CLIPBOARD_GENERATION: AtomicU64 = AtomicU64::new(0);

const DEFAULT_CLIPBOARD_TIMEOUT_SECS: u64 = 5;

struct CachedClipboardSettings {
    auto_clear_seconds: Option<u32>,
}

static CACHED_CLIPBOARD_SETTINGS: LazyLock<std::sync::Mutex<Option<CachedClipboardSettings>>> =
    LazyLock::new(|| std::sync::Mutex::new(None));

fn get_clipboard_auto_clear_seconds() -> Option<u32> {
    {
        let cache = CACHED_CLIPBOARD_SETTINGS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(ref settings) = *cache {
            return settings.auto_clear_seconds;
        }
    }
    let settings = crate::config::get_sync_settings();
    let result = settings.clipboard_auto_clear_seconds;
    let mut cache = CACHED_CLIPBOARD_SETTINGS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *cache = Some(CachedClipboardSettings {
        auto_clear_seconds: result,
    });
    result
}

pub fn invalidate_clipboard_settings_cache() {
    let mut cache = CACHED_CLIPBOARD_SETTINGS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *cache = None;
}

fn get_clipboard_timeout() -> Duration {
    let secs = std::env::var("SNP_CLIPBOARD_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CLIPBOARD_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

/// Schedule automatic clipboard clearing after the specified number of seconds.
///
/// Uses an atomic generation counter so that a new copy cancels any pending clear.
/// The clear runs on a detached thread — if the process exits first, the clear
/// is skipped (best-effort security measure).
pub fn schedule_clipboard_clear(seconds: u32) {
    if seconds == 0 {
        return;
    }

    let gen_at_spawn = CLIPBOARD_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    let _handle = thread::spawn(move || {
        thread::sleep(Duration::from_secs(seconds as u64));
        if gen_at_spawn == CLIPBOARD_GENERATION.load(Ordering::SeqCst)
            && let Err(e) = clear_clipboard()
        {
            tracing::warn!("Auto-clear clipboard failed: {}", e);
        }
    });
}

fn clear_clipboard_impl() -> SnipResult<()> {
    #[cfg(windows)]
    {
        clipboard_win::set_clipboard(clipboard_win::formats::Unicode, "")
            .map_err(|e| SnipError::clipboard_error("clear clipboard", format!("{}", e)))?;
    }

    #[cfg(not(windows))]
    {
        let mut ctx = Clipboard::new().map_err(|e| {
            SnipError::clipboard_error("create clipboard context for clear", format!("{e}"))
        })?;
        ctx.set_text("")
            .map_err(|e| SnipError::clipboard_error("clear clipboard", format!("{e}")))?;
    }

    log_clipboard_operation("clear_clipboard", true);
    Ok(())
}

fn with_clipboard_timeout<F, T>(operation: &str, f: F) -> SnipResult<T>
where
    F: FnOnce() -> SnipResult<T> + Send + 'static,
    T: Send + 'static,
{
    let timeout = get_clipboard_timeout();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = f();
        let _ = tx.send(result);
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            tracing::warn!(
                "Clipboard operation '{}' timed out after {} seconds",
                operation,
                timeout.as_secs()
            );
            Err(SnipError::clipboard_error(
                operation,
                format!(
                    "clipboard operation timed out after {} seconds",
                    timeout.as_secs()
                ),
            ))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(SnipError::clipboard_error(
            operation,
            "clipboard operation channel closed unexpectedly",
        )),
    }
}

pub fn clear_clipboard() -> SnipResult<()> {
    with_clipboard_timeout("clear", clear_clipboard_impl)
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
    let auto_clear_seconds = get_clipboard_auto_clear_seconds();
    copy_to_clipboard_with_auto_clear(text, auto_clear_seconds)
}

/// Copy text to the system clipboard.
///
/// Only plain text is supported. `clipboard-win` does not expose rich text
/// or HTML clipboard formats through its public API, so content type cannot
/// be preserved for non-text payloads.
#[cfg(windows)]
pub fn copy_to_clipboard(text: &str) -> SnipResult<()> {
    let text = text.to_owned();
    with_clipboard_timeout("copy", move || {
        clipboard_win::set_clipboard(clipboard_win::formats::Unicode, &text).map_err(|e| {
            log_clipboard_operation("set_text", false);
            SnipError::clipboard_error("set text", format!("{}", e))
        })?;

        log_clipboard_operation("set_text", true);
        Ok(())
    })
}

/// Copy text to the system clipboard.
///
/// Only plain text is supported. `arboard` handles text natively and does
/// not expose rich text or HTML clipboard formats, so content type cannot be
/// preserved for non-text payloads.
#[cfg(not(windows))]
pub fn copy_to_clipboard(text: &str) -> SnipResult<()> {
    let text = text.to_owned();
    with_clipboard_timeout("copy", move || {
        let mut ctx = Clipboard::new().map_err(|e| {
            log_clipboard_operation("create context", false);
            SnipError::clipboard_error("create clipboard context", format!("{e}"))
        })?;

        ctx.set_text(&text).map_err(|e| {
            log_clipboard_operation("set_text", false);
            SnipError::clipboard_error("set text", format!("{e}"))
        })?;

        log_clipboard_operation("set_text", true);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore] // Requires a display server (no clipboard on headless CI)
    fn test_clipboard_copy_empty_string() {
        let result = copy_to_clipboard("");
        assert!(result.is_ok());
    }

    #[test]
    #[ignore] // Requires a display server (no clipboard on headless CI)
    fn test_clipboard_copy_normal_text() {
        let result = copy_to_clipboard("test content");
        assert!(result.is_ok());
    }

    #[test]
    #[ignore] // Requires a display server (no clipboard on headless CI)
    fn test_clipboard_copy_unicode() {
        let result = copy_to_clipboard("héllo wörld 🎉");
        assert!(result.is_ok());
    }

    #[test]
    #[ignore] // Requires a display server (no clipboard on headless CI)
    fn test_clipboard_copy_multiline() {
        let result = copy_to_clipboard("line1\nline2\nline3");
        assert!(result.is_ok());
    }

    #[test]
    #[ignore] // Requires a display server (no clipboard on headless CI)
    fn test_clipboard_copy_special_chars() {
        let result = copy_to_clipboard("echo 'hello' | grep 'world'");
        assert!(result.is_ok());
    }

    #[test]
    #[ignore] // Requires a display server (no clipboard on headless CI)
    fn test_clipboard_copy_long_content() {
        let long_text = "x".repeat(100000);
        let result = copy_to_clipboard(&long_text);
        assert!(result.is_ok());
    }
}
