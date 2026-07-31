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
| `get_snippets_path()` | `~/.config/snp/snippets.toml` |
| `get_libraries_dir()` | `~/.config/snp/libraries/` |
| `get_libraries_index_path()` | `~/.config/snp/libraries.toml` |
| `get_sync_config_path()` | `~/.config/snp/sync.toml` |
| `get_usage_path()` | `~/.config/snp/usage.toml` |
| `get_themes_dir()` | `~/.config/snp/themes/` |
| `get_themes_config_path()` | `~/.config/snp/themes.toml` |
| `get_log_dir()` | `~/.config/snp/logs/` |
| `get_audit_log_path()` | `~/.config/snp/audit.log` |
| `get_backup_dir()` | `~/.config/snp/backups/` |
| `get_premade_dir()` | `~/.config/snp/premade/` |
| `get_auto_sync_status_path()` | `~/.config/snp/auto-sync-status.toml` |
| `get_auto_sync_pending_path()` | `~/.config/snp/auto-sync-pending.toml` |
| `get_transaction_journals_dir()` | `~/.config/snp/transaction-journals/` |

## Security

- Config directory created with `0o700` permissions on Unix
- `ensure_config_dir()` tightens existing directory permissions if needed
