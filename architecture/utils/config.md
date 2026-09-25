# config.rs — Path Resolution & Config Directory

[← Back to Overview](../overview.md)

## Overview

Manages platform-specific config directory resolution (XDG on Linux, Application Support on macOS, AppData on Windows) and macOS legacy config directory migration.

**File**: `src/utils/config.rs` (231 lines, including unit tests)

## Key Functions

### get_config_dir()

```rust
pub fn get_config_dir() -> PathBuf
```

Pure path builder — touches no filesystem. Returns `$XDG_CONFIG_HOME/snp` when `XDG_CONFIG_HOME` is set and non-empty, otherwise `<home>/.config/snp`. Falls back to `./snp` (with a warning) when no home directory is found. Deterministic across calls.

### ensure_config_dir()

```rust
pub fn ensure_config_dir() -> std::io::Result<PathBuf>
```

Creates the config directory if missing, tightens permissions to `0o700` on Unix. Idempotent and safe to call multiple times. Called once at startup; individual I/O helpers (logging init, library creation, premade downloads, audit logging) also call it defensively.

### get_legacy_macos_config_dir()

```rust
pub fn get_legacy_macos_config_dir() -> Option<PathBuf>
```

Returns `None` on non-macOS. On macOS, returns `~/Library/Application Support/snp/` when it exists and differs from the canonical path — even when the new path already exists, so interrupted cross-device migrations resume on the next startup.

### migrate_macos_config_dir()

```rust
pub fn migrate_macos_config_dir() -> std::io::Result<()>
```

Moves every entry from the legacy macOS path to the canonical path:

1. Fast path: `rename` each entry.
2. On `CrossesDevices` or destination-exists: `copy_recursively` + `sync_recursively`, then remove the source.
3. `sync_all` the new directory, then remove the legacy dir if empty.

No-op when there is no legacy directory.

## Path Helpers

| Function | Returns |
|----------|---------|
| `get_config_path(filename)` | `~/.config/snp/{filename}` |
| `get_snippets_path()` | `~/.config/snp/snippets.toml` |
| `get_sync_config_path()` | `~/.config/snp/sync.toml` |
| `derive_sync_state_dir()` | parent of `get_sync_config_path()` |

Other config-path functions (`get_libraries_dir`, `get_premade_dir`, `get_audit_log_path`, etc.) live on their respective domain types (`LibraryManager`, logging module), not in `config.rs`.

## XDG and Test Isolation

All paths flow through `XDG_CONFIG_HOME`, so tests isolate by pointing it at a `TempDir` (see [test-infrastructure.md](../test-infrastructure.md)). `derive_sync_state_dir` is the canonical location for auto-sync state files (pending marker, worker/execution locks, status file).

## Security

- Config directory created with `0o700` permissions on Unix
- `ensure_config_dir()` tightens existing directory permissions if needed
- Migration fsyncs copied data and the new directory before removing sources
