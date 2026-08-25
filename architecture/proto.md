# Protobuf API

[← Back to Overview](overview.md)

## Overview

**Directory**: `snip-proto/`

Defines the gRPC service and message types used for client-server communication. The generated Rust module is checked in so normal builds do not require `protoc`.

## Proto Definition

**File**: `snip-proto/proto/sync.proto` (216 lines)

### Service

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

### Message Types

| Message | Key Fields |
|---------|-----------|
| `Snippet` | id, description, command, tags[], created_at, updated_at, device_id, deleted, encrypted |
| `SyncRequest` | api_key, local_snippets[], last_sync_timestamp, library_id, limit, offset |
| `SyncResponse` | success, message, snippets[], server_timestamp, skipped_count, skipped_ids[], has_more, total_count |
| `Library` | id, name, created_at, snippet_count |
| `PremadeLibrary` | name, filename, description, snippet_count, tags[] |

## Code Generation

The generated Rust module is committed at `snip-proto/src/snip_proto.rs`.
Maintainers regenerate it explicitly after changing `sync.proto` with the
repository's protobuf generation workflow, then review and commit the result.
Normal workspace builds, packaging, and CI consume the checked-in source and
do not invoke `protoc`.

### Re-export Pattern

```rust
// snip-proto/src/lib.rs
pub mod sync {
    include!("snip_proto.rs");
}
pub use sync::*;
```

This allows consumers to import directly:
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

- `snip-proto/proto/sync.proto` — Service and message definitions
- `snip-proto/src/snip_proto.rs` — checked-in generated code
- `snip-proto/src/lib.rs` — Module re-exports
- `snip-proto/src/snip_proto.rs` — Generated code (checked in)
- `snip-proto/Cargo.toml` — Dependencies
