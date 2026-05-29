# Proto Module Improvement Plan

## Architecture Document Claims vs Implementation

### 1. Service Definition (proto.md lines 17-29)
- **Claim**: 9 RPCs defined in `SnippetSync` service
- **Actual**: 9 RPCs defined in `proto/sync.proto` lines 7-31
- **Status**: VERIFIED

### 2. Message Types Table (proto.md lines 34-40)
- **Claims**:
  - `Snippet`: id, description, command, tags[], created_at, updated_at, device_id, deleted, encrypted
  - `SyncRequest`: api_key, local_snippets[], last_sync_timestamp, library_id, limit
  - `SyncResponse`: success, message, snippets[], server_timestamp, skipped_count, skipped_ids[]
  - `Library`: id, name, created_at, snippet_count
  - `PremadeLibrary`: name, filename, description, snippet_count
- **Discrepancies Found**:
  - `SyncRequest` also has `offset` field (proto line 39) - NOT DOCUMENTED
  - `Library.snippet_count` is `i64` in proto (line 175) but documented without type
  - Missing from table: `PushSnippetsRequest`, `PushSnippetsResponse`, `SnippetList`, `HealthRequest/Response`, `RegisterRequest/Response`, `CreateLibraryRequest/Response`, `ListLibrariesRequest/Response`, `DeleteLibraryRequest/Response`, `ListPremadeLibrariesRequest/Response`, `GetPremadeLibraryRequest/Response`
- **Status**: PARTIALLY VERIFIED - table is incomplete

### 3. Code Generation (proto.md lines 42-48)
- **Claim**: Uses `tonic-build` to generate `src/snip_proto.rs`
- **Actual**:
  - Uses `tonic-prost-build` (not `tonic-build`) - see `snip-proto/Cargo.toml:13`
  - `build.rs:3` uses `tonic_prost_build::configure()`
  - `build_server(true)` correctly configured
- **Status**: VERIFIED with minor naming discrepancy (tonic-prost-build vs tonic-build)

### 4. Re-export Pattern (proto.md lines 50-63)
- **Claim**: `src/lib.rs` uses `pub mod sync { include!("snip_proto.rs") }` and `pub use sync::*`
- **Actual**: Exact match at `snip-proto/src/lib.rs:1-5`
- **Status**: VERIFIED

### 5. Dependencies (proto.md lines 65-71)
- **Claim**: tonic, prost, tonic-build
- **Actual**: tonic, prost, tonic-prost, tonic-prost-build (build), tonic-prost (build)
- **Status**: PARTIALLY VERIFIED - similar but not identical

---

## Bugs / Edge Cases

### BUG 1: Premade Library Filename Sanitization Too Restrictive
**Location**: `snip-sync/src/main.rs:798-806`
```rust
let sanitized: String = req
    .filename
    .chars()
    .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
    .collect();
```
**Issue**: Filters out dots (`.`), which are valid in filenames. A premade library named `devops.tools` would become `devopstools`.
**Impact**: Legitimate premade libraries may be unretrievable.
**Recommendation**: Allow dots in filenames, add path traversal protection instead.

### BUG 2: Rate Limiting Bypass on Register Endpoint
**Location**: `snip-sync/src/main.rs:360-387`
**Issue**: `register` uses `x-forwarded-for` header for rate limiting without validation. Clients can spoof this header to bypass rate limits.
**Impact**: Register endpoint rate limiting is ineffective against malicious clients.
**Recommendation**: Only use trusted proxy headers, or rate limit based on peer socket address.

### BUG 3: No Server-Side Limit on SyncRequest.local_snippets Array
**Location**: `proto/sync.proto:54-60` and `snip-sync/src/main.rs:580-609`
**Issue**: `SyncRequest.local_snippets` is `repeated Snippet` with no maximum count. Validation happens per-snippet but the array itself has no bound.
**Impact**: Clients could send extremely large arrays causing memory pressure.
**Recommendation**: Add `int32 local_snippets_limit` field or enforce maximum array size in protobuf config.

### BUG 4: Premade Library Endpoint Returns No Pagination Metadata
**Location**: `snip-sync/src/main.rs:755-782`
**Issue**: `ListPremadeLibraries` returns all libraries with no pagination. `GetPremadeLibraryResponse` has no pagination fields.
**Impact**: If many premade libraries exist, clients cannot paginate.
**Recommendation**: Add pagination support to premade library endpoints.

### BUG 5: Library Name Not Returned in ListLibrariesResponse
**Location**: `snip-sync/src/main.rs:711-719`
**Issue**: `list_libraries` returns `Library { id, name, created_at, snippet_count }` but the proto definition (`proto/sync.proto:140-145`) only defines `id`, `name`, `created_at`, `snippet_count`. The proto is correct but the doc table omitted fields.
**Status**: Implementation is correct.

---

## Potential Improvements

### IMPROVEMENT 1: Add TLS Transport Encryption
**Current**: Server warns "TLS is not enabled" (`snip-sync/src/main.rs:829-831`)
**Recommendation**: Add TLS support directly to the server, or document TLS requirement more prominently.

### IMPROVEMENT 2: Add Request ID / Correlation ID
**Current**: No request tracing ID across operations.
**Recommendation**: Add `request_id` field to all request/response messages for traceability.

### IMPROVEMENT 3: Batch API Key Verification
**Location**: `snip-sync/src/db.rs:216-234`
**Issue**: `get_user_by_api_key` fetches all users with matching prefix, then iterates verifying each.
**Impact**: With many users sharing prefix, linear scan O(n) per auth.
**Recommendation**: Consider indexed lookup improvement or API key prefix collision handling.

### IMPROVEMENT 4: Missing Index for Deleted Snippets Query
**Location**: `snip-sync/src/db.rs:372-437`
**Issue**: `get_snippets` filters by `user_id`, `library_id`, `updated_at`, and `deleted = 0`. No compound index on `(user_id, library_id, deleted, updated_at)`.
**Recommendation**: Add index:
```sql
CREATE INDEX idx_snippets_user_library_deleted ON snippets(user_id, library_id, deleted, updated_at);
```

### IMPROVEMENT 5: Premade Library Content Not Validated
**Location**: `snip-sync/src/main.rs:784-821`
**Issue**: Premade library content is returned as raw string with no size limit or content validation.
**Recommendation**: Add maximum content size limit and validate TOML structure.

### IMPROVEMENT 6: Health Check Returns Hardcoded `healthy: true`
**Location**: `snip-sync/src/main.rs:343-352`
**Issue**: Health check always returns `healthy: true` without checking database connectivity or other dependencies.
**Recommendation**: Actually verify database connection, or document that this is a liveness probe only.

### IMPROVEMENT 7: Sync Skipped Snippets Not Persisted
**Location**: `snip-sync/src/main.rs:578-608`
**Issue**: If validation fails for a snippet during sync, it's added to `skipped_ids` but the client never receives feedback that these were not persisted server-side.
**Impact**: Client may assume skipped snippets will not be re-sent, but they might be.
**Recommendation**: Document sync behavior, or add retry mechanism for skipped snippets.

---

## Security Considerations

### SEC-1: API Key Transmitted in Plaintext
**Issue**: API keys sent in request bodies over unencrypted connections.
**Mitigation**: Server warns about TLS; production should use reverse proxy with TLS.
**Recommendation**: Document this as a known limitation and requirement for production deployment.

### SEC-2: Register Endpoint Device ID Not Validated
**Location**: `snip-sync/src/main.rs:389`
**Issue**: `RegisterRequest.device_id` is ignored entirely (line 389: `let _req = request.into_inner();`).
**Impact**: No device binding or tracking.
**Recommendation**: Either validate/record device_id or remove from proto definition.

### SEC-3: CORS Wildcard in Development
**Location**: `snip-sync/src/main.rs:912-914`
**Issue**: `CORS_ALLOW_ALL=true` allows any origin.
**Recommendation**: Log warning when CORS_ALLOW_ALL is used, even in development.

---

## Documentation Discrepancies

| Document Field | Proto Definition | Notes |
|---------------|------------------|-------|
| SyncRequest.offset | Present (line 39) | Not in doc table |
| PushSnippetsRequest | Present | Not in doc table |
| HealthRequest/Response | Present | Not in doc table |
| All Library management messages | Present | Not in doc table |
| All Premade library messages | Present | Not in doc table |

---

## Summary

**Verified claims**: 4/5 major sections verified
**Bugs found**: 5
**Improvements identified**: 7
**Security concerns**: 3
**Documentation gaps**: Multiple message types missing from table

The proto implementation is generally sound and matches the documented service definition. Main issues are:
1. Incomplete documentation (message types table)
2. Premade library filename sanitization bug (too restrictive)
3. Rate limiting bypass in register endpoint
4. Missing pagination on premade endpoints
5. Health check not actually checking health