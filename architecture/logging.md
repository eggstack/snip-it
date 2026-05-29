# Logging

[← Back to Overview](overview.md)

## File

**`src/logging.rs`** (255 lines)

Structured logging using the `tracing` crate with file rotation and panic handling.

## Log Configuration

```rust
pub struct LogConfig {
    pub log_dir: PathBuf,      // Default: ~/.config/snp/logs/
    pub file_name: String,     // Default: "snp.log"
    pub level: Level,          // Default: INFO
    pub include_target: bool,  // Default: true
}
```

## Log Locations

- **All platforms**: `~/.config/snp/logs/snp.log`
- **Rotation**: Daily (via `tracing_appender::rolling::daily`)
- **Format**: Non-blocking writes, no ANSI colors, includes thread IDs, file, line numbers

## Log Levels

| Level | Filter |
|-------|--------|
| `trace` | Very detailed diagnostics |
| `debug` | Debug information |
| `info` | General information (default) |
| `warn` | Warning messages |
| `error` | Error messages |

Configured via `RUST_LOG` env var, defaults to `snp=info,warn`.

## Structured Logging Functions

| Function | Purpose |
|----------|---------|
| `log_startup_info()` | Version, platform, architecture, config dirs |
| `log_shutdown_info()` | Shutdown message, flush logs |
| `log_command_execution(cmd, args, result)` | Command success/failure |
| `log_config_operation(op, path, result)` | Config load/save/parse |
| `log_clipboard_operation(op, success)` | Clipboard success/failure |

## Panic Handler

`setup_panic_handler()` installs a custom panic hook that:

1. Restores the terminal (calls `ratatui::restore()`)
2. Logs panic info to tracing (location + message)
3. Prints panic info to stderr

This ensures the terminal is properly cleaned up even on panic.

## Audit Log

**File**: `~/.config/snp/audit.log`

Append-only log of snippet operations:

```
1716988800|execute|git commit|git commit -m "msg"|
1716988801|copy|docker ps|docker ps|
```

Format: `timestamp|action|description|command|output`

- Actions: `execute`, `copy`
- Pipe-delimited with escape sequences for special chars
- Silently fails if write fails (non-critical feature)

## Key Files

- `src/logging.rs` — Logging init, structured logging, panic handler, audit log
