# Sync Module Skill

## Purpose
Guide agents through working with the sync module (`src/sync.rs`, `src/sync_commands.rs`, `src/commands/sync_cmd.rs`).

## Known Issues

### PERF-3: Argon2 Key Derivation Per-Snippet (PARTIALLY ADDRESSED)
**Location**: `src/sync.rs`, `src/encryption.rs`

Each snippet gets a new random salt, running Argon2 key derivation for every single snippet. A session-local key cache (`KEY_CACHE` in `encryption.rs`) now avoids re-deriving keys for the same (api_key, salt) pair, but each unique salt still triggers a fresh Argon2 run. The cache is cleared at the end of sync via `clear_key_cache()`.

## Sync Flow

```
run_sync() flow (sync_commands.rs:365-742):
1. Validate config (api_key, device_id)
2. Create SyncClient with TLS
3. Health check
4. Resolve libraries to sync
5. Create missing libraries on server (first loop)
6. Per-library sync (second loop):
   - Push: encrypt local snippets, send to server
   - Pull: fetch server snippets, decrypt, merge locally
   - Bidirectional: both directions
7. Save merged snippets
8. Update last_sync timestamp (only if no encryption failures)
```

**Note:** Encryption failures are tracked via `skipped_count`/`skipped_ids` in the response. `last_sync` is NOT updated when there are failures, preventing permanent snippet loss.

## Merge Strategy

Last-write-wins based on `updated_at` timestamp:
- Server deleted + local not deleted → mark local as deleted
- Both deleted → exclude from output
- Server newer → server wins, preserve local-only fields (`output`, `folders`, `favorite`)
- Local newer or equal → local wins
- Local deleted → NOT resurrected by newer server copy (local-deleted-wins)
- Local-only snippets → preserved unchanged

## Key Functions

| Function | Location | Purpose |
|----------|----------|---------|
| `run_sync()` | `sync_commands.rs:365-742` | Main sync orchestration |
| `merge_snippets()` | `sync_commands.rs:744-851` | Merge algorithm |
| `encrypt_snippet()` | `sync.rs:518-544` | Encrypt snippet for server |
| `decrypt_snippet()` | `sync.rs:547-571` | Decrypt snippet from server |
| `sync_with_retry()` | `sync.rs:261-304` | Retry logic with exponential backoff |
| `SyncExecutionLock::wait_acquire()` | `auto_sync/execution_lock.rs` | Bounded-time lock acquisition for foreground callers |
| `SyncExecutionLock::try_acquire()` | `auto_sync/execution_lock.rs` | Non-blocking lock acquisition for workers |
| `clear_pending_after_explicit_sync()` | `auto_sync/notification.rs` | Generation-safe pending clear after manual sync |

**Note:** The executor subprocess (`auto-sync-execute`) invokes `run_sync`
directly — it does NOT acquire the `SyncExecutionLock`. The worker owns the
lock for the entire detached cycle.

## Test Coverage

Tests in `sync_commands.rs:896-1180`:
- `test_server_wins_with_newer_timestamp`
- `test_local_wins_with_newer_timestamp`
- `test_new_server_snippet_added`
- `test_deleted_server_snippet_excluded`
- `test_server_delete_local_already_deleted_excluded`
- `test_local_only_snippet_preserved`
- `test_local_deleted_snippet_not_preserved`
- `test_merge_preserves_folders`
- `test_merge_sorted_by_updated_at_descending`
- `test_local_deleted_not_resurrected_by_newer_server`
- `test_proto_snippet_excludes_usage_metadata`
- `test_merge_preserves_local_output_when_server_wins`

Missing: No tests for encryption roundtrip through sync, retry logic, or the critical encrypt-failure + timestamp-update interaction.
