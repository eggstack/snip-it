# Proto Module Review Plan

**Module**: `snip-proto/`
**Architecture Doc**: `architecture/proto.md`
**Date**: 2026-05-29

---

## 1. Document Accuracy

### Verified Correct

| Claim in `proto.md` | Verified In | Status |
|---------------------|-------------|--------|
| Directory is `snip-proto/` | `snip-proto/` exists | ✅ |
| Proto file at `snip-proto/proto/sync.proto` (172 lines) | `snip-proto/proto/sync.proto` — exactly 172 lines | ✅ |
| 10 RPCs in `SnippetSync` service | `sync.proto:7-31` — 10 RPCs defined | ✅ |
| Service: `GetSnippets`, `PushSnippets`, `Sync`, `Health`, `Register`, `CreateLibrary`, `ListLibraries`, `DeleteLibrary`, `ListPremadeLibraries`, `GetPremadeLibrary` | `sync.proto:9-30` | ✅ |
| `Snippet` message fields: id, description, command, tags[], created_at, updated_at, device_id, deleted, encrypted | `sync.proto:71-81` | ✅ |
| `SyncRequest` fields: api_key, local_snippets[], last_sync_timestamp, library_id, limit | `sync.proto:54-60` | ✅ |
| `SyncResponse` fields: success, message, snippets[], server_timestamp, skipped_count, skipped_ids[] | `sync.proto:62-69` | ✅ |
| `Library` message: id, name, created_at, snippet_count | `sync.proto:140-145` | ✅ |
| `PremadeLibrary` message: name, filename, description, snippet_count | `sync.proto:155-160` | ✅ |
| Re-export pattern: `pub mod sync { include!("snip_proto.rs"); } pub use sync::*;` | `snip-proto/src/lib.rs:1-5` | ✅ |
| Generated code in `src/snip_proto.rs` | `snip-proto/src/snip_proto.rs` exists (1196 lines) | ✅ |
| Consumers import via `use snip_proto::{...}` and `use snip_proto::snippet_sync_client::SnippetSyncClient` | `src/sync.rs:17-22`, `snip-sync/src/main.rs:13-21` | ✅ |
| Dependencies: `tonic`, `prost`, `tonic-build` | `snip-proto/Cargo.toml` — see discrepancy below | ⚠️ Partial |

### Discrepancies

1. **Wrong build crate name**: Architecture doc states `tonic-build` (line 71) but actual dependency is `tonic-prost-build` (`snip-proto/Cargo.toml:13`). The build script also uses `tonic_prost_build::configure()` (`snip-proto/build.rs:2`). The doc should say `tonic-prost-build`.

2. **`build_server(true)` not documented**: `build.rs:3` sets `.build_server(true)`, which generates both client and server code. The architecture doc does not mention this setting, yet the generated `snip_proto.rs` includes both `snippet_sync_client` and `snippet_sync_server` modules.

3. **Checked-in generated code**: Architecture doc (line 78) says `snip_proto.rs` is "checked in". This is verified — the file exists in the repo and is not `.gitignore`d. However, this is a **design concern** (see §3) because generated code can drift from the `.proto` definition if someone edits the proto but forgets to regenerate.

4. **Line count mismatch**: The doc does not specify a line count for `snip_proto.rs`, but the generated file is 1196 lines (vs. 172 for the `.proto`). Not a bug, but worth noting that the generated code is ~7x the source.

---

## 2. Bugs & Issues

### B1. No `include!("snip_proto.rs")` rebuild trigger

**File**: `snip-proto/build.rs:1-7`

`tonic-prost-build` generates `snip_proto.rs` at build time into `src/`, but `build.rs` does not emit `cargo:rerun-if-changed=proto/sync.proto`. Without this directive, Cargo's incremental build may not re-run the build script when `sync.proto` is modified. This means edits to the proto file may not trigger regeneration of `snip_proto.rs` in some build scenarios (e.g., `cargo check` with cached artifacts).

**Severity**: Medium
**Fix**: Add `println!("cargo:rerun-if-changed=proto/sync.proto");` at the start of `build.rs`.

### B2. `Snippet` message missing `string output` field

**File**: `snip-proto/proto/sync.proto:71-81`

The `Snippet` message in the proto definition does not include an `output` field, yet `Snippet` structs in the local codebase (`src/library.rs`) have an `output` field that is preserved during sync merge (`src/sync_commands.rs`). The `output` field is stripped when converting to proto (`src/sync_commands.rs:6-19`) and not transmitted over the wire. This is **by design** (local-only field), but it's not documented in the architecture doc. If someone adds an `output` field to the proto in the future, it could cause a collision.

**Severity**: Low (design, not a bug)

### B3. `Library.snippet_count` type mismatch between proto and generated code

**File**: `snip-proto/proto/sync.proto:144` vs `snip-proto/src/snip_proto.rs:175`

In the `.proto` file, `Library.snippet_count` is declared as `int64` (tag 4). The generated Rust code correctly maps this to `i64` at `snip_proto.rs:175`. However, the architecture doc's message table (line 39) just says `snippet_count` without noting the type. More importantly, **the client code in `src/sync.rs:253` constructs a `Library` with `snippet_count: 0` (an `i64`), which is correct**. But in `snip-sync/src/main.rs`, the server must convert from the database's `i64` (or `i32` from SQL) to the proto's `i64`. Verify the server handles this cast correctly.

**Severity**: Low (verified correct in generated code, but type is wide for a count)

### B4. `PushSnippetsRequest` field number gap

**File**: `snip-proto/proto/sync.proto:41-45`

```protobuf
message PushSnippetsRequest {
    string api_key = 1;
    string library_id = 3;      // field 3
    repeated Snippet snippets = 2;  // field 2
}
```

Field 2 (`snippets`) and field 3 (`library_id`) are defined out of numerical order. While protobuf allows this (fields are identified by number, not position), it's unconventional and could confuse maintainers. More importantly, the numbering gap is not actually a gap — it's just out-of-order. This is valid but poor practice.

**Severity**: Low (cosmetic/style)

### B5. `Snippet` has no `output` field but server may need to store it

**File**: `snip-proto/proto/sync.proto:71-81`

The `Snippet` proto message does not include fields for `output`, `folders`, or `favorite` — these are local-only fields. This is an intentional design choice documented in `AGENTS.md` under "Sync Merge Strategy". However, the architecture doc `proto.md` does not call this out. A developer reading only `proto.md` would not understand why certain fields are missing.

**Severity**: Low (documentation gap)

### B6. Generated `snip_proto.rs` is stale-risky

**File**: `snip-proto/src/snip_proto.rs`

The generated file is checked into version control. If a developer modifies `sync.proto` but forgets to run `cargo build` (or `cargo run --manifest-path snip-proto/build.rs`), the checked-in generated code will be out of sync. There is no CI check or build step that verifies the generated code matches the proto definition.

**Severity**: Medium

---

## 3. Design Issues

### D1. Checking in generated code (`snip_proto.rs`)

**File**: `snip-proto/src/snip_proto.rs`

The 1196-line generated file is checked into git. This is a common trade-off:
- **Pro**: Avoids requiring `protoc` and `tonic-build` toolchain for consumers who only need the client types (e.g., if `snip-proto` were published to crates.io).
- **Con**: Generated code can drift from the source `.proto` file. The build script still runs and overwrites the file, so the check-in is redundant in a normal dev workflow.

**Recommendation**: Either:
- Remove the check-in and add `.gitignore` for `snip_proto.rs`, OR
- Add a CI step that regenerates and checks for diffs, OR
- Keep as-is but document the expectation that `snip_proto.rs` is always regenerated at build time.

### D2. Flat re-export namespace

**File**: `snip-proto/src/lib.rs:5`

`pub use sync::*;` re-exports all 20+ message types plus client/server modules at the crate root. This means `use snip_proto::Snippet` works alongside `use snip_proto::snippet_sync_client::SnippetSyncClient`. While convenient, it creates a flat namespace where it's unclear whether a type is a message, a client, or a server.

**Recommendation**: Consider keeping the wildcard re-export for messages only, and requiring explicit paths for client/server modules. However, this is an existing pattern used consistently across the codebase, so changing it would be a breaking change for consumers.

### D3. No `DeleteLibrary` RPC on the client side

**File**: `snip-proto/src/snip_proto.rs` (generated), `src/sync.rs`

The generated client includes `delete_library()`, but `src/sync.rs` (`SyncClient`) does not expose a `delete_library()` method. The `DeleteLibrary` RPC is defined in the proto and generated, but the client wrapper does not use it. This may be intentional (library deletion may not be supported in the CLI yet) or dead code.

**Severity**: Low

### D4. `go_package` option is irrelevant

**File**: `snip-proto/proto/sync.proto:5`

```protobuf
option go_package = "github.com/snip-it/snip-proto;snip_proto";
```

This option is for Go code generation, which this project does not use. It's harmless but unnecessary clutter in a Rust-only project.

**Severity**: Low (cosmetic)

### D5. No proto versioning or backward compatibility annotations

**File**: `snip-proto/proto/sync.proto`

The proto file has no `reserved` fields, no `deprecated` annotations, and no version field in any message. If the proto evolves (e.g., adding fields to `Snippet`), there's no mechanism to prevent field number collisions or signal deprecation to older clients.

**Severity**: Medium

---

## 4. Security Concerns

### S1. API key transmitted in plaintext over gRPC

**File**: `snip-proto/proto/sync.proto:34,42,54,97,108,118,131,148,163`

The `api_key` field appears in 9 request messages as a plain `string`. While the client (`src/sync.rs:304-306`) configures TLS (`ClientTlsConfig`), the API key itself is not hashed or tokenized at the transport layer. An attacker with access to TLS termination (e.g., a reverse proxy) could intercept API keys.

**Recommendation**: Document that API keys should be treated as secrets and that TLS termination must be handled securely. Consider using bearer tokens or short-lived session tokens instead of long-lived API keys in proto messages.

### S2. No authentication metadata on `Health` RPC

**File**: `snip-proto/proto/sync.proto:89`

`HealthRequest` is an empty message with no `api_key` field. This is standard for health checks, but it means the health endpoint is unauthenticated. An attacker could use the health check to enumerate running server instances.

**Severity**: Low (standard practice)

### S3. `RegisterRequest` accepts arbitrary `device_id`

**File**: `snip-proto/proto/sync.proto:96-98`

The client sends `device_id: String::new()` (empty string) during registration (`src/sync.rs:205`). The server generates a device ID and returns it. This is fine, but if a client sends a crafted `device_id`, the server should validate/sanitize it to prevent injection or impersonation.

**Severity**: Low (verify server-side validation)

---

## 5. Performance Issues

### P1. `SyncRequest.local_snippets` has no size limit in proto

**File**: `snip-proto/proto/sync.proto:54-60`

The `local_snippets` repeated field has no upper bound in the proto definition. The client hardcodes `limit: 1000` (`src/sync.rs:114`), but a malicious or buggy client could send thousands of snippets in a single request, potentially causing memory pressure on the server.

**Recommendation**: Add a comment documenting the expected limit, or use a proto validation library (e.g., `prost-validate`) to enforce limits.

### P2. No message compression enabled

**File**: `snip-proto/build.rs:1-7`

The build script does not enable compression encoding. The generated server and client support compression via `accept_compressed()`/`send_compressed()` methods, but neither is used. For sync operations with many snippets, compression could significantly reduce payload size.

**Severity**: Low (optional optimization)

---

## 6. Test Coverage

### T1. No unit tests for proto conversion functions

**File**: `src/sync.rs:318-372`

The `encrypt_snippet()` and `decrypt_snippet()` functions have no unit tests. The existing test at `src/sync.rs:378-383` only verifies constants. Round-trip encryption/decryption of `ProtoSnippet` structs is untested.

**Severity**: Medium

### T2. No integration tests exercising proto serialization

**File**: `tests/integration.rs` (not read in detail, but searched)

The integration tests focus on CLI commands, not proto serialization. There are no tests that construct proto messages, serialize them, and verify the wire format matches expectations.

**Severity**: Low (covered by tonic/prost's own tests)

### T3. Server-side proto implementation not tested here

**File**: `snip-sync/src/main.rs`

The `SnippetSync` trait implementation lives in the server binary, not in a testable library. Unit testing the server requires spinning up the full gRPC server. Consider extracting the service implementation into a separate module for testability.

**Severity**: Medium

---

## 7. Priority Ranking

| ID | Description | Severity | Category |
|----|-------------|----------|----------|
| B1 | Missing `cargo:rerun-if-changed` in build.rs | Medium | Bug |
| B6 | Generated code checked in without drift protection | Medium | Bug |
| D1 | Checking in generated `snip_proto.rs` | Medium | Design |
| D5 | No proto versioning/backward compat annotations | Medium | Design |
| T1 | No unit tests for proto conversion functions | Medium | Test |
| T3 | Server service impl not unit-testable | Medium | Test |
| S1 | API key in plaintext in proto messages | Low | Security |
| P1 | No size limit on `local_snippets` in proto | Low | Performance |
| B4 | `PushSnippetsRequest` fields out of order | Low | Bug |
| B5 | Missing local-only fields not documented | Low | Doc |
| D2 | Flat re-export namespace | Low | Design |
| D3 | `DeleteLibrary` client method unused | Low | Design |
| D4 | Irrelevant `go_package` option | Low | Design |
| P2 | No message compression | Low | Performance |
| S2 | Unauthenticated health endpoint | Low | Security |
| S3 | Arbitrary device_id in RegisterRequest | Low | Security |
| T2 | No proto serialization integration tests | Low | Test |
| Doc1 | `tonic-build` should be `tonic-prost-build` | Low | Doc |
| Doc2 | `build_server(true)` not documented | Low | Doc |

---

## 8. Recommendations

1. **Immediate**: Add `println!("cargo:rerun-if-changed=proto/sync.proto");` to `build.rs` to ensure proto changes trigger regeneration (B1).

2. **Immediate**: Fix `architecture/proto.md` to say `tonic-prost-build` instead of `tonic-build` (Doc1).

3. **Short-term**: Add a CI step that regenerates `snip_proto.rs` and checks `git diff` to catch drift (B6, D1).

4. **Short-term**: Document the `build_server(true)` setting and its effect on generated code (Doc2).

5. **Short-term**: Add unit tests for `encrypt_snippet()`/`decrypt_snippet()` round-trip (T1).

6. **Medium-term**: Add proto `reserved` field numbers and consider `prost-validate` annotations for size limits (D5, P1).

7. **Long-term**: Consider extracting the `SnippetSync` service implementation from `snip-sync/src/main.rs` into a testable library crate (T3).

8. **Long-term**: Evaluate whether compression should be enabled for sync payloads (P2).
