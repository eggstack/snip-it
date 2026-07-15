# Sync Infrastructure (`sync.rs`, `sync_commands.rs`)

## Overview

Sync enables bidirectional synchronization of snippets between the local client and the snip-sync server. It uses gRPC for transport and implements end-to-end encryption (AES-256-GCM) for snippet data.

## Sync Client (`sync.rs`)

### SyncClient

Wraps the tonic gRPC client for the `SnippetSync` service defined in `snip-proto/`.

```rust
pub struct SyncClient {
    channel: Channel,
    retry_config: RetryConfig,
}
```

### Key Methods

- `sync_snippets()` — Full bidirectional sync
- `get_snippets()` — Pull from server
- `push_snippets()` — Push to server
- `register_device()` — Device registration
- `list_libraries()` / `create_library()` / `delete_library()` — Library management
- `list_premade()` / `get_premade()` — Premade libraries

### Retry Logic

The `retry_grpc!` macro implements exponential backoff with jitter for transient failures.

## Sync Orchestration (`sync_commands.rs`)

### run_sync()

Entry point for sync operations. Handles:
1. Load local snippets
2. Connect to server (with retry)
3. Determine sync direction
4. Pull → Merge → Push flow

### Sync Direction

```rust
pub enum SyncDirection {
    Push,           // Local → Server only
    Pull,           // Server → Local only
    Bidirectional,  // Both ways
}
```

## Merge Strategy (`merge_snippets()`)

**Last-write-wins** based on `updated_at` timestamp:

1. **Server wins** (server `updated_at` > local `updated_at`):
   - Server snippet replaces local (unless server `deleted: true`)
   - Local-only fields `output`, `folders`, `favorite` are **preserved**

2. **Local wins** (local `updated_at` > server `updated_at`):
   - Local snippet pushed to server

3. **Server deleted: true**:
   - Local copy marked `deleted: true` (data preserved, not shown in UI)
   - Never fully deleted to allow recovery

4. **Both deleted** (both have `deleted: true`):
   - Excluded from merged result

5. **Local-only** (snippet exists only locally or only server):
   - Preserved as-is

### Output Field Sync Contract

The `output` field is **local-only**: it is not in `ProtoSnippet`, never uploaded or downloaded, and always preserved from the local copy during merge. Another device will not receive the value automatically.

### Result

Merged snippets sorted by `updated_at` descending.

## Encryption

- **Key Derivation**: Argon2id from password/passphrase
- **Cipher**: AES-256-GCM
- **Payload**: `EncryptedPayload { salt, nonce, ciphertext }`
- `encrypt_snippet()` / `decrypt_snippet()` in `sync.rs`

## Protocol Buffers (`snip-proto/`)

Defines `SnippetSync` service:

```protobuf
service SnippetSync {
    rpc Sync(SyncRequest) returns (SyncResponse);
    rpc GetSnippets(GetRequest) returns (GetResponse);
    rpc PushSnippets(PushRequest) returns (PushResponse);
    rpc Register(RegisterRequest) returns (RegisterResponse);
    rpc CreateLibrary(CreateLibraryRequest) returns (CreateLibraryResponse);
    rpc ListLibraries(ListLibrariesRequest) returns (ListLibrariesResponse);
    rpc DeleteLibrary(DeleteLibraryRequest) returns (DeleteLibraryResponse);
    rpc ListPremadeLibraries(Empty) returns (ListPremadeLibrariesResponse);
    rpc GetPremadeLibrary(GetPremadeLibraryRequest) returns (GetPremadeLibraryResponse);
    rpc Health(HealthRequest) returns (HealthResponse);
}
```

## Settings

`~/.config/snp/sync.toml`:
- `server_url` — gRPC server address
- `api_key` — Stored in system keychain via `keyring` crate
- `direction` — Sync direction
- `interval` — Periodic sync interval (for cron)

## Error Handling

- `SnipError::Runtime` for sync-specific errors (sync failures, validation errors)
- `CryptoError` for encryption/decryption errors (converted to `SnipError::Runtime` via `From`)
- Network failures trigger retry with exponential backoff via `retry_grpc!` macro

## Auto-Sync Policy

**Module**: `src/auto_sync/policy.rs`

Auto-sync is disabled by default. When enabled via `snp sync config --auto-sync on`,
mutation commands spawn a detached one-shot worker (`snp auto-sync-worker`) that
performs the remote sync after the local change is committed. The parent returns
immediately — no in-process latency. The effective policy is resolved once per
command invocation via `AutoSyncPolicy::resolve()`.

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

### AutoSyncFailureMode

```rust
pub enum AutoSyncFailureMode {
    Ignore,  // Suppress user-facing failure
    Warn,    // Emit warning to stderr (default)
    Error,   // Nonzero exit code; local mutation still committed
}
```

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
    SyncMerge,  // Sync merge (NEVER triggers auto-sync — prevents loops)
    Recovery,   // Recovery operation
}
```

### Product Invariants

1. Auto-sync is disabled by default.
2. Local mutation commits before any remote work begins.
3. Remote failure never rolls back or corrupts a successful local mutation.
4. Existing `snp sync`, `snp cron`, daemon/service workflows remain unchanged.
5. Auto-sync never changes sync direction, credentials, server selection, library mapping, or conflict policy implicitly.
6. Machine-facing stdout remains free of background sync diagnostics.
7. Command bodies, output metadata, credentials, API keys, and encryption material are never included in auto-sync logs, errors, or worker artifacts.

## Auto-Sync Detached Worker (Release 5D corrective)

**Module**: `src/auto_sync/`

The detached one-shot worker replaces the earlier in-process coordinator. After
the parent mutation command commits the local change, it records a durable
pending marker, acquires a worker lock, and re-execs the current binary as
`snp auto-sync-worker` with platform-detached flags. The worker runs in its own
session and exits independently.

### Architecture

```text
Mutation command (parent)
  -> atomic local commit
  -> notify_mutation(kind, origin)
  -> AutoSyncPolicy::resolve()
  -> mark_pending(state_dir) -> PendingState{generation}
  -> try_acquire(state_dir) -> WorkerLock
  -> spawn::spawn_worker(current_exe, "auto-sync-worker", state_dir, nonce)
     -> setsid() (Unix) / DETACHED_PROCESS | CREATE_NO_WINDOW (Windows)
     -> stdin/stdout/stderr -> null
     -> child process detached
  -> mem::forget(WorkerLock) so lock file outlives parent
  -> return AutoSyncNotificationResult::Scheduled{generation}

snp auto-sync-worker (child, detached)
  -> AutoSyncPolicy::resolve(get_sync_settings())
  -> try_acquire(state_dir) -> WorkerLock (or AlreadyHeld -> NothingToDo)
  -> nonce_already_used? -> NothingToDo
  -> read_state_from_dir(state_dir) -> PendingState
  -> execute_sync(state_dir, &pending, &policy)
     -> run_with_timeout(run_default_sync, sync_timeout)
        -> on Ok: clear_if_generation_matches(state_dir, generation)
        -> on Err: record_failure(state_dir, generation, classification)
  -> exit(0)
```

### AutoSyncNotificationResult

```rust
pub enum AutoSyncNotificationResult {
    Disabled,
    Suppressed,
    Scheduled { generation: u64 },
    SchedulingFailed { generation: Option<u64> },
}
```

### WorkerOutcome

```rust
pub enum WorkerOutcome {
    Success,
    Failed,
    NothingToDo,
}
```

### FailureClass

```rust
pub enum FailureClass {
    Network,  // Timeout, DNS, connection refused
    Auth,     // Invalid API key, expired token
    Conflict, // Merge failure
    Unknown,  // Unclassified
}
```

Classified from `SnipError` via `FailureClass::from_error()`, or from a
`String` error message via `FailureClass::from_code()` (used by the worker
to label pending failures).

### Durable Pending State (Schema v2)

Persisted to `~/.config/snp/auto-sync-pending.toml` with CRC32 integrity:

```toml
schema = 2
generation = 1
created_at_unix_ms = 1700000000000

[snapshot.Mutation]
kind = "snippet_create"

integrity = "crc32:441c462e"
```

- Monotonic `generation` increments per `mark_pending`. Conditional clear keyed
  on observed generation prevents stale workers from clobbering fresh state.
- Schema v1 markers (`kind = "..."`, `created_at_unix_ms = ...`) migrate
  transparently to v2 on load.
- `created_at_unix_ms` is used by `startup_recover_pending` to clear stale
  markers older than 5 minutes.
- `integrity` is `crc32:<hex>` over the serialized snapshot.
- Atomic write via `tempfile + rename + fsync`.
- 0o600 permissions on Unix. No secrets, commands, or snippet content.

### Cross-Process Worker Lock

TOML lock at `~/.config/snp/auto-sync-worker.lock`:

```toml
pid = 12345
started_at_unix_ms = 1700000000000
nonce = "abc-12345-def"
```

- Atomic acquisition via `OpenOptions::create_new(true)` — only one parent wins.
- Stale detection: `kill -0 pid` on Unix (dead process → stale) plus
  `>5 minute` age threshold.
- Stale locks are reclaimed transparently by `try_acquire`.
- RAII `Drop` removes the file when the worker exits.
- The parent `mem::forget`s the lock after successful spawn so the lock file
  outlives the parent and the worker can detect it via `inspect`.
- Restrictive permissions (0o600 on Unix).
- **Platform note:** `kill -0` is Unix-only. On non-Unix platforms the lock
  check is conservative (treats non-existent processes as alive), and the
  5-minute age threshold still applies.

### Process Detachment

- Unix: `libc::setsid()` puts the worker in a new session, ensuring it does
  not die when the parent exits and has no controlling terminal.
- Windows: `DETACHED_PROCESS | CREATE_NO_WINDOW` flags on `CreateProcess`.
- `stdin`/`stdout`/`stderr` are routed to `null` so the worker cannot interfere
  with the parent's TTY.

### Failure Policy Rendering

When the parent fails to spawn the worker:
- `Ignore`: debug-level log only, no user-facing output
- `Warn`: `eprintln!` warning to stderr
- `Error`: `eprintln!` error to stderr + nonzero exit code

Worker-side failures are logged to `~/.config/snp/logs/` and surface via
`snp doctor --compatibility` diagnostics. The user is no longer present when
the worker runs, so stderr is not the appropriate channel.

### Design Decisions

**Architecture: detached one-shot worker (corrective)**

Three options were evaluated:
1. **Option A (in-process coordinator):** Initial design. The mutation command
   owns debounce and sync execution. Adds visible latency to mutation commands
   and holds the parent process hostage during network round-trips.
2. **Option B (persistent daemon):** Rejected. snp is a CLI tool with no
   existing long-running process; a daemon would require lifecycle, IPC, and
   uninstall handling disproportionate to the use case.
3. **Option C (detached one-shot worker):** Chosen (Release 5D corrective).
   The parent re-execs itself as a hidden `auto-sync-worker` subcommand with
   detached process flags. Zero IPC, portable across Unix and Windows, reuses
   the same `snp` binary's sync code path. The user never waits on network
   round-trips.

**Sync target: Global (not per-library)**

`run_default_sync` syncs all configured libraries. The `MutationContext::library_id`
field is retained for forward compatibility but currently unused. Per-library
targeting deferred until the sync protocol supports it.

**Delivery guarantees: Best-effort**

Auto-sync is convenience, not durable delivery. The durable pending marker
survives crash/restart, and `startup_recover_pending` clears stale state
(>5 minutes). Manual `snp sync` and cron remain the recovery path for missed
syncs.

### Doctor Integration

`snp doctor --compatibility` inspects auto-sync state using `auto_sync::paths`:
- `paths::state_dir()` — directory containing all auto-sync artifacts.
- `paths::pending_marker()` — full path to the pending TOML.
- `paths::worker_lock()` — full path to the worker lock TOML.
- Liveness probe uses `lock::process_alive(pid)` (`kill -0` on Unix).

Diagnostics emitted:
- `compat.auto_sync.enabled` / `compat.auto_sync.disabled` — policy state.
- `compat.auto_sync.pending_active` / `compat.auto_sync.pending_stale` /
  `compat.auto_sync.pending_unreadable` — pending marker status.
- `compat.auto_sync.lock_held` / `compat.auto_sync.lock_stale` /
  `compat.auto_sync.lock_unreadable` — worker lock status.

### Safety Invariants

1. Worker never mutates snippet libraries directly (only calls `run_default_sync`).
2. Secrets and snippet content never enter pending markers, lock files, worker argv, or worker env.
3. SyncMerge origin never triggers auto-sync (prevents loops).
4. PID+nonce worker lock prevents concurrent worker executions across processes.
5. Pending marker survives crash; stale markers (>5 min) cleared on startup recovery.
6. Manual and scheduled sync remain independent; explicit sync clears pending.
7. No new visible CLI surface added — `auto-sync-worker` is hidden.
8. Pending marker schema is versioned (v2) with CRC32 integrity.
9. Conditional clear keyed on observed generation prevents stale workers from
   clobbering fresh state.

## Auto-Sync Mutation Trigger Integration (Release 5C)

**Module**: `src/auto_sync/notification.rs`

Release 5C wires all syncable local mutations into the auto-sync coordinator
via the central mutation notification API. Auto-sync is now operational —
it triggers automatically after successful local mutations when enabled.

### Central Mutation Notification API

```rust
pub fn notify_mutation(kind: MutationKind, origin: MutationOrigin) -> AutoSyncNotificationResult
```

Convenience function for mutation commands. Loads sync settings, resolves
the policy, and calls `notify_local_mutation()`. Use this after a successful
local atomic write.

```rust
pub fn notify_local_mutation(
    policy: &AutoSyncPolicy,
    context: MutationContext,
) -> AutoSyncNotificationResult
```

Low-level function that takes a pre-resolved policy. Used for testing.

```rust
pub struct MutationContext {
    pub kind: MutationKind,
    pub origin: MutationOrigin,
    pub library_id: Option<String>,
}
```

### Mutation Flow

```text
user command
  -> validate
  -> local atomic write
  -> audit/local success
  -> notify_mutation(kind, origin)
  -> AutoSyncPolicy::resolve() + origin check
  -> mark_pending(state_dir) -> PendingState{generation}
  -> spawn::spawn_worker(current_exe, "auto-sync-worker", state_dir, nonce)
  -> return AutoSyncNotificationResult::Scheduled{generation}
```

### Command Trigger Matrix

| Command | Mutation | Origin | Triggers? | Notes |
|---------|----------|--------|-----------|-------|
| `snp new` (all sources) | SnippetCreate | User | Yes | After atomic save |
| `snp edit` (editor) | SnippetUpdate | User | Yes | After editor closes |
| `snp edit --output/--clear-output` | SnippetUpdate | User | **No** | Output is local-only |
| TUI delete | SnippetDelete | User | Yes | After tombstone save |
| `snp import pet` (create) | Import | Import | Yes | After library + config saved |
| `snp import pet` (merge, changed) | Import | Import | Yes | Only if imported > 0 |
| `snp import pet` (replace) | Import | Import | Yes | After replacement saved |
| `snp import pet` (dry-run) | — | — | **No** | Read-only |
| `snp import pet` (no-op merge) | — | — | **No** | Nothing changed |
| `snp library create` | LibraryChange | User | Yes | After library created |
| `snp library delete` | LibraryChange | User | Yes | After library deleted |
| `snp library set-primary` | — | — | **No** | Local-only metadata |
| `snp premade get` | — | — | **No** | Local copy of remote data |
| `snp sync` (manual) | — | — | Clears pending | Explicit sync clears auto-sync state |
| Sync merge writes | SyncConflictWrite | SyncMerge | **No** | Prevents feedback loops |

### Explicit Sync Precedence

When `--sync` flag is used (on `run`, `clip`, `search`, or TUI delete):

1. Explicit sync runs immediately via `run_default_sync()`
2. Pending auto-sync state is cleared via `clear_pending_after_explicit_sync()`
3. No duplicate delayed sync for the same mutation generation

### Transaction Boundaries

Each command defines its authoritative commit point:

- **`snp new`**: After `save_library()` or `save_snippets()` succeeds
- **`snp edit` (editor)**: After editor process exits successfully
- **`snp edit --output`**: After `save_library()` succeeds (but no sync trigger)
- **TUI delete**: After `save_library()` succeeds
- **`snp import pet`**: After library file saved AND library registered in config
- **`snp library create/delete`**: After library manager operation succeeds

Auto-sync is submitted only after all local state required for a consistent
view has committed. Backup failure does not trigger sync.

### Local-Only Fields

The `output` field is local-only — not in `ProtoSnippet`, never uploaded
or downloaded. Edits that change only the `output` field do NOT trigger
auto-sync because there is nothing to sync remotely.

### Product Invariants (Release 5C additions)

8. All syncable user mutation paths use one notification API.
9. Triggers occur strictly after commit.
10. Dry-run, cancel, failure, and no-op paths emit no request.
11. Local-only mutations follow explicit protocol scope.
12. Explicit/manual sync does not cause duplicate delayed sync.
13. Sync-origin writes cannot recurse.
14. Local state survives every remote/scheduling failure.
15. Tests prove exactly-once logical notification and clean stdout.
