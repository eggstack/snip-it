# config.rs — Path Resolution & Config Directory

## Overview

Manages platform-specific config directory resolution (XDG on Linux, Application Support on macOS, AppData on Windows) and macOS legacy config directory migration.

**File**: `src/utils/config.rs`

## Key Functions

### get_config_dir()

```rust
pub fn get_config_dir() -> PathBuf
```

Returns `~/.config/snp/` (XDG-compliant). Checks `XDG_CONFIG_HOME` first, falls back to `~/.config/snp/`.

### ensure_config_dir()

```rust
pub fn ensure_config_dir() -> std::io::Result<PathBuf>
```

Creates the config directory if missing, tightens permissions to `0o700` on Unix. Idempotent. Called at startup before any I/O.

### get_legacy_macos_config_dir()

```rust
pub fn get_legacy_macos_config_dir() -> Option<PathBuf>
```

Detects old macOS config at `~/Library/Application Support/snp/` when the canonical `~/.config/snp/` does not exist.

### migrate_macos_config_dir()

```rust
pub fn migrate_macos_config_dir() -> std::io::Result<()>
```

Recursively copies all files from the legacy macOS path to the canonical path.

## Path Helpers

| Function | Returns |
|----------|---------|
| `get_config_path(filename)` | `~/.config/snp/{filename}` |
| `get_snippets_path()` | `~/.config/snp/snippets.toml` |
| `get_sync_config_path()` | `~/.config/snp/sync.toml` |
| `derive_sync_state_dir()` | parent of `get_sync_config_path()` |

Other config-path functions (`get_libraries_dir`, `get_premade_dir`, `get_audit_log_path`, etc.) live on their respective domain types (`LibraryManager`, logging module), not in `config.rs`.

## Security

- Config directory created with `0o700` permissions on Unix
- `ensure_config_dir()` tightens existing directory permissions if needed
