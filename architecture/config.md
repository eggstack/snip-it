# Configuration

[← Back to Overview](overview.md)

## Config Directory Resolution

**File**: `src/utils/config.rs`

### Directory Layout

```
~/.config/snp/                    # XDG_CONFIG_HOME or ~/.config
├── snippets.toml                 # Legacy single-file (migrated)
├── libraries.toml                # Library metadata
├── sync.toml                     # Sync settings
├── audit.log                     # Audit trail
├── libraries/                    # Individual library files
│   ├── snippets.toml
│   ├── work.toml
│   └── personal.toml
├── premade/                      # Downloaded premade libraries
│   ├── docker.toml
│   └── git.toml
├── logs/                         # Rolling log files (daily rotation)
│   └── snp.log.2026-05-29
└── backups/                      # Timestamped backups
    └── snippets.20260529_120000.toml.bak
```

### Resolution Logic

```rust
fn get_config_dir() -> PathBuf {
    // 1. $XDG_CONFIG_HOME/snp (if set)
    // 2. ~/.config/snp (default)
}
```

### macOS Migration

Old macOS path (`~/Library/Application Support/snp/`) is automatically migrated to `~/.config/snp/` on first run. Files are moved, not copied; legacy dir is removed if empty.

## Sync Settings

**File**: `src/config.rs`

### `SyncSettings` struct

```rust
pub struct SyncSettings {
    pub enabled: bool,                    // Default: false
    pub server_url: String,               // Default: "http://localhost:50051"
    pub api_key: String,                  // From registration
    pub device_id: String,                // From registration
    pub sync_interval_minutes: u32,       // Default: 30
    pub auto_sync: bool,                  // Default: false
    pub sync_direction: SyncDirection,    // Default: Push
    pub clipboard_auto_clear_seconds: Option<u32>,
}
```

### `SyncDirection` enum

```rust
pub enum SyncDirection {
    Push,           // Local → Server only
    Pull,           // Server → Local only
    Bidirectional,  // Both directions (merge)
}
```

### TOML Format

Stored in `~/.config/snp/sync.toml`:

```toml
[sync]
enabled = true
server_url = "https://sync.example.com"
api_key = "your-api-key"
device_id = "device-uuid"
sync_interval_minutes = 30
auto_sync = false
sync_direction = "Bidirectional"
clipboard_auto_clear_seconds = 30
```

### Load/Save

- `load_sync_settings()` — Reads `sync.toml`, falls back to defaults
- `save_sync_settings()` — Writes with backslash-safe quoting
- `get_sync_settings()` — Convenience wrapper, never fails

## Environment Variables

| Variable | Used By | Default |
|----------|---------|---------|
| `XDG_CONFIG_HOME` | `utils/config.rs` | `~/.config` |
| `SNP_THEME` | `ui.rs` | `"auto"` |
| `COLORFGBG` | `ui.rs` (theme detection) | — |
| `SHELL` | `run_cmd.rs` | `"sh"` |
| `EDITOR` | `edit_cmd.rs` | `"vim"` |
| `RUST_LOG` | `tracing-subscriber` | `"snp=info,warn"` |

## Key Files

- `src/utils/config.rs` — Config directory paths, macOS migration
- `src/config.rs` — SyncSettings struct, load/save, SyncDirection
