# status_cmd — Auto-Sync Status Display

[← Back to Overview](../overview.md)

## Purpose

`status` displays the current auto-sync state, local library summary, and last/next sync attempt information.

**File**: `src/commands/status_cmd.rs`

## Displayed Information

### Local Summary

- Number of libraries and snippets
- Primary library name

### Sync State

One of eight `TopLevelSyncState` variants:

| State | Display |
|-------|---------|
| `NotConfigured` | "Sync: not configured" |
| `ConfiguredAutoSyncDisabled` | "Sync: auto-sync disabled" |
| `ConfiguredAndCurrent` | "Sync: current" |
| `PendingAwaitingScheduling` | "Sync: pending" |
| `PendingRetryBackoff` | "Sync: pending retry" |
| `PendingAttentionRequired` | "Sync: attention required" |
| `LiveExecution { pid }` | "Sync: active (pid=N)" |
| `CorruptOrInaccessible` | "Sync: corrupt or inaccessible state" |

### Attempt Info

- Last attempt timestamp and failure class
- Next retry time (if in backoff)
- Pending generation number (if pending)

### Log Directory

Path to the log directory for troubleshooting.

## Output Formats

- Default: human-readable text
- `--json`: machine-readable JSON (`StatusSnapshot`)
- `--sync-only`: omit local summary, show only sync state

## Data Source

Uses `status_snapshot::capture_snapshot()` which aggregates:
- Pending state (`auto-sync-pending.toml`)
- Execution lock status
- Durable sync status (`auto-sync-status.toml`)
- Sync settings (`sync.toml`)
