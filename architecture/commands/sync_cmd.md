# sync_cmd — Sync with Server

## Overview

`sync_cmd` synchronizes local snippets with the snip-sync server. Handles bidirectional sync, conflict resolution, and server library management.

## Entry Point

```rust
pub fn run(matches: &ArgMatches) -> SnipResult<()>
```

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

## Flags

- `--servers` — List server libraries only
- `--local` — Local-only sync mode
- `--interval <seconds>` — Set periodic sync interval

## Settings

Sync settings loaded from `~/.config/snp/sync.toml`:
- `server_url` — gRPC server address
- `api_key` — System keychain via `keyring`
- `direction` — `push`, `pull`, or `bidirectional`
- `interval` — Periodic sync interval

## Related

- [sync.md](../sync.md) — Full sync protocol and merge details
- [sync_commands.rs](../../sync_commands.rs) — Orchestration code
- [register_cmd.md](register_cmd.md) — Device registration
