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
