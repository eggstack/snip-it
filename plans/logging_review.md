# Logging Module Review Plan

## Module Overview

The `logging` module (`src/logging.rs`, 255 lines) provides structured logging via the `tracing` crate, file rotation via `tracing-appender`, panic handling, and an append-only audit log. Architecture documented in `architecture/logging.md` (79 lines).

---

## 1. Document Accuracy

### Verified Correct
- `LogConfig` struct fields and defaults match code (`logging.rs:30-45`)
- Log location: `~/.config/snp/logs/` via `get_default_log_dir()` (`logging.rs:48-50`)
- Daily rotation via `tracing_appender::rolling::daily` (`logging.rs:57`)
- Non-blocking writes with ANSI disabled, thread IDs, file+line enabled (`logging.rs:59-70`)
- Structured functions: `log_startup_info`, `log_shutdown_info`, `log_command_execution`, `log_config_operation`, `log_clipboard_operation` — all present and signature-matching
- Audit log format: `timestamp|action|description|command|output` with pipe-escaping (`logging.rs:239-246`)
- Panic handler: restores terminal, logs, prints to stderr (`logging.rs:128-138`)

### Discrepancies
1. **Doc claims line count is 255** — actual is 255 lines. Correct but trivially so.
2. **Doc says default filter is `snp=info,warn`** (`architecture/logging.md:38`) — this is correct (`logging.rs:62`), but the format is unusual. `snp=info,warn` means the `snp` crate gets `info` level, and the global default is `warn`. This is non-obvious and should be documented more explicitly.
3. **Doc says audit log format includes escape sequences for special chars** — confirmed but the doc doesn't specify what's escaped (backslash, pipe, `\n`, `\r`). This is only discoverable by reading code (`logging.rs:232-237`).
4. **Doc says `setup_panic_handler()` calls `ratatui::restore()`** — confirmed (`logging.rs:130`). However, `ratatui::restore()` is also called directly in `ui.rs` at lines 1094, 1220, and 1270, meaning the panic handler's call may be a double-restore if a panic occurs during an active TUI session where the terminal was already restored. This is benign (idempotent) but worth noting.

---

## 2. Bugs & Issues

### Critical
**None.** The module is functionally correct for its stated purposes.

### High

**H1. `init_logging` ignores `config.level`** (`logging.rs:52-84`)
The `LogConfig.level` field is logged at line 81 (`tracing::info!("Log level: {:?}", config.level)`) but **never used to configure the filter**. The actual filter is always determined by `RUST_LOG` env var or the hardcoded default `snp=info,warn`. The `level` field is dead configuration — callers cannot programmatically set the log level.

**H2. `shutdown_logging` drops the guard **after** logging a shutdown message** (`logging.rs:93-98`)
```rust
pub fn shutdown_logging() {
    if let Some(guard) = LOG_GUARD.lock().unwrap().take() {
        drop(guard);             // <-- flushes and stops the writer
        tracing::info!("Logging shutdown complete");  // <-- tries to write AFTER guard dropped
    }
}
```
Line 97 logs _after_ the guard is dropped on line 95. The `info!` call goes through the tracing subscriber, but the non-blocking writer has been dropped. This message will either be silently lost or, depending on tracing-subscriber internals, may panic or produce undefined behavior. The `log_shutdown_info` function (line 204-207) calls `tracing::info!` first, then `shutdown_logging()` — so the _shutdown banner_ survives, but the _"Logging shutdown complete"_ message at line 97 does not.

**H3. Race condition in audit log file access** (`logging.rs:248-253`)
The audit log opens the file with `append(true)` per call. On concurrent invocations (e.g., if a user triggers two snippet executions in quick succession via TUI), multiple processes/threads could interleave writes. While `O_APPEND` on POSIX is atomic for writes below `PIPE_BUF` (typically 4096 bytes), there is no file locking, and very long snippet commands/descriptions could exceed that threshold, causing interleaved/corrupted entries.

### Medium

**M1. Panic handler may double-restore terminal** (`logging.rs:128-138`)
If a panic occurs while the TUI is active, `ratatui::restore()` is called by the panic handler. But `ui.rs` already calls `ratatui::restore()` in several error/exit paths (lines 1094, 1220, 1270). If the terminal is already restored, calling it again is benign on most platforms but technically undefined on some terminal implementations. A guard flag would be cleaner.

**M2. `extract_panic_info` doesn't handle `Box<dyn Any + Send>` payloads** (`logging.rs:100-115`)
Only `&str` and `String` payloads are downcast. Panics using `Box<dyn Any>` (e.g., `panic!(42)`) or custom types produce "Unknown panic" with no diagnostic. This is a minor robustness gap.

**M3. `log_shutdown_info` is misleading** (`logging.rs:204-207`)
This function logs a shutdown banner and then calls `shutdown_logging()`. But the function name suggests it only logs shutdown info. Callers must be aware it also stops the logging subsystem. A name like `shutdown_with_info` or separating the two concerns would be clearer.

**M4. `LOG_GUARD` uses `Mutex` unnecessarily** (`logging.rs:26-27`)
`LazyLock<Mutex<Option<WorkerGuard>>>` — the guard is only ever written once (in `init_logging`) and read/dropped once (in `shutdown_logging`). A `std::sync::OnceLock` or even an `AtomicPtr` with unsafe would be simpler and avoid the mutex overhead. Not a performance issue, but a design smell.

### Low

**L1. `get_audit_log_path` creates the config dir** (`logging.rs:210-212`)
The audit log path helper calls `create_dir_all`, which is a side effect for a getter. Callers don't expect path resolution to create directories.

**L2. Audit log timestamp is seconds-precision** (`logging.rs:227-230`)
Two operations within the same second get the same timestamp, making ordering ambiguous. Using `SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()` or including a monotonic counter would improve audit fidelity.

**L3. `escape_pipe` is defined inside `audit_log` as a nested function** (`logging.rs:232-237`)
This is unusual in Rust (nested `fn` items are rare and don't capture environment). It should be a private module-level function for clarity and testability.

---

## 3. Design Issues

### D1. No `init` guard / double-init protection
`init_logging` can be called multiple times. `tracing_subscriber::registry().init()` will panic on second call (subscriber already set). `init_default_logging` doesn't guard against this either. In the current code, `main()` calls it once, but a library consumer or test harness could easily double-init.

### D2. Logging is file-only — no stdout/stderr layer
The subscriber only has a file layer. During development or when running `snp` directly in a terminal, users see no log output. Adding an optional stderr layer (controlled by an env var like `SNP_STDERR_LOG=1`) would improve debuggability.

### D3. Audit log is tightly coupled to `Snippet` type
`audit_log` takes `&crate::library::Snippet` directly, making it impossible to audit non-snippet operations (e.g., config changes, sync events) without passing a dummy snippet. A trait or a more generic log entry type would be more extensible.

### D4. No log file cleanup / size management
Daily rotation creates new files indefinitely (`snp.log.2024-01-01`, `snp.log.2024-01-02`, etc.). There is no mechanism to delete logs older than N days. Over months, disk usage grows unbounded.

### D5. Dead code: `LogConfig.level` field
As noted in H1, the `level` field is never used to configure the filter. It should either be wired into the `EnvFilter` construction or removed.

---

## 4. Security Concerns

### S1. Audit log has no access controls
The audit log file is created with default permissions (`0644` on most systems). On multi-user systems, other users can read snippet command history. The file should be created with restricted permissions (`0600`).

### S2. Audit log entries are not authenticated
The pipe-delimited format is trivially forgeable. A malicious process could inject fake audit entries. For a local-only tool this is low-risk, but if the audit log is ever forwarded to a SIEM or central server, it lacks integrity guarantees.

### S3. Panic handler writes to stderr without sanitization
If a panic message contains control characters or ANSI escape sequences, the `eprintln!` at line 136 will pass them through to the terminal. This is a minor terminal injection vector.

### S4. `config.level` logged at INFO level leaks implementation detail
Line 81 logs the resolved config level. If a user sets `RUST_LOG=debug` and their config has secrets nearby, the log level line is harmless, but the general pattern of logging config internals at INFO is a mild information leak.

---

## 5. Performance Issues

### P1. `audit_log` opens the file on every call
Each snippet execution opens, appends to, and closes the audit log file. For high-frequency usage, this is a measurable overhead. A persistent file handle (or buffered writer) would be more efficient.

### P2. Non-blocking writer uses default buffer size
`tracing_appender::non_blocking(file_appender)` uses the default channel buffer size (typically 128KB). For very high log throughput, this could cause dropped messages. Consider configuring the buffer size explicitly.

### P3. `escape_pipe` allocates 4 intermediate `String`s
The chained `.replace()` calls create temporary `String` allocations. For the audit log's low frequency this is negligible, but the pattern is wasteful for hot paths.

---

## 6. Test Coverage Gaps

### No unit tests exist for `logging.rs`
- `init_logging` / `init_default_logging`: Not tested
- `shutdown_logging`: Not tested
- `extract_panic_info`: Not tested
- `log_panic_info`: Not tested
- `setup_panic_handler`: Not tested
- `log_command_execution`: Not tested
- `log_config_operation`: Not tested
- `log_clipboard_operation`: Not tested
- `log_startup_info` / `log_shutdown_info`: Not tested
- `get_audit_log_path`: Not tested
- `audit_log`: Not tested
- `escape_pipe`: Not tested

The integration tests (`tests/integration.rs`) exercise the binary end-to-end but don't verify logging behavior (e.g., log file creation, content format, rotation).

### Recommended test cases
1. `test_init_logging_creates_dir` — verify log directory is created
2. `test_init_logging_double_init_panics` — confirm double-init behavior
3. `test_audit_log_format` — verify pipe-delimited format and escaping
4. `test_audit_log_concurrent_writes` — verify atomicity under concurrency
5. `test_extract_panic_info_str_payload` — &str panic message
6. `test_extract_panic_info_string_payload` — String panic message
7. `test_extract_panic_info_unknown_payload` — Box<dyn Any> fallback
8. `test_escape_pipe_special_chars` — backslash, pipe, newline, carriage return
9. `test_shutdown_logging_drops_guard` — verify guard is taken

---

## 7. Priority Ranking

| ID | Severity | Category | Summary |
|----|----------|----------|---------|
| H1 | **High** | Bug | `config.level` field is dead — never used for filter |
| H2 | **High** | Bug | `shutdown_logging` logs after dropping writer guard |
| H3 | **High** | Bug | Audit log has no file locking for concurrent writes |
| M1 | **Medium** | Design | Panic handler may double-restore terminal |
| M2 | **Medium** | Robustness | `extract_panic_info` misses non-string payloads |
| M3 | **Medium** | Design | `log_shutdown_info` has mixed responsibilities |
| M4 | **Medium** | Design | `LOG_GUARD` Mutex is unnecessary |
| D1 | **Medium** | Design | No double-init guard for subscriber |
| D2 | **Medium** | Design | No stderr logging option for dev/debug |
| D3 | **Medium** | Design | Audit log tightly coupled to `Snippet` type |
| D4 | **Medium** | Design | No log file cleanup / size management |
| D5 | **Medium** | Dead code | `LogConfig.level` unused |
| S1 | **Medium** | Security | Audit log created with default (world-readable) perms |
| S2 | **Low** | Security | Audit log entries not authenticated |
| S3 | **Low** | Security | Panic messages not sanitized for terminal injection |
| S4 | **Low** | Security | Config internals logged at INFO |
| P1 | **Low** | Performance | Audit log opens file on every call |
| P2 | **Low** | Performance | Non-blocking writer uses default buffer size |
| P3 | **Low** | Performance | `escape_pipe` allocates intermediate strings |
| L1 | **Low** | Design | `get_audit_log_path` has side effect (creates dirs) |
| L2 | **Low** | Design | Audit timestamp is seconds-precision |
| L3 | **Low** | Style | `escape_pipe` is a nested fn (unusual in Rust) |

---

## 8. Recommendations

### Immediate (before merge / release)
1. **Fix H2**: Swap order in `shutdown_logging` — log _before_ dropping the guard.
2. **Fix H1**: Either wire `config.level` into `EnvFilter` or remove the field and document that `RUST_LOG` is the source of truth.
3. **Fix S1**: Set file permissions to `0600` on the audit log (use `std::os::unix::fs::OpenOptionsExt` on Unix).

### Short-term
4. **Add double-init protection** (D1): Use `Once` or `OnceLock` to prevent `init_logging` from being called twice.
5. **Add stderr logging layer** (D2): Conditional `tracing_subscriber` stderr layer controlled by `SNP_DEBUG=1` env var.
6. **Add unit tests** for `escape_pipe`, `extract_panic_info`, `audit_log` format, and `init_logging` directory creation.
7. **Fix M1**: Add a `static AtomicBool` terminal-restore guard in the panic handler.

### Medium-term
8. **Implement log file cleanup** (D4): Delete log files older than 30 days on startup.
9. **Refactor audit log** (D3): Decouple from `Snippet` type with a generic `AuditEntry` struct.
10. **Add file locking** (H3): Use `flock` (Unix) or equivalent for audit log writes.
11. **Use `as_millis()`** (L2) for audit timestamps to reduce collision probability.

### Long-term
12. **Consider structured audit log format** (S2): Switch from pipe-delimited to JSON for machine parsing and integrity (e.g., include HMAC).
13. **Add audit log rotation** — currently append-only with no size limit.
14. **Explore `tracing-appender::rolling::Builder`** for configurable rotation policies (size-based, time-based, or hybrid).
