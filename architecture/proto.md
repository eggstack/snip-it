# snip-proto — The Sync Wire Protocol

[← Back to Overview](overview.md)

## Overview

`snip-proto` is the smallest workspace crate: the Protobuf schema for the
snip-it sync service and the **checked-in** tonic/prost stubs generated from
it. It is the contract between the `snip-it` client and the `snip-sync`
server.

**Files**:

| File | Lines | Role |
|------|-------|------|
| `snip-proto/proto/sync.proto` | 222 | The schema — the source of truth |
| `snip-proto/src/snip_proto.rs` | 1334 | The **generated** stubs (checked in, never hand-edited) |
| `snip-proto/src/lib.rs` | 5 | The crate root |

**Not covered here:** the client-side gRPC wrapper and retry policy
(`src/sync.rs`), the merge and conflict-resolution rules
(`src/sync_commands.rs`), the server implementation (`snip-sync/src/lib.rs`),
and the encryption of synced payloads (`.skills/encryption-module.md`).

The crate has **no dependencies beyond the tonic/prost stack** and **no
`build.rs`** — see [Code Generation](#code-generation).

## Crate Shape

```rust
// snip-proto/src/lib.rs:1-5
pub mod sync {
    include!("snip_proto.rs");
}

pub use sync::*;
```

The generated file is `include!`d into a `sync` module and then re-exported
flat, so consumers write `snip_proto::Snippet` — not
`snip_proto::sync::Snippet`.

In the client crate the whole crate is aliased:

```rust
// src/lib.rs:43
pub(crate) use snip_proto as proto;
```

so client code uses `crate::proto::Snippet` (usually imported with an alias:
`use crate::proto::Snippet as ProtoSnippet;` — `src/sync_commands.rs:12`).

The server crate depends on `snip-proto` directly by path
(`snip-sync/Cargo.toml:58`).

## The SnippetSync Service

```protobuf
// snip-proto/proto/sync.proto:7
service SnippetSync {
```

The package is `snip_proto` (`snip-proto/proto/sync.proto:3`), so the fully
qualified service name is `snip_proto.SnippetSync`
(`snip-proto/src/snip_proto.rs:1330`).

### The 11 RPCs

| # | RPC | Request | Response | Server impl | Used by the `snip-it` client |
|---|-----|---------|----------|-------------|-------------------------------|
| 1 | `GetSnippets` | `GetSnippetsRequest` | `SnippetList` | `snip-sync/src/lib.rs:1070` | **No** |
| 2 | `PushSnippets` | `PushSnippetsRequest` | `PushSnippetsResponse` | `snip-sync/src/lib.rs:1181` | Yes — `src/sync.rs:938` |
| 3 | `Sync` | `SyncRequest` | `SyncResponse` | `snip-sync/src/lib.rs:1375` | Yes — `src/sync.rs:1011` |
| 4 | `Health` | `HealthRequest` | `HealthResponse` | `snip-sync/src/lib.rs:982` | Yes — `src/sync.rs:1024` |
| 5 | `Register` | `RegisterRequest` | `RegisterResponse` | `snip-sync/src/lib.rs:999` | Yes — `src/sync.rs:1059` |
| 6 | `CreateLibrary` | `CreateLibraryRequest` | `CreateLibraryResponse` | `snip-sync/src/lib.rs:1641` | Yes — `src/sync.rs:1152` |
| 7 | `ListLibraries` | `ListLibrariesRequest` | `ListLibrariesResponse` | `snip-sync/src/lib.rs:1698` | Yes — `src/sync.rs:1106` |
| 8 | `DeleteLibrary` | `DeleteLibraryRequest` | `DeleteLibraryResponse` | `snip-sync/src/lib.rs:1759` | **No** |
| 9 | `ListPremadeLibraries` | `ListPremadeLibrariesRequest` | `ListPremadeLibrariesResponse` | `snip-sync/src/lib.rs:1819` | Yes — `src/sync.rs:1187` |
| 10 | `GetPremadeLibrary` | `GetPremadeLibraryRequest` | `GetPremadeLibraryResponse` | `snip-sync/src/lib.rs:1871` | Yes — `src/sync.rs:1220` |
| 11 | `SearchPremadeLibraries` | `SearchPremadeLibrariesRequest` | `SearchPremadeLibrariesResponse` | `snip-sync/src/lib.rs:1943` | Yes — `src/sync.rs:1264` |

The server implements all 11. The client calls **9** — it never calls
`GetSnippets` (it always uses the bidirectional `Sync`) and never calls
`DeleteLibrary` (the client's `delete_library` at `src/library/manager.rs:442`
is a *local* library delete, unrelated to the RPC).

### Generated Rust surface

| Symbol | Location |
|--------|----------|
| `SnippetSyncClient<T>` | `snip-proto/src/snip_proto.rs:278` (module `snippet_sync_client` at `:267`) |
| `SnippetSync` server trait | `snip-proto/src/snip_proto.rs:640` (module `snippet_sync_server` at `:629`) |
| `SnippetSyncServer<T>` | `snip-proto/src/snip_proto.rs:721` |
| `SERVICE_NAME` | `snip-proto/src/snip_proto.rs:1330` — `"snip_proto.SnippetSync"` |

Every RPC becomes an `async fn` on the trait and a method on the client, with
the RPC name converted to `snake_case` (`GetSnippets` → `get_snippets`).

## The `Snippet` Message

```protobuf
// snip-proto/proto/sync.proto:99-118
message Snippet {
    string id = 1;
    string description = 2;
    string command = 3;
    repeated string tags = 4;
    int64 created_at = 5;
    int64 updated_at = 6;
    string device_id = 7;
    bool deleted = 8;
    bool encrypted = 9;
}
```

**9 fields.** The wire struct is `snip_proto::Snippet`; client code aliases
it to `ProtoSnippet` to distinguish it from the local model
`crate::library::Snippet`.

| # | Field | Type | Semantics |
|---|-------|------|-----------|
| 1 | `id` | `string` | Client-generated stable identifier (required, non-empty) |
| 2 | `description` | `string` | Plaintext when `encrypted` is false; **empty** for encrypted records |
| 3 | `command` | `string` | Plaintext command when `encrypted` is false; the **encrypted payload** otherwise |
| 4 | `tags` | `repeated string` | Plaintext when `encrypted` is false; **empty** for encrypted records |
| 5 | `created_at` | `int64` | Unix timestamp of creation |
| 6 | `updated_at` | `int64` | Unix timestamp used for last-write-wins conflict resolution |
| 7 | `device_id` | `string` | Client/device identifier — the deterministic tie-breaker |
| 8 | `deleted` | `bool` | Deletion tombstone |
| 9 | `encrypted` | `bool` | `command` holds a client-encrypted payload |

### `output` Is Not a Wire Field

**`output` is a local-only field and is not on the wire.** It is not part of
`ProtoSnippet`, and the client's conversion sets it to nothing at all:

```rust
// src/sync_commands.rs:423-437
impl From<&Snippet> for ProtoSnippet {
    fn from(s: &Snippet) -> Self {
        ProtoSnippet {
            id: s.id.clone(),
            description: s.description.clone(),
            command: s.command.clone(),
            tags: s.tags.clone(),
            created_at: s.created_at,
            updated_at: s.updated_at,
            device_id: s.device_id.clone(),
            deleted: s.deleted,
            encrypted: false,
        }
    }
}
```

`output`, `folders`, and `favorite` are local-only and are excluded from the
conflict fingerprint (see [sync.md](sync.md)). This is why
`snp edit --output` requires `--filter` unless an exact selector is given —
an exact selector is the only way to name a snippet that is not otherwise
searchable by its `output` value.

### The `deleted` Tombstone

`deleted = true` marks a **deletion tombstone**: the snippet's content is
gone, but the record must survive so that other devices learn about the
deletion instead of resurrecting the old content.

Tombstone visibility differs by RPC:

| RPC | `include_deleted` | Tombstones returned? | Site |
|-----|-------------------|----------------------|------|
| `GetSnippets` | `false` | **No** | `snip-sync/src/lib.rs:1137` |
| `Sync` | `true` | **Yes** | `snip-sync/src/lib.rs:1557` |

The schema comment states this directly: *"Deletion tombstone. Tombstones are
returned by Sync, not GetSnippets."*
(`snip-proto/proto/sync.proto:114-115`.)

Deletion beats live content even when the tombstone carries an older
timestamp — there is no resurrection. See [sync.md](sync.md).

### The `encrypted` Flag

`encrypted = true` means the `command` field carries a **client-encrypted
payload** rather than a plaintext command. The server stores and relays the
payload opaquely; it has no built-in way to decrypt it (the key is derived
from the API key, which the server also receives as a bearer token — see
[encryption.md](encryption.md) for the trust boundary).

When the client encrypts a snippet it blanks the other plaintext fields:

```rust
// src/sync.rs:1408-1418
Ok(crate::proto::Snippet {
    id: snippet.id.clone(),
    description: String::new(),
    command: encrypted,
    tags: vec![],
    created_at: snippet.created_at,
    updated_at: snippet.updated_at,
    device_id: snippet.device_id.clone(),
    deleted: snippet.deleted,
    encrypted: true,
})
```

The server validates the two shapes separately
(`snip-sync/src/lib.rs:869-977`):

| Condition | Limit applied |
|-----------|---------------|
| `!encrypted` | `command` length ≤ `max_command_length` (default 1024) and must be non-blank |
| `encrypted` | `command` length ≤ `max_encrypted_payload_length` and must be non-blank |
| either | `description` ≤ `max_description_length` (default 1024), `tags.len()` ≤ `max_tags` (default 50), each tag ≤ `max_tag_length` |
| either | `id` non-empty and ≤ `max_id_length` (default 128); `device_id` non-empty and ≤ `max_device_id_length` (default 128) |

An encrypted snippet is therefore stored as **opaque bytes with an empty
description and no tags** — the server can merge it (by `id`,
`updated_at`, `device_id`) but cannot search or display it.

## Library Messages

### Library

```protobuf
// snip-proto/proto/sync.proto:177-182
message Library {
    string id = 1;
    string name = 2;
    int64 created_at = 3;
    int64 snippet_count = 4;
}
```

Library IDs are **scoped to the authenticated account**
(`snip-proto/proto/sync.proto:31`); an unowned library ID yields
`Status::not_found("Library not found")`, not a permission error
(`snip-sync/src/lib.rs:1115`).

### PremadeLibrary

```protobuf
// snip-proto/proto/sync.proto:194-200
message PremadeLibrary {
    string name = 1;
    string filename = 2;
    string description = 3;
    int32 snippet_count = 4;
    repeated string tags = 5;
}
```

Premade libraries are **server-to-client only** — there is no way to push a
premade library back. `GetPremadeLibraryResponse` returns the library as a
**TOML string** in `content` (`snip-proto/proto/sync.proto:207-212`), not as
a `Snippet` list; the client parses it with the same TOML helpers it uses
for local libraries.

## Paging and Incremental Sync

### Paging fields

| Field | Messages | Meaning |
|-------|----------|---------|
| `limit` | `GetSnippetsRequest`, `SyncRequest` | Maximum records to return; `0` selects the server default |
| `offset` | `GetSnippetsRequest`, `SyncRequest` | Records to skip |
| `has_more` | `SnippetList`, `SyncResponse`, `ListLibrariesResponse`, `ListPremadeLibrariesResponse` | More pages remain |
| `total_count` | `SnippetList`, `SyncResponse`, `ListLibrariesResponse`, `ListPremadeLibrariesResponse`, `SearchPremadeLibrariesResponse` | Total matching records |

Server defaults and clamps:

| RPC | `limit = 0` default | Clamp | Site |
|-----|---------------------|-------|------|
| `GetSnippets` | 100 | `clamp(1, MAX_REQUEST_LIMIT)` | `snip-sync/src/lib.rs:1128-1132` |
| `Sync` | 1000 | `clamp(1, MAX_REQUEST_LIMIT)` | `snip-sync/src/lib.rs:1537-1541` |

`MAX_REQUEST_LIMIT = 1000` (`snip-sync/src/lib.rs:117`).

`has_more` is computed as `offset + page_len < total` **OR** the page was
byte-truncated (`snip-sync/src/lib.rs:1168`, `:1536`) — a page that had
rows dropped for size still reports `has_more`, so the client does not stop
early and lose data.

### `since` and `last_sync_timestamp`

Two different fields carry the incremental-sync watermark, and they are
**not** interchangeable:

| Field | Message | Used for |
|-------|---------|----------|
| `since` | `GetSnippetsRequest` (`:42`) | The download filter of the read-only `GetSnippets` RPC |
| `last_sync_timestamp` | `SyncRequest` (`:71`) | The download filter **and** the watermark the client stores after a completed `Sync` |

**The download filter is inclusive (`>=`), not exclusive (`>`).** The server
SQL is:

```sql
-- snip-sync/src/db.rs:634-635
WHERE user_id = ?
  AND library_id = ?
  AND updated_at >= ?
  AND (? OR deleted = 0)
```

and the reason is stated in the source (`snip-sync/src/db.rs:626-631`):
timestamps have **second granularity** and the watermark returned to the
client is `MAX(updated_at)`. With `>`, a row written in the same second as
the previous sync's watermark would be skipped by every future incremental
sync. Re-delivering boundary rows is safe because merges are idempotent by
snippet identity.

> **Partially resolved.** The schema comment at `snip-proto/proto/sync.proto`
> said *"Results are filtered to records with updated_at strictly greater than
> since"* — that was **wrong**. The behaviour is `>=`. The `.proto` comments
> have now been corrected to state `>=` (and to explain the second-granularity
> reason), and the `SyncRequest.local_snippets` comment was corrected to say
> that every local record is merged rather than only those newer than
> `last_sync_timestamp`. The copy in the **generated** stub at
> `snip-proto/src/snip_proto.rs:357-358` (and `:642`) still carries the old
> wording — it is regenerated only on an explicit regen, and
> `protoc-gen-tonic` is not part of the normal toolchain. See
> [Known Drift](#known-drift-between-the-proto-and-the-generated-stubs).

**`last_sync_timestamp` does not gate the upload.** It selects the page of
*returned* records; every record in `SyncRequest.local_snippets` is merged
regardless of the watermark, using that record's own `updated_at` for
conflict resolution (`snip-sync/src/lib.rs:1457-1509`,
`snip-sync/src/db.rs:679`, `:647`).

> The `.proto` schema comment is **correct**: *"Every local record is considered
> for insertion or conflict resolution; `last_sync_timestamp` only selects the
> page of server rows returned in the response."*
> (`snip-proto/proto/sync.proto:74-76`). The older wording — *"Only records
> newer than last_sync_timestamp are considered for insertion or conflict
> resolution"* — survives only in the generated stub at
> `snip-proto/src/snip_proto.rs:48-49`. See
> [Known Drift](#known-drift-between-the-proto-and-the-generated-stubs).

### The watermark does not advance while paginating

When `has_more` is true the server returns the **incoming**
`last_sync_timestamp` as `server_timestamp`, not a fresh watermark:

```rust
// snip-sync/src/lib.rs:1596-1609 (shape)
let timestamp = if has_more {
    // Don't advance timestamp when paginating — client needs to fetch remaining pages
    req.last_sync_timestamp
} else {
    self.db.get_latest_timestamp(&user_id, &library_id).await ...
};
```

`get_latest_timestamp` is `MAX(updated_at)` over the library
(`snip-sync/src/db.rs:773-783`). Advancing the watermark while records
remain would silently skip them on the next incremental sync.

## Authentication

Every request message except `RegisterRequest` carries an `api_key` field. On
the three sync-path messages it is explicitly deprecated in the schema:

> *"Deprecated compatibility field. New clients should send the API key in
> the authorization metadata as `Bearer <api-key>`."*
> (`snip-proto/proto/sync.proto:43-44` — `GetSnippetsRequest`;
> `snip-proto/proto/sync.proto:57` and `:72` carry the shorter *"Deprecated
> compatibility field. Prefer authorization metadata."* wording)

The remaining request messages (`CreateLibraryRequest`, `ListLibrariesRequest`,
`DeleteLibraryRequest`, and the three premade-library requests) keep a bare
`api_key` field with no schema comment.

The server accepts the key from the authorization metadata first and falls
back to the message field on every authenticated RPC — e.g.
`snip-sync/src/lib.rs:1079` for `GetSnippets`, `:1169` for `PushSnippets`,
`:1353` for `Sync`.

`RegisterResponse.api_key` (`snip-proto/proto/sync.proto:139`) is the **one
exception** — that is where the key is *issued*.

## Message and Size Limits

| Limit | Value | Where |
|-------|-------|-------|
| gRPC max message size (server) | 4 MiB | `snip-sync/src/lib.rs:113` — `DEFAULT_GRPC_MAX_MESSAGE_SIZE` |
| Client request ceiling | 3.5 MiB (`3_584 * 1024`) | `src/sync.rs:46` — `DEFAULT_CLIENT_REQUEST_CEILING` |
| Max records per page | 1000 | `snip-sync/src/lib.rs:117` — `MAX_REQUEST_LIMIT` |
| Max command length (plaintext) | 1024 | `snip-sync/src/lib.rs:95` |
| Max description length | 1024 | `snip-sync/src/lib.rs:103` |
| Max tags | 50 | `snip-sync/src/lib.rs:104` |
| Max tag length | 100 | `snip-sync/src/lib.rs:105` |
| Max snippets per request | 10 000 | `snip-sync/src/lib.rs:109` — `DEFAULT_MAX_SYNC_SNIPPETS` (checked at `:1218` and `:1363`) |
| Max snippet ID length | 128 | `snip-sync/src/lib.rs:106` |
| Max device ID length | 128 | `snip-sync/src/lib.rs:107` |
| Max API key length | 512 | `snip-sync/src/lib.rs:108` |

The client ceiling is deliberately **below** the server's 4 MiB limit. The
client measures its request with Prost's `encoded_len()` — not a character
count — so the encoded size is what it bounds
(`src/sync.rs:1496-1531`).

## Code Generation

**`protoc` is needed only for an explicit, manual regen.** The generated file
is checked in and the crate has no `build.rs`:

```
snip-proto/
├── Cargo.toml
├── README.md
├── proto/
│   └── sync.proto
└── src/
    ├── lib.rs
    └── snip_proto.rs      ← checked in
```

The workflow:

1. Edit `snip-proto/proto/sync.proto`.
2. Run `protoc` manually with the tonic/prost plugins to regenerate
   `snip-proto/src/snip_proto.rs`.
3. Commit **both** files.

**`snip-proto/src/snip_proto.rs` must never be hand-edited.** It is
generated output; an edit there is lost on the next regen and will not match
the schema.

### Known drift between the proto and the generated stubs

The checked-in stub is **newer than the schema** for two comments. The
generated file was regenerated in commit `9d35dc1` ("Fix reported repository
bugs"), but the matching `.proto` comments were not updated at the time. The
`.proto` comments have since been corrected to match the implementation; the
stub retains the old wording until the next explicit regen.

| Topic | `snip-proto/proto/sync.proto` | `snip-proto/src/snip_proto.rs` | Reality |
|-------|--------------------|---------------------|---------|
| `since` filter | *"…updated_at greater than or equal to since (>=)"* (corrected) | *"strictly greater than since"* (`:357-358`, `:642`) | **`>=`** — `snip-sync/src/db.rs:634-635` |
| `last_sync_timestamp` | *"Every local record is considered for insertion or conflict resolution; `last_sync_timestamp` only selects the page of server rows returned."* (corrected) | *"The server uses each record's own timestamp for conflict resolution; `last_sync_timestamp` only selects returned data."* (`:48-49`) | Both correct — the stub was right first |

Doc comments do not affect the wire format, so the drift is harmless to
compatibility — but it is a trap for anyone reading the schema as the
specification. The `.proto` file is the schema and the **implementation** is
the behavioural truth. The `.proto` comments have been corrected; the
generated stub still carries the old wording and will pick it up on the next
explicit regen.

## Publishing

The publish order is fixed, because each crate depends on the previous one:

```
snip-proto  →  snip-sync  →  snip-it
```

| Crate | Version | Path dependency |
|-------|---------|-----------------|
| `snip-proto` | 0.1.3 | — |
| `snip-sync` | 0.1.6 | `snip-proto = { version = "0.1.3", path = "../snip-proto" }` (`snip-sync/Cargo.toml:58`) |
| `snip-it` | 1.3.9 | `snip-proto = { version = "0.1.3", path = "snip-proto" }` (`Cargo.toml:126`) |

**A proto change requires bumping `snip-proto`'s version in both
dependents.** `snip-sync/Cargo.toml:58` and `Cargo.toml:126` both pin
`version = "0.1.3"`; after a proto change both must be updated to the new
version, or `crates.io` will resolve the old published `snip-proto`.

Publishing is manual and local — there is no CI token or workflow that
publishes. See `RELEASING.md` and `CONTRIBUTING.md`.

## Key Files

| File | Role |
|------|------|
| `snip-proto/proto/sync.proto` | The schema — service, 11 RPCs, all messages |
| `snip-proto/src/snip_proto.rs` | The checked-in generated stubs — **never hand-edit** |
| `snip-proto/src/lib.rs` | `include!` + flat re-export |
| `snip-proto/Cargo.toml` | Crate manifest; **no** `build.rs`, no `protoc` in the build |
| `src/lib.rs:43` | `pub(crate) use snip_proto as proto;` — the client's alias |
| `src/sync.rs` | The client: 9 RPCs, the retry macro, the byte ceiling |
| `src/sync_commands.rs:423-437` | `From<&Snippet> for ProtoSnippet` — what reaches the wire |
| `snip-sync/src/lib.rs` | The server: all 11 RPCs, validation, paging |
| `snip-sync/src/db.rs:165-210` | The `>=` download filter |

## Invariants

- **`snip_proto.rs` is generated output.** Never edit it by hand; regenerate
  with `protoc`.
- **A proto change bumps `snip-proto`'s version in both dependents** before
  publishing.
- **The publish order is `snip-proto` → `snip-sync` → `snip-it`.**
- **`output` is not on the wire.** It is local-only, and `folders` and
  `favorite` are too.
- **`deleted` tombstones are returned by `Sync`, never by `GetSnippets`.**
- **The download filter is `updated_at >= since`.** The watermark is
  inclusive by design (second-granularity timestamps).
- **`last_sync_timestamp` does not gate uploads.** It selects the returned
  page; merges use each record's own `updated_at`.
- **The watermark does not advance while `has_more` is true.**
- **The client's 3.5 MiB ceiling is below the server's 4 MiB limit**, and is
  measured with Prost's `encoded_len()`.
- **Field numbers are permanent.** Never renumber or reuse a field number —
  it is a wire-format break.

## Gotchas

- **`GetSnippets` and `DeleteLibrary` are implemented but unused by the
  client.** A change to their semantics affects the server and any other
  client, not `snp-it`.
- **`GetSnippetsRequest.since` and `SyncRequest.last_sync_timestamp` are
  different fields** with different meanings. Do not substitute one for the
  other.
- **The `since` filter is `>=`.** A client that assumes `>` will
  double-deliver boundary rows (harmless — merges are idempotent) and one
  that assumes `>=` on a `>` server would skip rows.
- **The schema's comments are not authoritative** for the two items listed
  under [Known drift](#known-drift-between-the-proto-and-the-generated-stubs).
  The implementation is.
- **`api_key` in a request message is deprecated on the three sync-path
  messages** (`GetSnippetsRequest`, `PushSnippetsRequest`, `SyncRequest`). New
  clients must use the `Bearer <api-key>` authorization metadata; the server
  accepts the message field only as a fallback. See
  [Authentication](#authentication).
- **`has_more` can be true even when the row count fits the page**, because
  the page was byte-truncated. Never stop paginating on the row count alone.
- **Encrypted snippets carry no `description` and no `tags`.** A consumer
  that assumes those fields are populated will show an empty row.
- **`total_count` counts *matching* rows, including tombstones** for `Sync`
  and excluding them for `GetSnippets`.
- **The `encrypted` flag is not set by the generic conversion.**
  `From<&Snippet> for ProtoSnippet` always sets `encrypted: false`
  (`src/sync_commands.rs:434`); the encrypted form is built explicitly in
  `src/sync.rs:1408-1418`.
