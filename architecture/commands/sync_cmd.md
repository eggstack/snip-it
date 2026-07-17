# sync_cmd — Sync with Server

## Overview

`sync_cmd` synchronizes local snippets with the snip-sync server. Handles bidirectional sync, conflict resolution, and server library management.

## Entry Point

```rust
pub fn run(options: SyncOptions, runtime: &tokio::runtime::Runtime) -> SnipResult<()>
```

## Sync Execution Lock

The sync command acquires `SyncExecutionLock` via `wait_acquire` with a 30-second
timeout (foreground behavior). This serializes manual sync with the detached
auto-sync worker and any other concurrent sync caller.

Before running sync, the command captures the observed pending generation via
`observe_pending_generation()`. After sync succeeds (or fails), it clears the
pending marker via `clear_pending_after_explicit_sync(observed_generation,
sync_succeeded)` — generation-safe clearing that prevents a stale manual sync
from clobbering a newer pending mutation.

This means any manual `snp sync` call clears any pending auto-sync intent,
preventing duplicate delayed sync attempts for the same mutation generation.

## Sync Modes

### Local-Only Sync (`--local`)
```bash
snp sync --local
```
- Reads `~/.config/snp/sync.toml` for settings
- Performs sync without server (local-only backup/snapshot)

### Server Libraries (`--servers`)
```bash
snp sync --servers
```
- Lists available libraries on the server
- Does not perform actual sync

### Bidirectional Sync (default)
```bash
snp sync
```
1. Connect to server via gRPC
2. Load local snippets
3. Pull snippets from server
4. Merge using last-write-wins strategy
5. Push merged result to server
6. Save merged result locally

## Merge Logic

See [sync.md](../sync.md) for full merge strategy details.

Key points:
- **Last-write-wins** based on `updated_at`
- **Server deleted: true** → local copy marked deleted
- **Local-only preserved** — `output`, `folders`, `favorite` kept when server wins

## Subcommands

### `snp sync` (default)
Run a sync operation. Supports bidirectional, push-only, pull-only, and dry-run modes.

### `snp sync config`
View or update auto-sync policy settings.

Flags:
- `--show` — Display current auto-sync configuration
- `--auto-sync <on|off>` — Enable or disable auto-sync after mutations
- `--debounce <secs>` — Debounce delay in seconds (0-300)
- `--failure <ignore|warn|error>` — Failure behavior

Examples:
```bash
snp sync config --show
snp sync config --auto-sync on
snp sync config --debounce 5 --failure warn
```

## Legacy Flags (on `snp sync`)

- `--servers` — List server libraries only
- `--push-only` — Upload local changes only
- `--pull-only` — Download remote changes only
- `--dry-run` — Show what would be synced
- `--library` — Sync a specific library

## Settings

Sync settings loaded from `~/.config/snp/sync.toml`:
- `server_url` — gRPC server address
- `api_key` — System keychain via `keyring`
- `direction` — `push`, `pull`, or `bidirectional`
- `interval` — Periodic sync interval

### Auto-Sync Policy

Configured via `snp sync config`:
- `auto_sync` — Enable/disable auto-sync (default: off)
- `auto_sync_debounce_seconds` — Delay before sync fires (default: 2, range: 0-300)
- `auto_sync_failure` — Failure mode: ignore, warn (default), or error

## Related

- [sync.md](../sync.md) — Full sync protocol and merge details
- [sync_commands.rs](../../sync_commands.rs) — Orchestration code
- [register_cmd.md](register_cmd.md) — Device registration
