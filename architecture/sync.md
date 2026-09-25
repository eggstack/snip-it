# Sync Infrastructure (`sync.rs`, `sync_commands.rs`, `sync_failure.rs`)

[← Back to Overview](overview.md)

Bidirectional snippet sync over gRPC (tonic) with client-side
AES-256-GCM encryption. This document covers the sync-client layer only.
Auto-sync scheduling, workers, and pending state live in
[auto_sync.md](auto_sync.md); the wire schema lives in [proto.md](proto.md).

## SyncClient (`src/sync.rs`)

```rust
pub struct SyncClient {
    client: SnippetSyncClient<Channel>,
    settings: SyncSettings,
    limits: Option<SyncRunLimits>,
}
```

- `create(settings)` — manual path, `limits = None`, unbounded except for
  transport timeouts.
- `create_with_limits(settings, limits)` — automatic path, `limits = Some(..)`.
- Every RPC takes `&mut self` because tonic clients borrow mutably per call.

### `retry_grpc_unified!` (single retry policy)

All RPCs share one macro (`src/sync.rs`). It is a macro — not a
closure-based generic helper — so the `self.client.<rpc>()` reborrow
expands inline at each call site (a closure helper fails with
"captured variable cannot escape `FnMut`"). Do not reintroduce one.

Preserved semantics:

- 1 initial + 3 retries = 4 attempts (`DEFAULT_MAX_RETRIES = 3`).
- Backoff: 100 ms initial, 2x progression, 5 s cap, jitter in [0.5, 1.5)
  (`retry_jitter_multiplier`).
- Retryable codes: everything except `InvalidArgument`, `NotFound`,
  `AlreadyExists`, `PermissionDenied`, `Unauthenticated`
  (`SyncRetryConfig::is_retryable_grpc_error`).
- Warning log per retry with `attempt/total` and delay.
- Terminal errors map via `grpc_error_to_snip_error`.

### `RetryBackoff`: Standard vs RateLimitAware

| Variant | Used by | Behaviour |
|---------|---------|-----------|
| `Standard` | Every RPC except `Sync` | `delay * 2`, capped at `max_delay_ms` (5 s) |
| `RateLimitAware` | `sync_with_retry` (`Sync` RPC) only | `ResourceExhausted` → `delay * 4`, capped at 120 s; all other codes → standard progression |

### `SyncRunLimits` / timeouts

```rust
pub(crate) struct SyncRunLimits {
    pub deadline: std::time::Instant,
    pub request_timeout: Duration,
}
```

- `None` limits (manual `snp sync`, register, premade RPCs): unbounded;
  only the transport timeout applies.
- `Some(limits)` (automatic sync): each RPC is wrapped in
  `tokio::time::timeout(remaining)`; backoff sleeps that would overrun the
  deadline are refused. All expirations map to
  `SyncFailureKind::Timeout` (→ `FailureClass::Transient`), including
  `ensure_budget()` before the operation starts.
- Transport: connect timeout defaults to 10 s (`SNP_SYNC_CONNECT_TIMEOUT`
  override); request timeout defaults to 30 s (`SNP_SYNC_REQUEST_TIMEOUT`
  override, clamped as the upper bound for automatic-sync limits).
- Premade RPCs (`list/get/search_premade_libraries`) always run with
  `None` limits (manual-only path, historical unbounded behaviour).

### Client methods

| Method | RPC | Notes |
|--------|-----|-------|
| `sync_encrypted` / `sync_encrypted_with_ceiling` | `Sync` + `PushSnippets` | Entry points; see §4 |
| `push_snippets_batch` | `PushSnippets` | One batch, `Standard` backoff |
| `sync_with_retry` | `Sync` | `RateLimitAware` backoff; `api_key` passed explicitly so the body field stays empty |
| `health_check` | `Health` | `Timeout` maps to `Err`, anything else to `Ok(false)` |
| `register` | `Register` | Associated function, no settings needed |
| `list_libraries` / `create_library` | `ListLibraries` / `CreateLibrary` | Library list paginates (50/page, 10 000-page bound) |
| `list_premade_libraries` / `get_premade_library` / `search_premade_libraries` | premade RPCs | Client-side caps: 10 000 entries, 4 MiB content |
| `detect_device_conflict` | — | Warns when a server snippet carries a foreign non-empty `device_id` |

## `build_upload_batches` (byte-bounded batching)

```rust
pub(crate) fn build_upload_batches(
    encrypted_snippets: Vec<Snippet>,
    library_id: &str,
    last_sync: i64,
    sync_limit: i32,
    byte_ceiling: usize,
) -> SnipResult<Vec<Vec<Snippet>>>
```

- Ceiling: `DEFAULT_CLIENT_REQUEST_CEILING = 3_584 * 1024` (3.5 MiB),
  deliberately below the server 4 MiB gRPC default to leave framing headroom.
- Size is measured with Prost `encoded_len()` on a constructed
  `SyncRequest` — the larger of the two request envelopes — via
  `sync_request_encoded_len` / `snippet_field_encoded_len` (1-byte tag +
  varint length + message). Batches that fit `SyncRequest` also fit
  `PushSnippetsRequest`.
- Snippets are sorted by ID first: deterministic ordering across retries.
- Incremental build: tentatively append, measure, keep or split. After an
  overflow split the new singleton is **immediately re-validated**, so an
  oversized item following a small item is still caught before any remote
  mutation.
- A single oversized item fails with `SyncFailureKind::RequestTooLarge`
  before any batch is sent. The message names the snippet ID and sizes and
  states the local snippet is unchanged.

## Upload strategy (0 / 1 / many)

Owned by `sync_prepared_encrypted_inner`. All uploads are sent **before**
any response page is requested, so `has_more == false` on the first
response can never truncate uploads, and the final response describes
server state after all successful uploads.

| Batches | Transport |
|---------|-----------|
| 0 (pull-only: no locals, or every local failed encryption) | Empty-upload `Sync(offset=0)` fetches the authoritative first page |
| 1 | `Sync(batch, offset=0)` carries the upload **and** returns the first response page in one RPC |
| ≥ 2 | Each batch via `PushSnippets` (upload only), then an empty-upload `Sync(offset=0)` for the authoritative first page |

Further rules:

- `PushSnippets` is idempotent by snippet identity (server upserts `ON
  CONFLICT … WHERE newer`); retrying an accepted batch is safe.
- Multi-batch errors go through `add_batch_context(batch, total)`, which
  prefixes `batch n/m` while **preserving the original
  `SyncFailureKind`** — a `ClockSkew` stays `ClockSkew` /
  `FailureClass::Configuration`, and a `Runtime` (e.g. gRPC `internal` →
  `Internal`) is never re-wrapped as `SyncRequestFailed` (which would
  wrongly retry it as `Transient` forever).
- Pagination (`paginate_remaining`): stops on `!has_more` or an empty page;
  hard bound `MAX_PAGINATION_PAGES = 10_000`; `i32` offset saturation
  surfaces `RequestTooLarge` instead of looping at `i32::MAX`.
- Per-page decrypt failures are counted, not fatal: IDs land in
  `skipped_ids`, and `build_sync_response` marks the aggregate unsuccessful
  only when **all** snippets were skipped on one side.

## Prepared transport seam

- `sync_encrypted` / `sync_encrypted_with_ceiling` → `sync_encrypted_inner`
  (real encryption via `encrypt_snippets`, `key_cache_guard`,
  `ensure_budget`) → `sync_prepared_encrypted_inner` (the single
  zero/one/many transport implementation).
- `sync_encrypted_with_test_encrypt` lives in `#[cfg(test)] mod tests`
  (`src/sync.rs:1466`): it accepts an
  injected encrypt function and drives the same prepared transport (used by
  the all-encryption-failed pull-path regression). Never reachable from
  production.

## Orchestration (`src/sync_commands.rs`)

`run_sync(settings, library, push_only, pull_only, runtime)` delegates to
`run_sync_with_limits(…, limits)`:

1. Resolve direction (`Push` / `Pull` / `Bidirectional`; push-only warns it
   skips downloads). `run_default_sync` = bidirectional, all libraries.
2. `ensure_sync_configured`, connect (`ConnectFailed` on transport error),
   `check_server_health`.
3. Enumerate libraries from the **config index** (never the filesystem, so
   crash-orphaned files are not resurrected); error `NoLibrariesToSync`
   when empty. Unlinked libraries are created + linked on the server first.
4. Replay `check_and_complete_recovery_markers` (startup scan), then sync
   each library: `sync_encrypted(locals, last_sync, library_id)` → on
   `LibraryNotFound`, `handle_library_not_found` (see §8).
5. `merge_and_save` on success; advance `last_sync` to `server_timestamp`
   only when `skipped_count == 0`, so encryption/decryption failures are
   retried next time. Partial per-library failures accumulate in
   `SyncStatus`; the run reports them without aborting sibling libraries.

## Merge (`merge_snippets`)

Live versions order by the deterministic key
`(updated_at, device_id, SHA-256(synced fields))` — never role-dependent
server-wins. `choose_version` compares timestamps first, then `device_id`,
and computes the SHA-256 fingerprint lazily only on a full tie.

Fingerprint inputs (`fingerprint()`): `id`, description, command, tags **in
stored order** (length-prefixed), `created_at`, `updated_at`, `device_id`,
deletion flag. Local-only `output`, `folders`, `favorite` are excluded and
never influence conflict ordering; swapping two inputs on an equal
timestamp yields the same winner.

- **Deletion wins**: a deleted version beats live content even with an
  older timestamp (no resurrection). Both-deleted merges to
  `Equivalent` and the record is omitted from display; tombstones persist
  locally until the server acknowledges them. A server-only tombstone for
  an ID the device never saw is dropped (nothing to preserve).
- Winner `Remote` adopts server description/command/tags/timestamps but
  keeps local `output`/`folders`/`favorite`; winner `Local` keeps the local
  record untouched. One-sided live records are preserved.
- Result is sorted by `updated_at` descending (`sort_by_cached_key` over
  the version key — O(n) hashes, not O(n log n)).
- Clocks are still wall-clock Unix seconds: a fast clock can dominate
  edits until real time catches up. Sync is deliberately not a CRDT (no
  logical/vector clocks).

### Output field contract

`output` is **local-only**: it is not a field of `ProtoSnippet`, is never
uploaded or downloaded, and merge always preserves the local value (server
wins still keep local `output`; new server-only snippets start with empty
`output`). `snp edit --output` requires `--filter`.

## Transport security

- **TLS required.** `create_tls_channel` refuses plaintext gRPC to
  non-loopback hosts. `http://` is allowed only for loopback
  (`localhost`, `127.x.x.x`, `[::1]`, IPv4-mapped IPv6 loopback) **or**
  when `SNIP_SYNC_ALLOW_HTTP` is truthy (`true`/`1`/`yes`/`on`,
  case-insensitive, for local development only). HTTPS uses system native
  roots with hostname verification (`domain_name`) and assumes HTTP/2
  (skips ALPN negotiation).
- **Bearer metadata.** The API key travels as gRPC `authorization:
  Bearer <key>` metadata (`add_api_key_metadata`); body `api_key` fields
  are sent empty for the sync-family RPCs (the proto fields remain only as
  deprecated compatibility surface). API keys are `Zeroizing`-wrapped in
  memory and never logged (see `error.rs`, `utils/redact.rs`).
- **Field/size limits** (server defaults in `snip-sync/src/lib.rs`;
  client enforces the upload ceiling + premade caps):

  | Limit | Value |
  |-------|-------|
  | gRPC max message (server) | 4 MiB |
  | Client upload ceiling | 3.5 MiB |
  | `command` / `description` | 1024 chars |
  | tags / tag length / id / device_id | 50 / 100 / 128 / 128 chars |
  | API key | 512 chars |
  | `Sync` page (`MAX_REQUEST_LIMIT`) | 1000 records |
  | Server sync-set cap | 10 000 snippets |
  | Client premade caps | 10 000 entries, 4 MiB content |

- **Clock-skew diagnostics.** The server validates `created_at` /
  `updated_at` against one `now` sample and rejects outliers with
  `InvalidArgument("CLOCK_SKEW: … N seconds ahead of server time;
  synchronize the client clock and retry")`. The client maps
  `InvalidArgument` with a `CLOCK_SKEW:` prefix to
  `SyncFailureKind::ClockSkew` → `FailureClass::Configuration`.
- **Sanitized server errors.** Unauthenticated callers and internal faults
  surface as generic `Unauthenticated` / `Status::internal("Internal
  error")`; detail is logged server-side only.
- **Rate limiting.** `ResourceExhausted` ("Rate limit exceeded") triggers
  the `RateLimitAware` 4x/120 s backoff on the `Sync` RPC only.

## Remote library recovery (`<library>.sync_recovery`)

When a linked remote library is missing (`LibraryNotFound`), the client
drives an atomic TOML state machine beside the library file
(`libraries/<name>.sync_recovery`, schema 1, `Creating → RemoteCreated →
Linked`), holding only local name/ID, phase, timestamp, and the recovered
server ID — never credentials or snippet content:

1. Write/validate the marker (corrupt or identity-mismatched markers —
   wrong schema, name stem, local ID, or missing library file — are
   preserved and block blind recreation).
2. Reuse the recorded server ID, or list remotes by **normalized** name
   (`lowercase`, spaces → `-`): exactly one match is reused, zero matches
   create one, multiple matches fail visibly as ambiguous. A `Linked`
   marker never creates a remote library.
3. Persist the server ID (`RemoteCreated`) **before** relinking locally;
   relink writes the server ID plus `last_sync = 0` together.
4. Retry the sync from cursor 0, merge + save, advance `last_sync` — only
   then remove the marker. Startup (`check_and_complete_recovery_markers`)
   resumes `Creating` / `RemoteCreated` / `Linked` markers the same way.

## Failures (`src/sync_failure.rs`, `src/error.rs`)

`SyncFailureKind` has 21 variants:

`NotConfigured`, `ConnectFailed`, `HealthCheckFailed`,
`AuthenticationFailed`, `SyncRequestFailed`, `CreateLibraryFailed`,
`GetPremadeLibraryFailed`, `RegistrationFailed`,
`LibraryManagerInitFailed`, `LibraryModeInitFailed`,
`LibrariesDirReadFailed`, `NoLibrariesToSync`,
`SaveMergedLibraryFailed`, `PartialSyncFailure`,
`PremadePartialFailure`, `EncryptionFailed`, `DecryptionFailed`,
`LibraryNotFound`, `Timeout`, `RequestTooLarge`, `ClockSkew`.

`FailureClass::from_error` maps them without string matching:

| `FailureClass` | Members |
|----------------|---------|
| `Transient` | `ConnectFailed`, `HealthCheckFailed`, `SyncRequestFailed`, `GetPremadeLibraryFailed`, `PartialSyncFailure`, `PremadePartialFailure`, `Timeout` |
| `Configuration` | `NotConfigured`, `AuthenticationFailed`, `CreateLibraryFailed`, `RegistrationFailed`, `LibraryNotFound`, `RequestTooLarge`, `ClockSkew` |
| `LocalFailure` | `LibraryManagerInitFailed`, `LibraryModeInitFailed`, `LibrariesDirReadFailed`, `SaveMergedLibraryFailed` |
| `Internal` | `NoLibrariesToSync`, `EncryptionFailed`, `DecryptionFailed` |

Legacy `Runtime` / `Io` / `Toml` errors fall back to substring heuristics
(auth/keychain → `Configuration`, network/timeout/server → `Transient`,
save/read/conflict/merge → `LocalFailure`, else `Internal`).
`FailureClass` lives in the sync-client layer so `sync.rs` classifies
without depending on `auto_sync`; `auto_sync::policy` re-exports it and
adds `RetryDisposition`. Scheduling errors are typed (`ScheduleError::
Pending` vs `Spawn`) — never collapsed into `NoPending` / `SpawnNow` /
success.

## Auto-sync pointer

Detached scheduling, debounce, pending generations, locks, and the worker
loop are documented in [auto_sync.md](auto_sync.md). The only contract
this layer owns: every sync path (manual, explicit `--sync`, cron,
detached worker) holds the shared `SyncExecutionLock` for the whole
operation, and the worker calls `run_sync_with_limits` directly with
`Some(SyncRunLimits)`.
