# Sync Module Skill

## Purpose
Guide agents through working with the sync module (`src/sync.rs`, `src/sync_commands.rs`, `src/commands/sync_cmd.rs`).

## Known Issues

### PERF-3: Argon2 Key Derivation Per-Snippet (PARTIALLY ADDRESSED)
**Location**: `src/sync.rs`, `src/encryption.rs`

Each snippet gets a new random salt, running Argon2 key derivation for every single snippet. A session-local key cache (`KEY_CACHE` in `encryption.rs`) now avoids re-deriving keys for the same (api_key, salt) pair, but each unique salt still triggers a fresh Argon2 run. The cache is cleared at the end of sync via `clear_key_cache()`.

## Sync Flow

```
sync_encrypted() flow (sync.rs):
1. Encrypt local snippets
2. Build byte-bounded upload batches (Prost encoded_len, 3.5 MiB ceiling)
3. Sort batches by snippet ID for deterministic ordering
4. Send first batch via Sync RPC (upload + first response page)
5. Send remaining batches via PushSnippets RPC (upload only)
6. Paginate remaining response pages via Sync RPC (no upload)
7. Aggregate and decrypt all server snippets
8. Return merged SyncResponse

run_sync() flow (sync_commands.rs:599-...):
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

Live versions use deterministic ordering by `(updated_at, device_id,
SHA-256(synced-fields))`; this is role-independent for equal timestamps.
Deletion wins over live content even when the live timestamp is newer, so
explicit deletions are not silently resurrected. Both-deleted records are
omitted from display while required tombstones remain available for upload.
Server wins preserve local-only fields (`output`, `folders`, `favorite`).
Severe clock skew can still make one device dominate until its clock catches
up; this is wall-clock ordering, not CRDT or logical-clock reconciliation.

Missing remote libraries use an atomic `<library>.sync_recovery` TOML marker.
Startup and the normal missing-library path resume the existing
`Creating`/`RemoteCreated`/`Linked` phases. The server ID is recorded before
local relinking, linkage and `last_sync` are saved together, and
corrupt/mismatched/ambiguous recovery state blocks blind recreation. A linked
marker never creates a second remote library and is removed only after merged
content and the final cursor are durable.

## Key Functions

| Function | Location | Purpose |
|----------|----------|---------|
| `run_sync()` | `sync_commands.rs:599` | Main sync orchestration |
| `merge_snippets()` | `sync_commands.rs:1127` | Merge algorithm |
| `encrypt_snippet()` | `sync.rs:701` | Encrypt snippet for server |
| `decrypt_snippet()` | `sync.rs:734` | Decrypt snippet from server |
| `sync_with_retry()` | `sync.rs:380` | Retry logic with exponential backoff |
| `build_upload_batches()` | `sync.rs:972` | Byte-bounded batch splitting using Prost encoded_len |
| `accumulate_page()` | `sync.rs:410` | Decrypt and accumulate server snippets from a response page |
| `SyncRunLimits` | `sync.rs` | Internal automatic-sync deadline and request budget |
| `SyncExecutionLock::wait_acquire()` | `auto_sync/execution_lock.rs` | Bounded-time lock acquisition for foreground callers |
| `SyncExecutionLock::try_acquire()` | `auto_sync/execution_lock.rs` | Non-blocking lock acquisition for workers |
| `clear_pending_after_explicit_sync()` | `auto_sync/notification.rs` | Generation-safe pending clear after manual sync |

**Note:** The detached auto-sync helper invokes `run_sync_with_limits` directly
and owns the `SyncExecutionLock` for the entire detached cycle. Manual sync and
cron use the unbounded canonical wrapper.

## Test Coverage

Tests in `sync_commands.rs` (unit tests near end of file):
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

Tests in `sync.rs` (batching and clock skew):
- `test_build_upload_batches_empty_list`
- `test_build_upload_batches_single_small_item`
- `test_build_upload_batches_fits_one_request`
- `test_build_upload_batches_exact_boundary_fit`
- `test_build_upload_batches_one_byte_over_starts_new_batch`
- `test_build_upload_batches_oversized_single_item`
- `test_build_upload_batches_stable_id_ordering`
- `test_build_upload_batches_metadata_overhead_included`
- `test_build_upload_batches_no_batch_exceeds_ceiling`
- `test_clock_skew_invalid_argument_is_typed`
- `test_non_clock_skew_invalid_argument_is_generic`
- `test_request_too_large_failure_class`
- `test_clock_skew_failure_class`

Focused coverage also includes equal-timestamp role swaps, same-device content
fingerprint ties, delete/live role swaps, atomic recovery marker round trips,
and preservation of corrupt markers. Existing integration coverage continues to
cover encryption and retry/timestamp behavior.

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
| `Timeout` | TransientTimeout | sync.rs |
| `RequestTooLarge` | Configuration | sync.rs |
| `ClockSkew` | Configuration | sync.rs |

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

Auto-sync delegates error classification to `FailureClass::from_error()` in `policy.rs`. For `SnipError::SyncFailure` variants, classification is direct variant matching — no string analysis. For legacy `SnipError::Runtime` variants, fallback heuristic string matching is used.

### Exponential Backoff

`transient_backoff(consecutive_failures: u32) -> Duration` computes capped exponential backoff with jitter: ~5s, ~15s, ~30s, ~60s, then exponential growth capped at 15 minutes. Jitter is 0-20% of base delay.

**Note:** The `AutoSyncPolicy.max_retries` field was **removed** in Phase 06A — it was never read. Retry behavior is now driven entirely by durable backoff state in `auto-sync-status.toml`. The `SyncRetryConfig.max_retries` in `sync.rs` (controlling per-request gRPC retries within a single sync operation) is unaffected.

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

## Status Snapshot (Phase 04A)

`capture_snapshot()` in `src/status_snapshot.rs` produces a read-only `StatusSnapshot` aggregating all auto-sync artifacts. `snp status` exposes this as JSON (`--json`) or human-readable text.

**Top-level state precedence:** CorruptOrInaccessible → LiveExecution → PendingAttentionRequired → PendingRetryBackoff → PendingAwaitingScheduling → ConfiguredAndCurrent → ConfiguredAutoSyncDisabled → NotConfigured.

**Attempt state derivation:** NeverAttempted → AttentionRequired → Succeeded → RetryScheduled → Deferred → Succeeded.

**Diagnostic codes:** CONFIG_LOAD_FAILED, NOT_CONFIGURED, PENDING_CORRUPT, PENDING_INACCESSIBLE, EXECUTION_LOCK_STALE, EXECUTION_LOCK_MALFORMED, EXECUTION_LOCK_INACCESSIBLE, WORKER_LOCK_STALE, WORKER_LOCK_MALFORMED, WORKER_LOCK_INACCESSIBLE, ATTENTION_REQUIRED, STATUS_CORRUPT.

## Recovery Commands (Phase 04A)

| Command | Purpose | Acquires lock? | Clears pending? |
|---------|---------|---------------|-----------------|
| `snp sync retry` | Immediate sync, bypass backoff | Yes (wait_acquire) | Yes (on success) |
| `snp sync clear-failure` | Reset failure disposition | No | No |
| `snp sync discard-pending` | Remove pending marker | No | Yes (generation-safe) |
| `snp sync repair` | Quarantine corrupt artifacts, remove stale locks | No | No |

**`snp sync retry`:** Validates config, acquires execution lock, reads pending generation, runs `run_sync()`, records outcome. Use when status shows "attention required" or "pending retry".

**`snp sync clear-failure`:** Resets `attention_required=false`, `consecutive_failures=0`, `next_attempt_at_unix_ms=0` in status file. Use after fixing the underlying issue to allow immediate retry.

**`snp sync discard-pending`:** Reads pending generation, prompts for confirmation (unless `--force`), calls `clear_if_generation_matches`. Refuses if generation changed during prompt. Use to abandon sync intent.

**`snp sync repair`:** Inspects status file, execution lock, worker lock, pending txn lock, temp files, and permissions. Without `--apply` lists issues; with `--apply` quarantines and removes stale/corrupt artifacts.
