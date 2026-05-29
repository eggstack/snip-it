# Logging Module Review & Improvement Plan

## Architecture Document Claims vs Actual Implementation

| Claim | Documented | Actual | Status |
|-------|-----------|--------|--------|
| File location | `src/logging.rs` | `src/logging.rs` | ✓ MATCH |
| Line count | 255 lines | 263 lines | ✗ DISCREPANCY |
| LogConfig fields | log_dir, file_name, level, include_target | log_dir, file_name, level, include_target | ✓ MATCH |
| Default log dir | `~/.config/snp/logs/` | `get_config_dir().join("logs")` via XDG | ✓ MATCH |
| Log file name | `snp.log` | `snp.log` | ✓ MATCH |
| Default level | INFO | INFO | ✓ MATCH |
| Rotation | Daily via `tracing_appender::rolling::daily` | Daily (line 57) | ✓ MATCH |
| Format | Non-blocking, no ANSI, thread IDs, file, line | Lines 72-78: non_blocking, with_ansi(false), with_thread_ids, with_file, with_line_number | ✓ MATCH |
| RUST_LOG default | `snp=info,warn` | `snp={level}` (line 69) | ✗ DISCREPANCY |
| log_startup_info() | Version, platform, arch, config dirs | Lines 200-210: Version, Platform, Architecture, Config dir, Log dir | ✓ MATCH |
| log_shutdown_info() | Shutdown message, flush logs | Lines 212-215: logs shutdown message, calls shutdown_logging() | ✓ MATCH |
| log_command_execution(cmd, args, result) | Table says this signature | Line 148: `command: &str, args: &[String], result: &Result<(), String>` | ✓ MATCH |
| log_config_operation(op, path, result) | Result type unspecified | Line 172: `Result<(), &str>` | ✓ MATCH |
| log_clipboard_operation(op, success) | (op, success) where success is bool | Line 192: `operation: &str, success: bool` | ✓ MATCH |
| Panic handler restores terminal | Calls `ratatui::restore()` | Line 138 | ✓ MATCH |
| Panic handler logs to tracing | Yes | Lines 125-134 | ✓ MATCH |
| Panic handler prints to stderr | Yes | Line 144 | ✓ MATCH |
| Audit log path | `~/.config/snp/audit.log` | Line 220: `cfg_dir.join("audit.log")` | ✓ MATCH |
| Audit log format | `timestamp|action|description|command|output` | Line 247-254: timestamp, action, escaped description, escaped command, escaped output | ✓ MATCH |
| Audit log escape sequences | Pipe-delimited with escape sequences | Line 240-245: escapes `\\`, `|`, `\n`, `\r` | ✓ MATCH |
| Audit log silently fails | Silently fails if write fails | Line 228-232: returns Ok(()) on error | ✓ MATCH |
| Audit log actions | `execute`, `copy` | Used as "execute" (run_cmd.rs:58) and "copy" (clip_cmd.rs:21, run_cmd.rs:83) | ✓ MATCH |

## Bugs & Edge Cases

### 1. `log_config_operation` error type limitation (logging.rs:172)
**Bug**: Function signature uses `&str` for error type:
```rust
pub fn log_config_operation(operation: &str, path: &Path, result: &Result<(), &str>)
```
This only accepts `'static` lifetimes. Callers passing `String` errors must coerce to `&str` with `.as_str()` or use `.map_err(|e| e.as_str())`. This is fragile and inconsistent with other logging functions.

**Fix**: Change to `impl Display` or `impl std::fmt::Display` for flexible error reporting.

### 2. `shutdown_logging` may lose buffered logs (logging.rs:101-106)
**Bug**: `shutdown_logging` drops the `WorkerGuard` which should flush the non-blocking writer, but this happens AFTER the main application logic completes. If the process is terminated by signal (SIGINT, SIGTERM), `shutdown_logging` is never called.

**Evidence**: `main.rs` lines 306-316 shows `log_shutdown_info()` only called on successful command completion. Signal handler at line 308 is set up but no graceful shutdown on signals.

### 3. Audit log unbounded growth (logging.rs:217-263)
**Bug**: No log rotation or retention policy. `audit.log` grows indefinitely.

### 4. Audit log failure is invisible (logging.rs:223-232)
**Bug**: Errors are silently swallowed and logged at debug level only in callers. A persistent disk issue would not alert the user.

### 5. `LOG_GUARD` mutex poisoning (logging.rs:26-27, 102, 86)
**Bug**: `LOG_GUARD.lock().unwrap()` will panic if the mutex is poisoned. While unlikely in practice (only on panic in logging code itself), unwrap is inconsistent with defensive coding elsewhere.

## Security Concerns

### 1. Audit log contains snippet content (logging.rs:247-254)
**Concern**: Audit log records `description`, `command`, and `output` fields of snippets. If a snippet contains sensitive data (passwords, API keys, PII), it gets written to a plain text file.

**Recommendation**: Add a config option to exclude output from audit logs, or allow users to mark certain snippets as "no audit".

### 2. Log directory permissions (logging.rs:55)
**Concern**: `fs::create_dir_all(log_dir)` creates directories with default permissions. On Unix, this means `rwxr-xr-x` (755) for directories and `rw-r--r--` (644) for files. Other users on a multi-user system could read logs.

**Recommendation**: Consider setting restrictive permissions (700) on log directories, especially for audit.log.

## Potential Improvements

### 1. Add log level filter per module
Currently all logs go to the same level filter. Could allow `SNP_LOG_SYNC=debug,snp::sync=warn` style filtering.

### 2. Add structured metadata to audit log
Could include additional context: timestamp (ISO 8601), user, library name, execution duration.

### 3. Add async audit log writer
Currently audit writes are blocking IO in the main thread. For high-frequency operations, this could cause latency spikes.

### 4. Add rotation policy configuration
Allow configuring max log file size or retention days, not just daily rotation.

### 5. Add startup self-check
Verify log directory is writable before initializing, to fail fast rather than silently.

### 6. Add `log_sync_operation` function
Currently there is `log_config_operation` but no equivalent for sync operations (connect, merge, conflict resolution).

### 7. Add structured error context to `log_command_execution`
Currently the error is just a String. Could include error kind, source location, stack trace for debugging.

### 8. Consider implementing `tracing::instrument` for function spans
Could provide automatic span creation for key functions for better observability.

## Summary

The logging module is well-architected and matches most documentation. Main discrepancies:
1. Documented line count (255) doesn't match actual (263)
2. RUST_LOG default `snp=info,warn` vs actual `snp={level}` (single level)
3. Document doesn't mention the `include_target` field behavior in detail

Main concerns:
1. Audit log security (sensitive data in plain text)
2. Log directory permissions (default umask)
3. Signal handling doesn't trigger graceful log flush
4. `log_config_operation` API is fragile with `&str` error type