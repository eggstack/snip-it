# Protobuf API

[← Back to Overview](overview.md)

## Overview

**Directory**: `snip-proto/`

Single `SnippetSync` gRPC service (`snip-proto/proto/sync.proto`, 216 lines)
plus checked-in tonic stubs (`snip-proto/src/snip_proto.rs`, via
`snip-proto/src/lib.rs`). Normal builds, packaging, and CI consume the
checked-in source and do not invoke `protoc`.

## Service: 11 RPCs

```protobuf
service SnippetSync {
    rpc GetSnippets (GetSnippetsRequest) returns (SnippetList);
    rpc PushSnippets (PushSnippetsRequest) returns (PushSnippetsResponse);
    rpc Sync (SyncRequest) returns (SyncResponse);
    rpc Health (HealthRequest) returns (HealthResponse);
    rpc Register (RegisterRequest) returns (RegisterResponse);
    rpc CreateLibrary (CreateLibraryRequest) returns (CreateLibraryResponse);
    rpc ListLibraries (ListLibrariesRequest) returns (ListLibrariesResponse);
    rpc DeleteLibrary (DeleteLibraryRequest) returns (DeleteLibraryResponse);
    rpc ListPremadeLibraries (ListPremadeLibrariesRequest) returns (ListPremadeLibrariesResponse);
    rpc GetPremadeLibrary (GetPremadeLibraryRequest) returns (GetPremadeLibraryResponse);
    rpc SearchPremadeLibraries (SearchPremadeLibrariesRequest) returns (SearchPremadeLibrariesResponse);
}
```

| RPC | Request | Response | Purpose |
|-----|---------|----------|---------|
| `GetSnippets` | `GetSnippetsRequest{since, library_id, limit, offset}` | `SnippetList{snippets, total_count, has_more}` | Non-deleted records with `updated_at` strictly greater than `since`; paginated |
| `PushSnippets` | `PushSnippetsRequest{library_id, snippets[]}` | `PushSnippetsResponse{success, message, accepted_count, rejected_count}` | Upload-only; invalid snippets rejected individually and reported |
| `Sync` | `SyncRequest{local_snippets[], last_sync_timestamp, library_id, limit, offset}` | `SyncResponse{success, message, snippets[], server_timestamp, skipped_count, skipped_ids[], has_more, total_count}` | Bidirectional merge: only locals newer than `last_sync_timestamp` are considered; returns changed server records **including deletion tombstones** |
| `Health` | `HealthRequest{}` | `HealthResponse{healthy, version}` | Liveness / version probe |
| `Register` | `RegisterRequest{device_id}` | `RegisterResponse{success, api_key, message, device_id}` | Device/account registration; server mints the API key |
| `CreateLibrary` | `CreateLibraryRequest{name}` | `CreateLibraryResponse{success, library_id, message}` | Create one library (IDs scoped to the account) |
| `ListLibraries` | `ListLibrariesRequest{limit, offset}` | `ListLibrariesResponse{libraries[], total_count, has_more}` | Paginated account library list |
| `DeleteLibrary` | `DeleteLibraryRequest{library_id}` | `DeleteLibraryResponse{success, message}` | Delete one library |
| `ListPremadeLibraries` | `ListPremadeLibrariesRequest{}` | `ListPremadeLibrariesResponse{libraries[], has_more, total_count}` | Server-to-client catalogue |
| `GetPremadeLibrary` | `GetPremadeLibraryRequest{filename}` | `GetPremadeLibraryResponse{success, name, content, message}` | Download one premade library by filename |
| `SearchPremadeLibraries` | `SearchPremadeLibrariesRequest{query}` | `SearchPremadeLibrariesResponse{libraries[], total_count}` | Catalogue search |

Notes:

- Every request still carries a deprecated `api_key` string field for wire
  compatibility, but current clients send it empty and authenticate via
  gRPC `authorization: Bearer <key>` metadata instead.
- Empty `library_id` selects the account default library. Zero `limit`
  selects the server default page size.
- `Sync` tombstones vs `GetSnippets`: only `Sync` returns `deleted = true`
  records; `GetSnippets` filters them out.

## `Snippet` (ProtoSnippet): 9 fields, no `output`

```protobuf
message Snippet {
    string id = 1;              // client-generated stable identifier
    string description = 2;     // plaintext, or empty when encrypted
    string command = 3;         // plaintext, or encrypted payload when encrypted
    repeated string tags = 4;   // plaintext, or empty when encrypted
    int64 created_at = 5;
    int64 updated_at = 6;       // last-write-wins clock
    string device_id = 7;       // deterministic tie-breaker
    bool deleted = 8;           // tombstone (Sync only)
    bool encrypted = 9;         // command holds the client-encrypted payload
}
```

The local `output` (notes) field is **not in `ProtoSnippet`** — it is
local-only, never uploaded or downloaded, and always preserved from the
local copy during merge (see [sync.md](sync.md)). Encrypted records carry
`{description, command, tags}` as JSON inside AES-256-GCM `command`, with
plaintext fields emptied.

Supporting messages: `Library{id, name, created_at, snippet_count}`,
`PremadeLibrary{name, filename, description, snippet_count, tags[]}`,
`SnippetList`, `SyncResponse` (with `server_timestamp` cursor,
`skipped_*`, `has_more`/`total_count` pagination).

## Size limits, pagination, idempotency

- Server gRPC default: 4 MiB max message (`GRPC_MAX_MESSAGE_SIZE`);
  per-field caps: command/description 1024, tags 50 × 100 chars, id and
  device_id 128, API key 512; `Sync` page cap 1000 (`MAX_REQUEST_LIMIT`);
  sync-set cap 10 000 snippets.
- Client stays under the server with a 3.5 MiB upload ceiling measured by
  Prost `encoded_len()` on the `SyncRequest` envelope; one oversized
  snippet fails as `RequestTooLarge` before any mutation (see
  [sync.md](sync.md)). Premade responses are capped client-side (10 000
  entries, 4 MiB content).
- Pagination: `limit`/`offset` + `has_more`; clients bound fetches
  (10 000-page guard) and treat `!has_more` or an empty page as end.
- Idempotency: `PushSnippets` upserts by snippet identity (`ON CONFLICT …
  WHERE newer`) — retries are safe. `Sync` merges by
  `(updated_at, device_id, SHA-256 synced fields)` with deletion-wins; see
  [sync.md](sync.md) for the conflict contract.

## Authentication

The `api_key` string field on requests is deprecated compatibility surface:
clients send it empty and authenticate via `authorization: Bearer <key>`
gRPC metadata (`add_api_key_metadata` in `src/sync.rs`). The server
enforces per-key rate limiting (`RateLimitExceeded` →
`ResourceExhausted`), validates timestamps against one `now` sample
(`CLOCK_SKEW:` rejections on `InvalidArgument`), and sanitizes internal
faults to generic `Internal error` / `Unauthenticated` statuses with detail
logged server-side only. See [sync.md](sync.md) and [server.md](server.md).

## Pagination and error semantics

- `GetSnippets` / `Sync` / `ListLibraries` paginate with `limit`/`offset` +
  `has_more`/`total_count`. Zero `limit` selects the server default
  (Sync page cap 1000); clients stop on `!has_more` or an empty page and
  bound runaway servers (10 000-page guard, `i32` saturation → explicit
  error rather than an infinite loop).
- `PushSnippetsResponse` reports per-snippet `accepted_count` /
  `rejected_count`; invalid snippets are rejected individually without
  failing the batch. `SyncResponse` carries `server_timestamp` (the next
  `last_sync` cursor), `skipped_count`/`skipped_ids` (encryption/decryption
  failures retried next run), and `has_more` for the download side.

## Code generation

The generated Rust module is committed at `snip-proto/src/snip_proto.rs`
— **never edit it by hand**. Maintainers regenerate explicitly with
`protoc` **only after editing `proto/sync.proto`**, review the diff, bump
the `snip-proto` version, propagate the bump to both dependents
(`snip-sync`, `snip-it`; publish order `snip-proto` → `snip-sync` →
`snip-it`), and commit the result.

### Re-export Pattern

```rust
// snip-proto/src/lib.rs
pub mod sync {
    include!("snip_proto.rs");
}
pub use sync::*;
```

Consumers import directly:

```rust
use snip_proto::{Snippet, SyncRequest, snippet_sync_client::SnippetSyncClient};
```

## Dependencies

| Crate | Purpose |
|-------|---------|
| `tonic` | gRPC framework |
| `prost` | Protocol Buffers implementation |
| `tonic-prost` | Runtime codec integration for generated stubs |

## Key Files

- `snip-proto/proto/sync.proto` — Service and message definitions (source of truth)
- `snip-proto/src/snip_proto.rs` — Checked-in generated stubs (do not edit)
- `snip-proto/src/lib.rs` — Module re-exports (`pub mod sync` + `pub use sync::*`)
- `snip-proto/Cargo.toml` — Dependencies (keep `snip-proto` version bumps in sync with dependents)
