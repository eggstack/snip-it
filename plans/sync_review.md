# Sync Module Review

**Date**: 2026-05-29
**Files Reviewed**: `src/sync.rs` (384 lines), `src/sync_commands.rs` (680 lines), `src/commands/sync_cmd.rs` (253 lines), `architecture/sync.md` (128 lines), `snip-proto/src/snip_proto.rs` (generated), `src/encryption.rs` (325 lines), `src/config.rs` (178 lines), `src/library.rs` (760 lines)

---

## 1. Document Accuracy

### Verified Correct (architecture/sync.md)

- **SyncClient struct**: Matches `src/sync.rs:69-72` — wraps `SnippetSyncClient<Channel>` + `SyncSettings`. Correct.
- **TLS with native roots**: `src/sync.rs:304-306` — `ClientTlsConfig::new().with_enabled_roots().assume_http2(true)`. Correct.
- **10s connect timeout, 30s request timeout**: `src/sync.rs:310-311`. Correct.
- **Retry constants**: MAX_RETRIES=3, INITIAL_DELAY_MS=100, MAX_DELAY_MS=5000 — `src/sync.rs:26-28`. Correct.
- **Encryption flow**: `encrypt_snippet()` at `src/sync.rs:318-345` and `decrypt_snippet()` at `src/sync.rs:347-372`. JSON serialization of description+command+tags, encrypt with API key, store in `command` field with `encrypted: true`. Correct.
- **Merge strategy rules 1-7**: `src/sync_commands.rs:375-456` — all 7 rules verified in order.
- **Local-only fields**: `output`, `folders`, `favorite` — preserved when server wins at `src/sync_commands.rs:411-423`. Correct.
- **`run_sync()` flow**: Validate → create client → health check → resolve libraries → push → pull → merge → save → update timestamp. Matches `src/sync_commands.rs:118-366`. Correct.
- **Premade sync**: `run_premade_sync()` at `src/sync_commands.rs:57-116` — lists premade, skips existing, saves to premade dir. Correct.

### Discrepancies

| Claim in architecture/sync.md | Actual Code | Severity |
|-------------------------------|-------------|----------|
| "`sync.rs` (384 lines)" | Correct: 384 lines | — |
| "`sync_commands.rs` (680 lines)" | Correct: 680 lines | — |
| Methods table lists `sync_encrypted(local, last_sync, library_id)` as "Full bidirectional sync" | The method is used for push, pull, and bidirectional depending on what `local_snippets` is passed. For pull-only, an empty `vec![]` is passed. The doc description is misleading — it's not inherently "bidirectional". | Low |
| Architecture says "Last-write-wins based on `updated_at` timestamp" | Code at `sync_commands.rs:410` uses strict `>` (greater than), not `>=`. Equal timestamps mean local wins (`sync_commands.rs:424-426`). This is correct "local wins ties" semantics but the doc implies pure last-write-wins without clarifying tie-breaking. | Low |
| Architecture doesn't mention the `delete_library` gRPC method exists | Generated proto has `DeleteLibrary` RPC, and client has `delete_library()` method, but `SyncClient` never exposes it. Dead proto method. | Low |

---

## 2. Bugs & Issues

### BUG-1: `run_sync()` duplicates library iteration — server library creation loop then sync loop (Critical)

**Location**: `src/sync_commands.rs:186-228` and `src/sync_commands.rs:234-366`

The function iterates `libraries_to_sync` twice:
1. First loop (lines 186-228): Creates missing libraries on the server and links them.
2. Second loop (lines 234-366): Performs actual sync for each library.

**Problem**: After the first loop creates a library on the server and calls `mgr.update_library_id()`, the `mgr` object's in-memory config is updated, but the **second loop re-reads `library_id` from the same `mgr`** (line 245-248). This works in-memory. However, if the first loop's `create_library` call succeeds but `update_library_id` fails (line 213-215), the second loop will see an empty `library_id` and skip the library (line 250-253) — a silent failure with only a warning.

More critically: if a library was just created on the server in the first loop, its `last_sync` is 0. The second loop's push filter (`s.updated_at >= _last_sync || s.created_at >= _last_sync`) will push **all** snippets, even unmodified ones, on every sync. This is wasteful but not data-corrupting.

**Impact**: Medium — causes unnecessary full pushes for newly created libraries. The `last_sync` should be updated to the server timestamp after library creation, or the first loop's results should carry forward to the second.

### BUG-2: `sync_with_retry` clones the entire request on every retry attempt (Medium)

**Location**: `src/sync.rs:149-184`

```rust
for attempt in 0..=MAX_RETRIES {
    let req = SyncRequest {
        api_key: request.api_key.clone(),
        local_snippets: request.local_snippets.clone(),
        ...
    };
```

Every retry attempt deep-clones the entire `Vec<Snippet>` including all encrypted snippet data. For large libraries (hundreds of snippets), this means 4 copies of the full payload are allocated (1 original + 3 retries). The `retry_grpc!` macro doesn't have this problem because it takes a future expression that is re-evaluated.

**Impact**: Medium — memory pressure on large sync operations, but not functionally broken.

### BUG-3: `health_check()` swallows all errors (Low)

**Location**: `src/sync.rs:186-194`

```rust
pub async fn health_check(&mut self) -> SnipResult<bool> {
    match retry_grpc!(...) {
        Ok(response) => Ok(response.into_inner().healthy),
        Err(_) => Ok(false),
    }
}
```

A TLS certificate error, DNS failure, or connection timeout all return `Ok(false)` — indistinguishable from a server that responded with `healthy: false`. The caller in `sync_commands.rs:48-54` then prints "Server is not reachable" which is correct, but the actual error reason is lost.

**Impact**: Low — makes debugging connection issues harder. The error should be logged or propagated.

### BUG-4: Encryption failures silently skip snippets without aborting sync (Medium)

**Location**: `src/sync.rs:96-107`

```rust
for s in &local_snippets {
    match encrypt_snippet(&api_key, s) {
        Ok(es) => encrypted_snippets.push(es),
        Err(e) => {
            encrypt_failed_ids.push(s.id.clone());
            tracing::warn!("Failed to encrypt snippet {}: {}", s.id, e);
        }
    }
}
```

If encryption fails for some snippets (e.g., corrupted data), they are silently excluded from the sync request. The server never receives them, but the client's `last_sync` timestamp is updated (line 319), meaning those snippets will **never** be retried on subsequent syncs. They are effectively lost from sync permanently.

**Impact**: High — data loss scenario. Failed snippets should either block the sync or be retried.

### BUG-5: `run_sync()` updates `last_sync` even when sync response is partial/failure (Medium)

**Location**: `src/sync_commands.rs:300-306` (push path)

```rust
if direction == SyncDirection::Push {
    let _ = mgr.update_last_sync(lib_name, new_timestamp);
    completed += 1;
    results.push((lib_name.clone(), true, String::new()));
    continue;
}
```

And `src/sync_commands.rs:319` (bidirectional path):
```rust
let _ = mgr.update_last_sync(lib_name, new_timestamp);
```

The `last_sync` is updated even when `response.success` is true but `response.skipped_count > 0` (some snippets failed encryption/decryption). On next sync, those failed snippets won't be re-sent because their `updated_at` hasn't changed.

**Impact**: High — same root cause as BUG-4. Skipped snippets are permanently excluded from sync.

### BUG-6: Pull-only path doesn't check `response.success` (Low)

**Location**: `src/sync_commands.rs:340-363`

```rust
if direction == SyncDirection::Pull && !library_id.is_empty() {
    let result = runtime.block_on(client.sync_encrypted(vec![], _last_sync, &library_id));
    match result {
        Ok(response) => {
            if response.success {
                // ... merge and save
            }
            // No else — non-success silently ignored
        }
```

If `response.success` is false, the pull silently does nothing. No error message, no result recorded. Compare with the push/bidirectional path which does record the failure.

**Impact**: Low — user gets no feedback when pull fails.

### BUG-7: `created_at` overwritten for snippets getting IDs for the first time (Low)

**Location**: `src/sync_commands.rs:266-273`

```rust
for (idx, s) in snippets.snippets.iter_mut().enumerate() {
    if s.id.is_empty() {
        s.id = uuid::Uuid::new_v4().to_string();
        s.created_at = now;
        s.updated_at = now;
```

When assigning IDs to existing snippets that were created earlier (before IDs were required), both `created_at` and `updated_at` are overwritten to the current time. This loses the original creation timestamp, which could affect sort order and merge behavior on the server.

**Impact**: Low — affects only legacy snippets without IDs, and the timestamps are likely 0 or close to creation time anyway.

### BUG-8: `sync_cmd.rs` calls `run_sync` twice in some code paths (Medium)

**Location**: `src/commands/sync_cmd.rs:204-252`

When `servers` is false and `list_libraries()` succeeds (lines 210-233), `run_sync` is called at line 224 and the function returns. However, if `list_libraries()` fails (line 234), the code falls through to line 237-251, where it tries `list_and_link_server_libraries` AND then calls `run_sync` again at line 244. This means:

1. If `list_libraries()` succeeds: sync runs once.
2. If `list_libraries()` fails AND api_key+device_id are set: sync runs once (after re-linking).
3. If `list_libraries()` fails AND api_key+device_id are empty: sync runs once.

This isn't a double-call bug, but the control flow is confusing and fragile. The `list_and_link_server_libraries` function also calls `list_libraries()` internally, which could fail again.

**Impact**: Low — not a functional bug, but convoluted control flow.

---

## 3. Design Issues

### DESIGN-1: `run_sync()` is a 250-line monolithic function

**Location**: `src/sync_commands.rs:118-366`

This function handles:
- Direction resolution
- Config validation
- Client creation
- Health check
- Library discovery
- Library creation on server (first loop)
- Per-library sync (second loop)
- Push, pull, and bidirectional paths
- Backup creation
- Result reporting

It should be decomposed into smaller functions for testability and readability.

### DESIGN-2: Mixed error handling patterns

- `run_sync()` uses `eprintln!` + early return (lines 25-31, 140-143)
- `create_sync_client()` swallows the error with `.ok()` (line 40)
- `run_premade_sync()` uses `eprintln!` + early return
- `run()` in `sync_cmd.rs` returns `SnipResult<()>` but internally falls back to `SyncSettings::default()` on error (line 176)

There's no consistent error propagation strategy. Some errors are logged, some are swallowed, some are returned.

### DESIGN-3: `SyncClient` API key sent in every request as plaintext

**Location**: `src/sync.rs:94, 110, 222, etc.`

The API key is cloned and sent in every gRPC request body. While TLS protects it in transit, it's duplicated across many requests. A token/session approach would be more efficient and reduce the attack surface (the key never leaves the client after initial auth).

### DESIGN-4: `SyncClient` methods take `&mut self` unnecessarily

**Location**: `src/sync.rs:88, 186, 221, 235, 263, 275`

All methods on `SyncClient` take `&mut self` because the underlying gRPC client requires `&mut self` for the `ready()` call. This prevents sharing a single client across concurrent operations. Not a bug, but a design limitation that prevents parallel library syncs.

### DESIGN-5: No pagination for `list_libraries()`

**Location**: `src/sync.rs:221-233`

```rust
pub async fn list_libraries(&mut self) -> SnipResult<Vec<Library>> {
    // ...
    limit: 50,
    offset: 0,
```

Hardcoded `limit: 50` with no pagination loop. If a user has more than 50 libraries, only the first 50 are returned. The response has `has_more` and `total_count` fields (proto lines 150-151) but they're never checked.

### DESIGN-6: `SyncDirection` default is `Push` but sync is called as `Bidirectional`

**Location**: `src/config.rs:54-55` vs `src/sync_commands.rs:131`

The config default is `SyncDirection::Push`, but `run_sync()` defaults to `Bidirectional` when neither `push_only` nor `pull_only` is set (line 131). The config's `sync_direction` field is never actually used in the sync flow — it's overridden by the CLI flags.

### DESIGN-7: Redundant `let _ = mgr.update_last_sync(...)` — error silently discarded

**Location**: `src/sync_commands.rs:303, 319, 357`

```rust
let _ = mgr.update_last_sync(lib_name, new_timestamp);
```

If updating the last_sync timestamp fails (e.g., disk full), the error is silently discarded. On the next sync, all snippets will be re-synced unnecessarily.

---

## 4. Security Concerns

### SEC-1: API key transmitted in gRPC request bodies (Medium)

**Location**: `src/sync.rs:110` — `api_key: api_key.clone()` in every `SyncRequest`.

The API key is sent in every request payload. While TLS encrypts it in transit, if the server is compromised, the attacker sees the plaintext API key. The client uses the same key for both authentication AND encryption (`encryption::encrypt(api_key, ...)`), meaning a server compromise exposes the encryption key too.

**Recommendation**: Use a session token for auth; keep the API key client-side only for encryption.

### SEC-2: Argon2 memory cost is very low (Low)

**Location**: `src/encryption.rs:32`

```rust
const ARGON2_MEMORY_COST_KIB: u32 = 1 << 6; // 64 KiB
```

64 KiB memory cost is extremely low for Argon2id. OWASP recommends at minimum 19 MiB (19456 KiB) for Argon2id. The current setting provides minimal resistance against GPU/ASIC attacks on the derived key.

**Impact**: Low for this use case (API key is already somewhat low-entropy and transmitted over TLS), but worth noting.

### SEC-3: No certificate pinning (Low)

**Location**: `src/sync.rs:304-306`

```rust
let tls_config = ClientTlsConfig::new()
    .with_enabled_roots()
    .assume_http2(true);
```

Uses system certificate roots with no pinning. Vulnerable to MITM if a CA is compromised. Acceptable for most CLI tools but noted for completeness.

### SEC-4: Default server URL is HTTP, not HTTPS (Low)

**Location**: `src/config.rs:61`

```rust
fn default_sync_url() -> String {
    "http://localhost:50051".to_string()
}
```

Default is `http://localhost:50051`. For localhost this is fine, but the `create_tls_channel` function will fail to establish TLS on an HTTP URL, giving a confusing error. Users who copy the default URL format and change the host to a remote server may accidentally use HTTP.

---

## 5. Performance Issues

### PERF-1: Full sync request cloned on every retry (Medium)

**Location**: `src/sync.rs:155-161`

As noted in BUG-2, the entire `SyncRequest` including all snippet data is cloned for each retry attempt. For a library with 500 snippets, each ~1KB, this means 3 extra ~500KB allocations per sync.

### PERF-2: No streaming for large sync payloads (Medium)

The gRPC `Sync` RPC sends all snippets in a single request and receives all in a single response. For large libraries (thousands of snippets), this could exceed gRPC's default 4MB message limit. The code doesn't configure `max_decoding_message_size` or `max_encoding_message_size`.

**Location**: `src/sync.rs:82-85` — `SnippetSyncClient::new(channel)` uses defaults.

### PERF-3: Encryption is sequential (Low)

**Location**: `src/sync.rs:99-107`

Snippets are encrypted one at a time in a loop. Since each encryption involves Argon2 key derivation (which is intentionally slow), this is unnecessarily sequential. However, the same key is derived from the same API key + random salt each time, so the Argon2 computation is repeated for every snippet.

**Critical sub-issue**: Each snippet gets a new random salt (`encryption.rs:154-155`), meaning Argon2 key derivation runs for every single snippet. For 100 snippets, that's 100 Argon2 runs. The derived key could be cached (with a per-session random salt) to avoid redundant derivation.

### PERF-4: `merge_snippets` uses HashMap + HashSet for O(n+m) merge (Good)

**Location**: `src/sync_commands.rs:376-380`

The merge algorithm is well-designed: O(n) local map build + O(m) server iteration + O(n) local-only pass. This is efficient. No issue here.

---

## 6. Test Coverage Analysis

### Existing Tests

**`src/sync.rs` tests** (lines 374-384):
- `test_constants` — only verifies constant values. **No functional tests.**

**`src/sync_commands.rs` tests** (lines 463-679):
- `test_server_wins_with_newer_timestamp` — verifies merge with server newer
- `test_local_wins_with_newer_timestamp` — verifies merge with local newer
- `test_new_server_snippet_added` — verifies new server snippet added
- `test_deleted_server_snippet_excluded` — verifies server-deleted behavior
- `test_server_delete_local_already_deleted_excluded` — verifies both-deleted
- `test_local_only_snippet_preserved` — verifies local-only kept
- `test_local_deleted_snippet_not_preserved` — verifies local-deleted excluded
- `test_merge_preserves_folders` — verifies folder preservation
- `test_merge_sorted_by_updated_at_descending` — verifies sort order

### Coverage Gaps

| Area | Gap | Severity |
|------|-----|----------|
| `encrypt_snippet` / `decrypt_snippet` | No tests for roundtrip encryption through sync flow | High |
| `retry_grpc!` macro | No tests for retry behavior, backoff timing, max retries | High |
| `sync_with_retry` | No tests for retry logic, request cloning, error handling | High |
| `run_sync()` | No integration test for the full sync orchestration | High |
| `run_premade_sync()` | No tests | Medium |
| Push path (direction filtering) | No tests for `updated_at >= _last_sync` filter | Medium |
| Pull-only path | No tests | Medium |
| Library creation during sync | No tests for the server library creation loop | Medium |
| `health_check` error swallowing | No tests | Low |
| Encryption failure + skipped snippets | No tests for the interaction between encrypt failures and last_sync update | High |
| `sync_cmd.rs` library linking | No tests for `link_server_library` conflict resolution | Medium |

---

## 7. Priority Ranking

| Priority | ID | Description | Location |
|----------|----|-------------|----------|
| **Critical** | BUG-4/BUG-5 | Encryption failures cause permanent snippet loss (skipped + timestamp updated) | `sync.rs:96-107`, `sync_commands.rs:319` |
| **High** | SEC-1 | API key used for both auth and encryption, sent in every request body | `sync.rs:110` |
| **High** | PERF-3 | Argon2 key derivation repeated per-snippet instead of once per session | `sync.rs:331`, `encryption.rs:117` |
| **Medium** | BUG-2/PERF-1 | Full request cloned on every retry | `sync.rs:155-161` |
| **Medium** | DESIGN-1 | `run_sync()` is a 250-line monolith | `sync_commands.rs:118-366` |
| **Medium** | DESIGN-5 | No pagination for `list_libraries()` (hardcoded limit 50) | `sync.rs:228` |
| **Medium** | BUG-6 | Pull-only path silently ignores failures | `sync_commands.rs:340-363` |
| **Medium** | PERF-2 | No gRPC message size configuration | `sync.rs:82-85` |
| **Medium** | DESIGN-2 | Inconsistent error handling patterns | Throughout |
| **Low** | BUG-1 | Newly created libraries get full push on first sync | `sync_commands.rs:186-228` |
| **Low** | BUG-3 | `health_check` swallows errors | `sync.rs:186-194` |
| **Low** | BUG-7 | `created_at` overwritten for legacy snippets | `sync_commands.rs:266-273` |
| **Low** | BUG-8 | Confusing control flow in `sync_cmd.rs::run()` | `sync_cmd.rs:204-252` |
| **Low** | DESIGN-6 | Config `sync_direction` field unused | `config.rs:54-55` |
| **Low** | DESIGN-7 | `update_last_sync` errors silently discarded | `sync_commands.rs:303,319,357` |
| **Low** | SEC-2 | Argon2 memory cost too low (64 KiB vs OWASP 19 MiB) | `encryption.rs:32` |
| **Low** | SEC-4 | Default server URL uses HTTP | `config.rs:61` |

---

## 8. Recommendations

### Immediate (Critical/High)

1. **Fix BUG-4/BUG-5**: When encryption fails for any snippet, either:
   - (a) Abort the sync and don't update `last_sync`, OR
   - (b) Track failed snippet IDs and exclude them from the `last_sync` update so they're retried next time, OR
   - (c) Write failed snippet IDs to a retry queue file that's checked on next sync.

2. **Fix SEC-1**: Separate auth from encryption. Use the API key only for deriving the encryption key locally. For server auth, derive a separate auth token or use a session mechanism.

3. **Fix PERF-3**: Derive the encryption key once per sync operation (once per API key + a session salt) instead of per-snippet. The current approach runs Argon2 N times where N = number of snippets.

### Short-term (Medium)

4. **Decompose `run_sync()`**: Extract `create_missing_libraries()`, `sync_single_library()`, `push_to_server()`, `pull_from_server()` as separate functions.

5. **Add pagination to `list_libraries()`**: Loop with offset until `has_more` is false.

6. **Fix BUG-6**: Add error recording for pull-only failures.

7. **Configure gRPC message sizes**: Set appropriate `max_decoding_message_size` and `max_encoding_message_size` on the client.

### Long-term (Low)

8. **Add integration tests** for the full sync flow including encryption roundtrip through the sync pipeline.

9. **Standardize error handling**: Decide on a pattern (return errors, log + continue, log + abort) and apply consistently.

10. **Increase Argon2 memory cost** to OWASP-recommended minimum.

11. **Consider streaming RPC** for large library syncs instead of single-message request/response.
