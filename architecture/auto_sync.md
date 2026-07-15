# Auto-Sync Architecture

Deep-dive reference for the auto-sync subsystem. For the full sync protocol and merge strategy, see [sync.md](sync.md).

## Overview

Auto-sync is an optional, opt-in background synchronization mechanism (Release 5A–5C). It is **disabled by default** and must be explicitly enabled via `snp sync config --auto-sync on`.

When enabled, mutation commands (`new`, `edit`, `import`, `delete`, `library create/delete`) trigger a debounced background sync after the local change is committed. The core invariant: **local mutations always succeed before any remote work begins.**

## Canonical Data Flow

```text
mutation command
  -> validate input
  -> atomic local commit (save_library / save_snippets)
  -> audit + local success
  -> notify_mutation(kind, origin)
  -> AutoSyncPolicy::resolve()
  -> origin check (SyncMerge → suppress)
  -> run_auto_sync()
     -> CoordinatorLock::acquire()
     -> load pending state
     -> retry loop with backoff
        -> sync_commands::run_default_sync()
        -> on success: clear pending, return Succeeded
        -> on failure: record failure class, continue retry
     -> exhausted: apply failure policy (Ignore / Warn / Error)
  -> return AutoSyncNotificationResult
```

**Key point:** All mutation commands use a single central API (`notify_mutation` / `notify_local_mutation`). No command calls separate ad-hoc auto-sync logic.

## Module

All auto-sync logic lives in `src/auto_sync.rs`. It contains:

- **Policy resolution** (`AutoSyncPolicy::resolve`)
- **Coordinator state machine** (`DebounceState`, `AutoSyncCoordinator`)
- **Mutation notification API** (`notify_mutation`, `notify_local_mutation`)
- **Cross-process locking** (`CoordinatorLock`)
- **Durable pending state** (`PendingState`, CRC32 integrity)
- **Retry/backoff** (exponential, bounded)
- **Failure policy rendering** (Ignore, Warn, Error)

## Key Types

### AutoSyncPolicy

```rust
pub struct AutoSyncPolicy {
    pub enabled: bool,
    pub debounce: Duration,
    pub failure_mode: AutoSyncFailureMode,
    pub max_retries: u32,
    pub sync_timeout: Duration,
}
```

Resolved once per command invocation from `SyncSettings`. Not `Send` or `Sync` — single-threaded per invocation.

### MutationKind

```rust
pub enum MutationKind {
    SnippetCreate,
    SnippetUpdate,
    SnippetDelete,
    Import,
    LibraryChange,
    PremadeInstall,
    SyncConflictWrite,
    AccountConfig,  // Never triggers sync
}
```

### MutationOrigin

```rust
pub enum MutationOrigin {
    User,       // User-initiated mutation
    Import,     // Import operation
    SyncMerge,  // Sync merge — NEVER triggers auto-sync (prevents loops)
    Recovery,   // Recovery operation
}
```

### AutoSyncNotificationResult

```rust
pub enum AutoSyncNotificationResult {
    Disabled,
    Suppressed,
    Executed(AutoSyncStatus),
}
```

## Trigger Matrix

| Operation | MutationKind | Remote-syncable | Auto-sync event | Notes |
|---|---:|---:|---:|---|
| new snippet | SnippetCreate | yes | yes | after save |
| edit command | SnippetUpdate | yes | yes | after editor save |
| output-only edit | SnippetUpdate | no | no | local-only field |
| delete/tombstone | SnippetDelete | yes | yes | after tombstone save |
| import dry-run | — | no | no | read-only |
| import merge no-op | — | no | no | no event |
| import create/replace | Import | yes | one | logical transaction |
| set primary library | — | no | no | local-only metadata |
| sync merge write | SyncConflictWrite | already sync | no | recursion suppressed |
| library create | LibraryChange | yes | yes | after library created |
| library delete | LibraryChange | yes | yes | after library deleted |
| premade get | — | local copy | no | no trigger |
| `snp sync` (explicit) | — | — | clears pending | prevents duplicate delayed sync |

## Debounce State Machine

```text
Idle ──────────────────────────────────────────────────────► Pending
  ◄──────────────────────────────────────────────────────── Running
Pending + mutation ──► Pending (updated deadline, bounded)
Pending + expired ───► Running
Running + mutation ──► Running (follow_up = true)
Running complete ────► Pending (short deadline) if follow_up
Running complete ────► Idle
```

- First mutation starts a debounce window (configurable, default 2s)
- Later mutations extend the deadline but never exceed the 300s maximum
- One sync runs after the quiet period
- Mutations during running schedule at most one follow-up
- Follow-up uses a 1-second short deadline

## Durable Pending State

**File:** `~/.config/snp/auto-sync-pending.toml`

```toml
# integrity: <crc32>
version = 1
pending = true
requested_at = 1234567890
last_attempt_at = 0
last_result = ""
library_id = null
```

- CRC32 integrity header on line 1
- Written atomically via `write_private_atomic`
- No secrets, commands, or snippet content in the file
- Stale pending (> 5 minutes) is cleared on recovery

## Cross-Process Locking

**File:** `~/.config/snp/auto-sync.lock`

Contains only the PID number. Uses `create_new(true)` for atomic creation.

- Stale detection via `kill -0` (Unix) — dead PID → lock removed
- Restrictive permissions (0o600)
- Advisory only — cannot block manual `snp sync`
- Non-Unix: fails open (all locks treated as stale)

## Retry and Backoff

- `max_retries`: default 1 (one retry after initial failure)
- Exponential backoff: 1s initial, doubling each retry, capped at 30s
- `sync_timeout`: per-attempt timeout (default 30s)
- Failed attempts record in durable pending state for diagnostics

## Failure Policy

| Mode | Behavior |
|------|----------|
| `Ignore` | Silent — debug-level log only, no user-facing output |
| `Warn` (default) | Stderr warning: `auto-sync failed: <reason>` |
| `Error` | Nonzero exit code, but local mutation remains committed |

`Warn`/`Error` produce user-visible messages because auto-sync runs synchronously within the calling command — the user is present to see stderr output.

## Local-Only Fields

The `output` field is local-only — not in `ProtoSnippet`, never uploaded or downloaded. Edits that change only the `output` field do NOT trigger auto-sync.

The `favorite` and `folders` fields are also local-only — preserved when server wins the merge conflict.

## Explicit Sync Precedence

When `--sync` flag is used (on `run`, `clip`, `search`, or TUI delete):

1. Explicit sync runs immediately via `run_default_sync()`
2. Pending auto-sync state is cleared via `clear_pending_after_explicit_sync()`
3. No duplicate delayed sync for the same mutation generation

## Design Decisions

### Architecture: In-Process (Option A)

The mutation command owns both debounce and sync execution. Simplest correct design. The process must remain alive for debounce + sync, which is acceptable since mutation commands can wait.

Alternatives evaluated and rejected:
- **Detached helper process** — adds IPC, process supervision, cross-platform complexity
- **Persistent daemon** — disproportionate for a CLI tool with no existing long-running process

### Sync Target: Global

`run_default_sync` syncs all configured libraries. The `library_id` field in `AutoSyncRequest` is vestigial — preserved for forward compatibility but currently unused.

### Delivery Guarantees: Best-Effort

Auto-sync is convenience, not durable delivery. The durable pending marker survives crash/restart, and `recover_stale_pending()` clears stale state (>5 minutes). Manual `snp sync` and cron remain the recovery path for missed syncs.

## Doctor Integration

`snp doctor --compatibility` inspects auto-sync state:
- Pending marker existence and staleness
- Lock file existence and owner liveness
- Auto-sync config settings (enabled/disabled, debounce, failure mode)

## Safety Invariants

1. Auto-sync is disabled by default
2. Local mutation commits before any remote work begins
3. Remote failure never rolls back or corrupts a successful local mutation
4. SyncMerge origin never triggers auto-sync (prevents loops)
5. All syncable mutations use one notification API
6. Triggers occur strictly after commit
7. No command payload, credentials, or encryption material in coordinator artifacts
8. Manual and scheduled sync remain independent
9. Local state survives every remote/scheduling failure
10. No auto-sync fields enter snippet TOML, ProtoSnippet, or import/export schema
