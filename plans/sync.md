# Sync Architecture Improvement Plan

## Overview

This document reviews the sync architecture claims against implementation and identifies bugs, discrepancies, and potential improvements.

---

## 1. Architecture Claims vs Implementation

### 1.1 SyncClient Struct

**Documented:**
```rust
pub struct SyncClient {
    channel: Channel,
    retry_config: RetryConfig,
}
```

**Actual (`src/sync.rs:69-72`):**
```rust
pub struct SyncClient {
    client: SnippetSyncClient<Channel>,
    settings: SyncSettings,
}
```

**Status:** DISCREPANCY - The documented struct doesn't match reality. The `channel` is wrapped inside the tonic client, and `retry_config` is not stored as a struct (retry params are constants).

---

### 1.2 Key Methods

| Documented | Actual | Status |
|-----------|--------|--------|
| `sync_snippets()` | `sync_encrypted()` | PARTIAL - Renamed, slightly different signature |
| `get_snippets()` | NOT EXPOSED | MISSING from client |
| `push_snippets()` | NOT EXPOSED | MISSING from client |
| `register_device()` | `register()` | OK |
| `list_libraries()` | `list_libraries()` | OK |
| `create_library()` | `create_library()` | OK |
| `delete_library()` | NOT IMPLEMENTED | MISSING |
| `list_premade()` | `list_premade_libraries()` | OK |
| `get_premade()` | `get_premade_library()` | OK |

**Status:** DISCREPANCY - `delete_library()`, `get_snippets()`, and `push_snippets()` are not implemented in `SyncClient` despite being in the proto definition and documented.

---

### 1.3 Retry Logic

**Documented:** Exponential backoff with jitter for transient failures.

**Actual (`src/sync.rs:31-57`):**
```rust
const MAX_RETRIES: u32 = 3;
const INITIAL_DELAY_MS: u64 = 100;
const MAX_DELAY_MS: u64 = 5000;
// ... retry_grpc! macro implements exponential backoff
```

**Status:** VERIFIED - Retry logic correctly documented.

---

### 1.4 SyncDirection Enum

**Documented:** `Push`, `Pull`, `Bidirectional`

**Actual (`src/config.rs:118-123`):** Same three variants.

**Status:** VERIFIED

---

### 1.5 Merge Strategy

**Documented:** Last-write-wins based on `updated_at` timestamp.

**Actual (`src/sync_commands.rs:428-445`):**
```rust
if server_snip.updated_at > local_snip.updated_at {
    // server wins
} else {
    // local wins
}
```

**Status:** BUG - Equal timestamps favor local, not server. The condition `>` should be `>=` if server is supposed to win on equal timestamps. This could cause data loss in race conditions.

---

### 1.6 Encryption

**Documented:** AES-256-GCM + Argon2id key derivation.

**Actual (`src/encryption.rs`):** OWASP-compliant Argon2id params (16 MiB memory, 3 iterations, 4 parallelism), AES-256-GCM.

**Status:** VERIFIED

---

### 1.7 Protocol Buffers

**Documented:** All RPCs listed match proto definition.

**Status:** VERIFIED

---

### 1.8 Settings

**Documented:** `server_url`, `api_key`, `direction`, `interval`.

**Actual:** Additional fields: `device_id`, `auto_sync`, `clipboard_auto_clear_seconds`.

**Status:** MINOR DISCREPANCY - Documentation is incomplete but not incorrect.

---

### 1.9 Error Handling

**Documented:** `SnipError::Sync`, `SnipError::Grpc`, `SnipError::Encryption`.

**Actual:** Uses `SnipError::runtime_error()` for sync errors, not custom variants.

**Status:** MINOR DISCREPANCY - Error types not as documented.

---

## 2. Bugs Found

### 2.1 Push-Only Mode Counter Bug (`sync_commands.rs:306-323`)

```rust
if direction == SyncDirection::Push {
    if !has_failures {
        let _ = mgr.update_last_sync(lib_name, new_timestamp);
    }
    completed += 1;  // <-- ONLY incremented here
    if has_failures {
        results.push(...);
    } else {
        results.push((lib_name.clone(), true, String::new()));
    }
    continue;  // <-- Skips to next library WITHOUT incrementing completed when has_failures=true
}
```

**Impact:** When pushing with encryption failures, `completed` is not incremented, causing the progress counter to be wrong and potentially causing the loop to process the same library again.

**Severity:** Medium

---

### 2.2 Merge Equal Timestamp Bug (`sync_commands.rs:429`)

```rust
if server_snip.updated_at > local_snip.updated_at {
    // server wins
} else {
    merged_snippets.push((*local_snip).clone());  // local wins on equal timestamps
}
```

**Impact:** When both devices edit a snippet simultaneously with identical timestamps, local changes win over server. This can cause data loss on the server side.

**Severity:** Medium

---

### 2.3 Missing `output` Field in Encryption (`sync.rs:60-64`)

```rust
struct EncryptedSnippetData {
    description: String,
    command: String,
    tags: Vec<String>,
    // output field is MISSING
}
```

**Impact:** The `output` field (cached command output) is not encrypted. This could leak sensitive data if the output contains sensitive information.

**Severity:** Medium (Security)

---

### 2.4 Missing `delete_library` in SyncClient

**Impact:** The documented `delete_library()` method is not implemented in `SyncClient`. Users cannot delete libraries via the sync client.

**Severity:** Low (Can delete via local file system)

---

## 3. Potential Improvements

### 3.1 Error Propagation

`run_sync()` uses `eprintln!` and silent returns instead of propagating errors. Consider returning `Result` types to allow callers to handle failures appropriately.

---

### 3.2 Configurable Retry Parameters

Retry parameters (`MAX_RETRIES`, `INITIAL_DELAY_MS`, `MAX_DELAY_MS`) are hardcoded constants. These could be made configurable via `SyncSettings`.

---

### 3.3 TLS Server Name Verification

`create_tls_channel()` (`sync.rs:299-316`) does not verify server certificate hostnames:

```rust
let tls_config = ClientTlsConfig::new()
    .with_enabled_roots()
    .assume_http2(true);  // No server_name verification
```

Consider adding `server_name` verification to prevent MITM attacks.

---

### 3.4 Missing `get_snippets` and `push_snippets` Client Methods

The proto defines `GetSnippets` and `PushSnippets` RPCs, but `SyncClient` only exposes `sync_encrypted()`. These could be useful for incremental syncs without full merge logic.

---

### 3.5 Hardcoded Sync Limit

`sync_encrypted()` hardcodes `limit: 1000` in the request. Consider making this configurable for large libraries.

---

### 3.6 Device ID Conflict Detection

No validation that `device_id` is unique per user/library. A user could accidentally overwrite another device's snippets if IDs collide.

---

### 3.7 Library Identification by Filename

Libraries are matched by filename (`sync_commands.rs:187`). If a user renames a library file, sync breaks. Consider using a stable library ID instead.

---

### 3.8 Sync Status Reporting

When sync succeeds but with skipped snippets, the result message is not prominently displayed:
```rust
if !msg.is_empty() {
    println!("  {} - {}", name, msg);
}
```

The message format could be clearer (e.g., warning color, separate summary count).

---

### 3.9 Backup on Merge Failure

`library::backup_library()` is called before save but its error is silently ignored:
```rust
let _ = library::backup_library(&lib_path).ok();
```

Consider logging backup failures.

---

### 3.10 Retryable vs Non-Retryable Error Classification

All gRPC errors trigger the same retry behavior. Distinguishing between retryable (network) and non-retryable (auth, not found) errors could improve UX.

---

## 4. Security Considerations

### 4.1 API Key in Memory

The API key is cloned into each `SyncRequest`. Consider:
- Using a secure memory wipe after use
- Zeroizing the key when dropped

### 4.2 Output Field Not Encrypted

As noted in bug 2.3, the `output` field is not encrypted. This is a data leakage vector.

### 4.3 TLS Verification

As noted in3.3, TLS server name verification is not performed.

---

## 5. Summary

| Category | Count |
|----------|-------|
| Verified claims | 6 |
| Discrepancies | 4 |
| Bugs | 4 |
| Potential improvements | 10 |
| Security concerns | 3 |

**Priority fixes:**
1. Push-only mode counter bug (2.1)
2. Merge equal timestamp bug (2.2)
3. Missing `output` in encryption (2.3)
4. TLS verification (3.3)
