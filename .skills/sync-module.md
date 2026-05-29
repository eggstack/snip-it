# Sync Module Skill

## Purpose
Guide agents through working with the sync module (`src/sync.rs`, `src/sync_commands.rs`, `src/commands/sync_cmd.rs`).

## Critical Known Issues

### BUG-4/5: Encryption Failures Cause Permanent Snippet Loss
**Location**: `src/sync.rs:96-107`, `src/sync_commands.rs:319`

When encryption fails for snippets, they are silently excluded from the sync request. The `last_sync` timestamp is still updated, meaning those snippets will never be retried on subsequent syncs.

**Fix approach**: When encryption fails, either:
- (a) Abort the sync and don't update `last_sync`, OR
- (b) Track failed snippet IDs and exclude them from the `last_sync` update, OR
- (c) Write failed snippet IDs to a retry queue file

### PERF-3: Argon2 Key Derivation Per-Snippet
**Location**: `src/sync.rs:331`, `src/encryption.rs:117`

Each snippet gets a new random salt, running Argon2 key derivation for every single snippet. For 100 snippets, that's 100 Argon2 runs. The derived key could be cached per sync session.

## Sync Flow

```
run_sync() flow:
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
8. Update last_sync timestamp
```

## Merge Strategy

Last-write-wins based on `updated_at` timestamp:
- Server deleted + local not deleted → mark local as deleted
- Both deleted → exclude from output
- Server newer → server wins, preserve local-only fields (`output`, `folders`, `favorite`)
- Local newer or equal → local wins
- Local-only snippets → preserved unchanged

## Key Functions

| Function | Location | Purpose |
|----------|----------|---------|
| `run_sync()` | `sync_commands.rs:118-366` | Main sync orchestration |
| `merge_snippets()` | `sync_commands.rs:375-456` | Merge algorithm |
| `encrypt_snippet()` | `sync.rs:318-345` | Encrypt snippet for server |
| `decrypt_snippet()` | `sync.rs:347-372` | Decrypt snippet from server |
| `sync_with_retry()` | `sync.rs:144-178` | Retry logic with exponential backoff |

## Test Coverage

Existing tests in `sync_commands.rs:463-679`:
- `test_server_wins_with_newer_timestamp`
- `test_local_wins_with_newer_timestamp`
- `test_new_server_snippet_added`
- `test_deleted_server_snippet_excluded`
- `test_server_delete_local_already_deleted_excluded`
- `test_local_only_snippet_preserved`
- `test_local_deleted_snippet_not_preserved`
- `test_merge_preserves_folders`
- `test_merge_sorted_by_updated_at_descending`

Missing: No tests for encryption roundtrip through sync, retry logic, or the critical encrypt-failure + timestamp-update interaction.
