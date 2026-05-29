# Server (snip-sync) Improvement Plan

## Architecture Claims vs. Implementation Verification

### 1. Database Schema

| Claim | Status | Notes |
|-------|--------|-------|
| `users` table with `id`, `api_key_hash`, `api_key_prefix`, `device_id`, `created_at` | **INCORRECT** | Schema uses `api_key` (hashed, not prefixed), `api_key_prefix`, no `device_id` in users table. `device_id` is in `snippets` table |
| `libraries` table with `id`, `user_id`, `name`, `created_at` | **INCORRECT** | Schema has `deleted_at` column, not `deleted` |
| `snippets` table with listed columns | **CORRECT** | Schema matches |

### 2. API Key Hashing

| Claim | Status | Notes |
|-------|--------|-------|
| Argon2id hash of API key | **VERIFIED** | `hash_api_key()` uses Argon2id with OWASP minimum params |
| SHA-256 prefix (first 8 chars of base64) for indexed lookup | **VERIFIED** | `compute_api_key_prefix()` implemented |
| `migrate_plaintext_api_keys()` backfills hashes for legacy data | **VERIFIED** | Tested and working |

### 3. Key Operations

| Operation | Status | Notes |
|-----------|--------|-------|
| `create_user(api_key)` | **VERIFIED** | Creates user + default library |
| `get_user_by_api_key(key)` | **VERIFIED** | Prefix lookup + hash verification |
| `upsert_snippet(snippet, user_id, library_id)` | **VERIFIED** | Includes conflict resolution by timestamp |
| `get_snippets(user_id, library_id, since, limit, offset)` | **VERIFIED** | Paginated, filters `deleted=0` |
| `get_latest_timestamp(user_id, library_id)` | **VERIFIED** | Returns MAX updated_at |
| `create_library(user_id, name)` | **VERIFIED** | Validates name constraints |
| `list_libraries(user_id, limit, offset)` | **VERIFIED** | Returns snippet counts |
| `delete_library(user_id, library_id)` | **VERIFIED** | Soft-delete + cascade to snippets |
| `verify_library_ownership(user_id, library_id)` | **VERIFIED** | Checks deleted_at |

### 4. gRPC Service

| RPC | Status | Notes |
|-----|--------|-------|
| Health | **VERIFIED** | Returns version + healthy |
| Register | **VERIFIED** | Rate limited by IP, returns api_key + device_id |
| GetSnippets | **VERIFIED** | Pagination with defaults |
| PushSnippets | **VERIFIED** | Validates and reports accepted/rejected |
| Sync | **VERIFIED** | Bidirectional with skipped_ids |
| CreateLibrary | **VERIFIED** | Name validation |
| ListLibraries | **VERIFIED** | Pagination with defaults |
| DeleteLibrary | **VERIFIED** | Prevents deleting "default" |
| ListPremadeLibraries | **VERIFIED** | No auth required |
| GetPremadeLibrary | **VERIFIED** | Sanitizes filename |

### 5. Input Validation

| Claim | Status | Notes |
|-------|--------|-------|
| Max command length: 1024 bytes | **VERIFIED** | Implemented |
| Max description length: 1024 bytes | **VERIFIED** | Implemented |
| Max tags: 50 | **VERIFIED** | Implemented |
| Max tag length: 100 bytes | **VERIFIED** | Implemented |
| Request timeout: 30s (configurable) | **VERIFIED** | Implemented |

### 6. HTTP Server

| Endpoint | Status | Notes |
|----------|--------|-------|
| `/health` GET | **VERIFIED** | No auth, returns JSON |
| `/metrics` GET | **VERIFIED** | Basic auth required |

### 7. Rate Limiter

| Claim | Status | Notes |
|-------|--------|-------|
| Default: 120 req/min per API key | **VERIFIED** | Implemented |
| Sliding window | **VERIFIED** | Uses retain on entry |
| Background cleanup every 60s | **VERIFIED** | Implemented in spawn loop |

### 8. Metrics

| Metric | Status | Notes |
|--------|--------|-------|
| `snip_sync_requests_total` | **VERIFIED** | Implemented |
| `snip_sync_sync_operations_total` | **VERIFIED** | Implemented |
| `snip_sync_library_operations_total` | **VERIFIED** | Implemented |
| `snip_sync_rate_limit_hits_total` | **VERIFIED** | Implemented |
| `snip_sync_auth_failures_total` | **VERIFIED** | Implemented |
| Protected by HTTP Basic auth | **VERIFIED** | `METRICS_USERNAME`/`METRICS_PASSWORD` |

### 9. Premade Manager

| Claim | Status | Notes |
|-------|--------|-------|
| Scans `.toml` files | **VERIFIED** | Impl |
| Extracts count + description | **VERIFIED** | Both optional, defaults applied |
| Serves content via gRPC | **VERIFIED** | Implemented |
| Path traversal prevention | **VERIFIED** | Canonicalize + prefix check |

### 10. Configuration

| Claim | Status | Notes |
|-------|--------|-------|
| `config.toml` + env var overrides | **VERIFIED** | Full precedence chain implemented |
| All documented fields present | **VERIFIED** | All config values match |

---

## Bugs / Edge Cases Found

### 1. **Path Traversal in Premade get() - Time-of-Check-to-Time-of-Use (TOCTOU)**

**File**: `premade.rs:187-206`

```rust
let canonical_dir = self.dir.canonicalize().unwrap_or_else(|_| self.dir.clone());
let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());

if !canonical_path.starts_with(&canonical_dir) {
    return Err(Status::invalid_argument(
        "Invalid filename: path traversal detected",
    ));
}

fs::read_to_string(&path).map_err(...); // <-- uses original `path`, not canonical_path
```

**Bug**: The canonicalization check validates `canonical_path`, but `fs::read_to_string` reads from `path`. An attacker could exploit a symlink race condition.

**Fix**: Read from `canonical_path` instead of `path`.

---

### 2. **Premade File Content Not Sanitized Before Serving**

**File**: `premade.rs:208`

`get()` returns raw file content without running `fix_invalid_toml_escapes()`. If a premade file has invalid TOML escapes, clients receive malformed content.

---

### 3. **Missing Input Validation on `api_key` Field**

**File**: `main.rs:389`

The `register` RPC accepts any content in the `RegisterRequest`. The `api_key` field (which becomes the user's API key) is not validated for length or format. A malicious client could submit extremely long strings.

---

### 4. **Sync Operation Returns `skipped_count` but Doesn't Filter Server Snippets by Vault**

**File**: `main.rs:549-654`

When `sync` returns server snippets, it does not filter deleted snippets (deleted: true) from the result. The `get_snippets()` query filters `deleted = 0`, but the caller might need awareness of deletions for merge logic. The architecture doc says "Server `deleted: true` snippets are excluded from merge (destructive)" which is correct here — but this design should be documented.

---

### 5. **Race Condition in Rate Limiter Cleanup Task**

**File**: `rate_limiter.rs:17-27`

The spawned cleanup task holds a lock across an `await` point (`tokio::time::sleep`). If the task panics, the lock is poisoned. Should use a separate channel to trigger shutdown instead.

```rust
tokio::spawn(async move {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await; // <-- await while holding lock
        let mut requests = requests_clone.lock().await;
        ...
    }
});
```

---

### 6. **No Limits on `local_snippets` Array in Sync**

**File**: `main.rs:580`

The `SyncRequest.local_snippets` array has no size limit. A client could send millions of snippets, causing memory exhaustion. Unlike `PushSnippets` where validation errors are tracked, sync silently skips validation failures but doesn't limit array size.

---

### 7. **Hardcoded `MAX_REQUEST_LIMIT` Magic Number**

**File**: `main.rs:35`

```rust
const MAX_REQUEST_LIMIT: i32 = 1000;
```

This limits fetched snippets per request. The architecture doc doesn't mention this limit, and it could cause sync chunks to be too small for large libraries. It's not configurable.

---

## Potential Improvements

### 1. **Make `MAX_REQUEST_LIMIT` Configurable**
Add to `LimitsConfig` and environment variable `MAX_REQUEST_LIMIT`.

### 2. **Add Batch Size Limit for Sync local_snippets**
Validate `req.local_snippets.len()` against a reasonable limit (e.g., 1000) to prevent memory exhaustion.

### 3. **Validate `api_key` Length in `register`**
Add a max length check (e.g., 256 bytes) to prevent resource exhaustion from malicious keys.

### 4. **Fix TOCTOU in Premade `get()`**
Use the canonicalized path for file reads.

### 5. **Apply TOML Fix to Premade `get()` Content**
Run `fix_invalid_toml_escapes()` in `get()` so clients receive valid TOML.

### 6. **Graceful Shutdown for Rate Limiter Cleanup Task**
Use a shutdown signal (channel) instead of running forever.

### 7. **Add SQL Injection Defense in Library Delete**
The `delete_library` check for `req.library_id == "default"` is a string comparison, but the input is User ID. The library name "default" is checked instead of protecting the UUID-based library ID, which is already handled correctly by the code but the check is confusingly redundant.

### 8. **Document the `deleted` Snippet Merge Semantics**
The architecture should explicitly note that `deleted: true` snippets returned in sync responses are excluded from client merge — they signal destruction, not tombstoning.

### 9. **Add Health Check for Database Connectivity**
The `Health` RPC returns `healthy: true` unconditionally. It should ping the database to verify connectivity.

### 10. **Consider TLS Warning is Only in Startup Log**
The TLS warning at `main.rs:829-831` only appears in logs, not in any health/ready endpoint. A production deployment may miss this.

---

## Discrepancies Between Documentation and Implementation

1. **Database `users` table columns** (minor): Doc mentions `device_id` column, but it doesn't exist. The `device_id` is stored in `snippets` table.

2. **Database `libraries` table columns** (minor): Doc says `deleted` column, but code uses `deleted_at` for soft-delete.

3. **Premade Manager TOML escaping**: Doc doesn't mention that `fix_invalid_toml_escapes()` is applied only during scanning (`list()` uses it), not during `get()`. This is an inconsistency — scanning sanitizes but serving doesn't.

4. **Not documented as configurable**: `MAX_REQUEST_LIMIT` (1000) is not documented and not configurable.

5. **`Sync` RPC undocumented limit**: The sync endpoint uses `limit=1000` as default vs. `GetSnippets` using `limit=100`. This difference is not documented.
