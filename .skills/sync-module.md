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

## Failure Classification and Retry (Phase 03)

### SyncFailureKind Enum

`SyncFailureKind` (`src/error.rs`) provides typed error variants for sync operations:

| Variant | Maps to FailureClass | Source |
|---------|---------------------|--------|
| `NotConfigured` | DeferredNotConfigured | sync_commands.rs |
| `ConnectFailed` | TransientNetwork | sync.rs |
| `HealthCheckFailed` | TransientNetwork | sync_commands.rs |
| `AuthenticationFailed` | Authentication | sync.rs |
| `SyncRequestFailed` | TransientNetwork | sync.rs |
| `CreateLibraryFailed` | Configuration | sync.rs |
| `GetPremadeLibraryFailed` | TransientNetwork | sync.rs |
| `RegistrationFailed` | Authentication | sync.rs |
| `LibraryManagerInitFailed` | LocalPersistence | sync_commands.rs |
| `LibraryModeInitFailed` | LocalPersistence | sync_commands.rs |
| `LibrariesDirReadFailed` | LocalPersistence | sync_commands.rs |
| `NoLibrariesToSync` | Internal | sync_commands.rs |
| `SaveMergedLibraryFailed` | LocalPersistence | sync_commands.rs |
| `PartialSyncFailure` | Partial | sync_commands.rs |
| `PremadePartialFailure` | Partial | sync_commands.rs |
| `EncryptionFailed` | Internal | sync.rs |
| `DecryptionFailed` | Internal | sync.rs |

### FailureClass Enum

`FailureClass` (`src/auto_sync/policy.rs`) classifies sync errors into 11 variants:

| Variant | Meaning | Retry Disposition |
|---------|---------|-------------------|
| `DeferredDisabled` | Auto-sync disabled at runtime | WaitForConfigurationChange |
| `DeferredNotConfigured` | Missing api_key, server_url, or library mapping | WaitForConfigurationChange |
| `TransientNetwork` | DNS, connection refused, TLS handshake failure | RetryAfter(exponential backoff) |
| `TransientTimeout` | gRPC deadline exceeded or sync timeout hit | RetryAfter(exponential backoff) |
| `Authentication` | Invalid API key, expired token, auth rejected | RequiresAttention |
| `Configuration` | Corrupt config, bad schema, invalid library path | RequiresAttention |
| `Conflict` | Merge conflict or protocol version mismatch | RequiresAttention |
| `Partial` | Some snippets synced, others failed | RequiresAttention |
| `LocalPersistence` | Disk full, permission denied on config dir | RequiresAttention |
| `CredentialStore` | Keyring/keychain unavailable or locked | RequiresAttention |
| `Internal` | Unrecoverable bug or unexpected invariant violation | RetryAfter (bounded to 3 attempts) |

### Classification: Variant-Based (Not String Matching)

`classify_sync_error()` in `executor.rs` delegates to `FailureClass::from_error()` in `policy.rs`. For `SnipError::SyncFailure` variants, classification is direct variant matching — no string analysis. For legacy `SnipError::Runtime` variants, fallback heuristic string matching is used.

### Exponential Backoff

`transient_backoff(consecutive_failures: u32) -> Duration` computes capped exponential backoff with jitter: ~5s, ~15s, ~30s, ~60s, then exponential growth capped at 15 minutes. Jitter is 0-20% of base delay.

### Status Persistence

`auto-sync-status.toml` in the state directory records the last failure classification, attempt count, next retry timestamp, and a config fingerprint for deferral release detection. Messages are sanitized: control characters stripped, Bearer tokens and API key values redacted.

### Config Fingerprint and Deferral Release

`compute_config_fingerprint()` hashes non-secret structural inputs (server URL, enabled flags, direction, API key presence). `release_deferral_on_config_change()` checks if the fingerprint has changed since a deferred failure; if so, it clears `attention_required`, resets `consecutive_failures`, and permits a new attempt.

### Schedule Decision

`schedule_sync()` in schedule.rs is the centralized entry point for all worker scheduling decisions:

1. Policy configured and enabled.
2. Pending marker exists with valid work.
3. Execution lock is not held by a live process.
4. Backoff delay has elapsed (unless explicit retry).
5. Failure class allows automatic retry.
6. Config change releases deferred failures (authentication/credential/configuration).
