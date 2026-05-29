# Sync System

[← Back to Overview](overview.md)

## Overview

The sync system enables bidirectional snippet synchronization between the CLI client and a remote `snip-sync` server. All data is encrypted end-to-end using AES-256-GCM.

## gRPC Client

**File**: `src/sync.rs` (384 lines)

### `SyncClient`

Wraps a tonic gRPC client with encryption handling:

```rust
pub struct SyncClient {
    client: SnippetSyncClient<Channel>,
    settings: SyncSettings,
}
```

### Connection

- TLS with native certificate roots
- 10s connect timeout, 30s request timeout
- HTTP/2 assumed for gRPC

### Retry Strategy

Exponential backoff for all gRPC calls:
- Max 3 retries
- Initial delay: 100ms
- Max delay: 5000ms
- Doubles each attempt

### Methods

| Method | Description |
|--------|-------------|
| `create(settings)` | Connect to server with TLS |
| `sync_encrypted(local, last_sync, library_id)` | Full bidirectional sync |
| `health_check()` | Server health probe |
| `register(server_url)` | Create new account, get API key |
| `list_libraries()` | List server-side libraries |
| `create_library(name)` | Create library on server |
| `list_premade_libraries()` | List available premade libraries |
| `get_premade_library(filename)` | Download premade library content |

### Encryption Flow

Before sending snippets to server:
1. Serialize snippet data (description, command, tags) as JSON
2. Encrypt JSON with user's API key via `encryption::encrypt()`
3. Send encrypted blob in `command` field with `encrypted: true`

On receipt:
1. Check `encrypted` flag
2. If true, decrypt with `encryption::decrypt()`
3. Deserialize JSON back to snippet fields

## Sync Commands

**File**: `src/sync_commands.rs` (680 lines)

Orchestration layer between CLI commands and the gRPC client.

### `run_sync()`

Main sync entry point:

1. Validate sync is configured (enabled + API key)
2. Create sync client, check server health
3. Resolve libraries to sync (single or all)
4. For each library:
   - Load local snippets
   - Generate UUIDs for snippets missing IDs
   - Push changed local snippets (if Push or Bidirectional)
   - Pull server snippets (if Pull or Bidirectional)
   - Merge using last-write-wins strategy
   - Save merged result
   - Update last_sync timestamp

### Merge Strategy

```rust
fn merge_snippets(local: &Snippets, server: &[ProtoSnippet]) -> Snippets
```

Rules:
1. **Server-deleted** → Mark local copy as `deleted: true` (preserve data)
2. **Both deleted** → Exclude entirely
3. **Server newer** (`updated_at > local.updated_at`) → Server wins, preserve local-only fields (`output`, `folders`, `favorite`)
4. **Local newer or equal** → Local wins
5. **Server-only** → Add to local
6. **Local-only** → Keep in local
7. **Sort** → By `updated_at` descending

### Local-Only Fields

These fields are never synced to the server and are preserved when server wins the merge:
- `output` — Output file path
- `folders` — Folder organization
- `favorite` — Starred flag

### Library Linking

`sync_cmd.rs` handles linking local libraries to server libraries:
- Lists server libraries
- Creates local library files for server libraries
- Links via `library_id` in `libraries.toml`
- Conflict resolution: skip, overwrite with server, or rename local

### Premade Library Sync

`run_premade_sync()` — Downloads missing premade libraries from server:
- Lists available premade libraries
- Skips already-downloaded ones
- Saves to `~/.config/snp/premade/`

## Key Files

- `src/sync.rs` — gRPC client, TLS setup, retry logic, encrypt/decrypt wrappers
- `src/sync_commands.rs` — Sync orchestration, merge logic, library linking
- `src/commands/sync_cmd.rs` — CLI sync command, server library linking
- `src/commands/premade_cmd.rs` — Premade library browsing/downloading
- `src/commands/register_cmd.rs` — Account registration
