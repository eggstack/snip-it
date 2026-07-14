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

**Module**: `src/auto_sync.rs`

Auto-sync is disabled by default. When enabled via `snp sync config --auto-sync on`,
mutation commands trigger a debounced background sync after the local change is
committed. The effective policy is resolved once per command invocation via
`AutoSyncPolicy::resolve()`.

### AutoSyncPolicy

```rust
pub struct AutoSyncPolicy {
    pub enabled: bool,
    pub debounce: Duration,
    pub failure_mode: AutoSyncFailureMode,
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

### Product Invariants

1. Auto-sync is disabled by default.
2. Local mutation commits before any remote work begins.
3. Remote failure never rolls back or corrupts a successful local mutation.
4. Existing `snp sync`, `snp cron`, daemon/service workflows remain unchanged.
5. Auto-sync never changes sync direction, credentials, server selection, library mapping, or conflict policy implicitly.
6. Machine-facing stdout remains free of background sync diagnostics.
7. Command bodies, output metadata, credentials, API keys, and encryption material are never included in auto-sync logs or errors.

## Auto-Sync Coordinator (Release 5B)

**Module**: `src/auto_sync.rs`

The coordinator extends the policy model with a stateful debounce engine, durable
pending markers, and PID-file based cross-process locking. It provides infrastructure
only — no mutation command is wired until Release 5C.

### Architecture

```text
Mutation ──► AutoSyncCoordinator::request()
                 │
                 ├─ suppress if origin == SyncMerge
                 ├─ suppress if policy.disabled
                 ├─ update DebounceState
                 ├─ persist PendingState (durable marker)
                 └─ return AutoSyncStatus

Timer / caller ──► AutoSyncCoordinator::tick()
                      │
                      ├─ DebounceState::Pending expired?
                      │     └─► Acquire CoordinatorLock
                      │         ├─ lock held → Running
                      │         └─ lock denied → Pending (retry)
                      └─ DebounceState::Running complete?
                            ├─ follow_up → Pending (short deadline)
                            └─ no follow_up → Idle, clear pending
```

### AutoSyncRequest

```rust
pub struct AutoSyncRequest {
    pub library_id: Option<String>,
    pub mutation_kind: MutationKind,
    pub requested_at: i64,
}
```

Contains no snippet content, credentials, or encryption material.

### MutationOrigin

```rust
pub enum MutationOrigin {
    User,       // User-initiated mutation
    Import,     // Import operation
    SyncMerge,  // Sync merge (NEVER triggers auto-sync — prevents loops)
    Recovery,   // Recovery operation
}
```

### AutoSyncStatus

```rust
pub enum AutoSyncStatus {
    Disabled,
    Pending,
    Running,
    Succeeded { completed_at: i64 },
    Failed { completed_at: i64, class: FailureClass },
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

Classified from `SnipError` via `FailureClass::from_error()`.

### Debounce State Machine

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

### Durable Pending State

Persisted to `~/.config/snp/auto-sync-pending.toml` with CRC32 integrity:

```toml
# integrity: <crc32>
version = 1
pending = true
requested_at = 1234567890
last_attempt_at = 0
last_result = ""
```

- Survives process crash/restart
- Stale pending (> 5 minutes) is cleared on recovery
- No secrets, commands, or snippet content in the file

### Cross-Process Locking

PID-file based lock at `~/.config/snp/auto-sync.lock`:
- Atomic creation via `create_new(true)`
- Stale detection via `kill -0` (Unix) — dead PID → lock removed
- Restrictive permissions (0o600)
- Advisory only — cannot block manual `snp sync`

### Retry and Backoff

`run_auto_sync` supports configurable retry with exponential backoff:
- `max_retries`: default 1 (one retry after initial failure)
- Exponential backoff: 1s initial, doubling each retry, capped at 30s
- `sync_timeout`: per-attempt timeout (default 30s, configurable)
- Failed attempts record in the durable pending state for diagnostics

### Failure Policy Rendering

When all retry attempts are exhausted:
- `Ignore`: debug-level log only, no user-facing output
- `Warn`: `eprintln!` warning to stderr + tracing log
- `Error`: `eprintln!` error to stderr + tracing error log

The `Warn`/`Error` modes produce user-visible messages because auto-sync
runs synchronously within the calling command — the user is present to
see stderr output.

### Design Decisions

**Architecture: Option A (in-process coordinator)**

Three options were evaluated:
1. **Option A (in-process):** Chosen. The mutation command owns debounce
   and sync execution. Simplest correct design. The process must remain
   alive for debounce + sync, which is acceptable since mutation commands
   can wait.
2. **Option B (detached helper process):** Rejected. Adds significant
   complexity (IPC, process supervision, cross-platform detachment) for
   marginal benefit over in-process sync.
3. **Option C (persistent daemon):** Rejected. snp is a CLI tool with no
   existing long-running process; a daemon is disproportionate.

**Sync target: Global (not per-library)**

`run_default_sync` syncs all configured libraries. The `library_id` field
in `AutoSyncRequest` is vestigial — preserved for forward compatibility
but currently unused. Per-library targeting deferred until the sync
protocol supports it.

**Delivery guarantees: Best-effort**

Auto-sync is convenience, not durable delivery. The durable pending
marker survives crash/restart, and `recover_stale_pending()` clears
stale state (>5 minutes). Manual `snp sync` and cron remain the
recovery path for missed syncs.

### Doctor Integration

`snp doctor --compatibility` inspects auto-sync state:
- Pending marker existence and staleness
- Lock file existence and owner liveness
- Auto-sync config settings (enabled/disabled, debounce, failure mode)

### Safety Invariants

1. Coordinator never mutates snippet libraries directly
2. Secrets and snippet content never enter coordinator state or logs
3. SyncMerge origin never triggers auto-sync (prevents loops)
4. Lock prevents concurrent sync executions
5. Pending marker survives crash for recovery
6. Manual and scheduled sync remain independent
7. No new CLI surface added (infrastructure only)
