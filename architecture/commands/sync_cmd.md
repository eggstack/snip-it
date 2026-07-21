# sync_cmd — Sync with Server

## Overview

`sync_cmd` synchronizes local snippets with the snip-sync server. Handles bidirectional sync, conflict resolution, server library management, and auto-sync recovery commands.

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

### `snp sync retry`
Force an immediate sync attempt, bypassing backoff. Acquires the execution lock, runs sync, records success/failure, and clears pending on success.

```bash
snp sync retry [--library <name>]
```

**Behavior:**
1. Validates sync is enabled and API key is configured.
2. Acquires `SyncExecutionLock` via `wait_acquire` (30s timeout).
3. Reads the pending marker to capture the current generation.
4. Runs `run_sync()` with the configured direction.
5. On success: records success in status file, clears pending marker via `clear_if_generation_matches`.
6. On failure: records failure class and backoff in status file; pending marker preserved.

**When to use:** When `snp status` shows `Sync: attention required` or `Sync: pending retry` and you want to retry immediately without waiting for the next mutation or backoff window.

### `snp sync clear-failure`
Clear the failure state from the status file, resetting `attention_required`, `consecutive_failures`, and `next_attempt_at_unix_ms`.

```bash
snp sync clear-failure
```

**Behavior:**
1. Reads `auto-sync-status.toml`.
2. If corrupt: returns an error.
3. If missing: prints "No failure recorded".
4. If valid: sets `attention_required = false`, `consecutive_failures = 0`, `next_attempt_at_unix_ms = 0`, `message = "cleared by operator"`, writes back.

**When to use:** When you've fixed the underlying issue (e.g., updated API key, fixed network) and want to allow immediate retry without the status file blocking scheduling. This does NOT trigger a sync — it only clears the failure disposition so the next mutation or `snp sync retry` can proceed.

### `snp sync discard-pending`
Remove the pending sync marker, abandoning synchronization intent for the current generation.

```bash
snp sync discard-pending [--force] [--generation <N>]
```

**Behavior:**
1. Reads the pending marker to capture the current generation.
2. If no pending work: prints "No pending sync work" and exits.
3. If not `--force` and running in a terminal: prompts for confirmation.
4. If `--generation` specified and doesn't match observed: exits with error.
5. Calls `clear_if_generation_matches(observed_generation)` — if generation changed during the prompt, refuses to discard.
6. Records the discard outcome.

**When to use:** When you want to abandon sync intent — e.g., after a failed sync that you don't intend to retry, or when switching sync accounts. This never deletes local snippet data, only the pending marker.

**Safety:** The generation check prevents a stale discard from clobbering a newer mutation. If a mutation arrives between the prompt and the clear, the discard fails with "Generation changed".

### `snp sync repair`
Diagnose and repair corrupt auto-sync state.

```bash
snp sync repair [--dry-run] [--apply]
```

**Behavior (without `--apply`):** Lists all detected issues as dry-run actions:
- Corrupt status file → quarantine and recreate
- Stale execution lock (dead process) → remove
- Stale worker lock (dead process) → remove
- Stale pending transaction lock → remove
- Orphaned temp files (`snp-sync-tmp.*`, `.quarantine.*`) → remove
- Incorrect file permissions (not `0o600`) → fix

**Behavior with `--apply`:** Executes all detected repair actions:
- Corrupt files are moved to `.quarantine.{timestamp}/` before removal.
- Stale locks are quarantined and removed.
- Temp files are deleted.
- Permissions are set to `0o600` on Unix.

**When to use:** When `snp status` shows `Sync: corrupt or inaccessible state` or when `snp doctor` reports lock/status issues. Safe to run during active sync — repair actions only affect stale or corrupt artifacts.

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
- [auto_sync.md](../auto_sync.md) — Auto-sync policy, debounce, triggers
- [status.md](../status.md) — Status snapshot module
- [sync_commands.rs](../../sync_commands.rs) — Orchestration code
- [register_cmd.md](register_cmd.md) — Device registration
