# Sync

[← Back to Overview](overview.md)

## Overview

The sync subsystem moves snippets between the local library files and a remote
`snip-sync` server over gRPC (Tonic). It owns the client transport, the single
retry/backoff policy, byte-bounded upload batching, paginated download, the
last-write-wins merge, per-library orchestration (server-library creation and
linking, remote-library recovery), and the typed failure classification the
auto-sync scheduler consumes.

It deliberately does **not** cover the wire schema (see
[proto.md](proto.md)), the server implementation (see [server.md](server.md)),
snippet-body encryption (see [encryption.md](encryption.md)), local persistence
and the transaction gate (see [persistence.md](persistence.md)), the detached
auto-sync worker and its scheduling policy (see
[auto_sync.md](auto_sync.md)), or the CLI surface (see [cli.md](cli.md)).

## Module Map

| File | Role |
|------|------|
| `src/sync.rs` | gRPC client, TLS transport, retry macro, batching, download pagination |
| `src/sync_commands.rs` | orchestration: pre-flight, linking, merge, recovery, result counters |
| `src/sync_failure.rs` | `FailureClass` — typed failure classification (sync-client layer) |
| `src/error.rs` | `SnipError`, `SyncFailureKind` (21 variants, `src/error.rs:30-73`) |
| `snip-proto/proto/sync.proto` | wire contract the client speaks |

## Client Construction

```rust
// src/sync.rs:311-315
pub struct SyncClient {
    client: SnippetSyncClient<Channel>,
    settings: SyncSettings,
    limits: Option<SyncRunLimits>,
}

// src/sync.rs:332-356
pub async fn create(settings: SyncSettings) -> SnipResult<Self>
pub(crate) async fn create_with_limits(
    settings: SyncSettings,
    limits: Option<SyncRunLimits>,
) -> SnipResult<Self>
```

`create` delegates to `create_with_limits(settings, None)`. The client is
constructed over a tonic `Channel`; a connect failure is mapped to
`SyncFailureKind::ConnectFailed` (`src/sync.rs:344-349`).

Two guards run before every network call:

- `ensure_budget` (`src/sync.rs:318-329`) — fails with
  `SyncFailureKind::Timeout` when the auto-sync deadline has already elapsed.
  It is a no-op for manual sync (`limits == None`).
- `require_api_key` (`src/sync.rs:800-808`) — fails fast with a typed error
  naming `snp register --force`. `sync.toml` deserializes `api_key` with
  `#[serde(default)]`, so a headerless legacy config parses with an empty key;
  without this guard every RPC would burn the whole retry ladder on a request
  that can only ever come back `Unauthenticated`.

### Authentication

`add_api_key_metadata` (`src/sync.rs:291-299`) puts `Bearer <api_key>` into the
gRPC `authorization` metadata header. The request body's `api_key` field is
left empty on purpose — the key must not travel in the body. An empty key is
logged as an error and the header is omitted rather than panicking.

### Transport Security

`create_tls_channel` (`src/sync.rs:1208-1261`):

- Plaintext `http://` to a non-loopback host is refused unless
  `SNIP_SYNC_ALLOW_HTTP=true` (`src/sync.rs:1216-1222`).
- For `https://`, the client configures `ClientTlsConfig::new()
  .with_enabled_roots().domain_name(host).assume_http2(true)`
  (`src/sync.rs:1246-1257`). `domain_name` pins SNI/certificate verification to
  the URI host; `assume_http2(true)` skips ALPN negotiation so a server without
  ALPN still works instead of surfacing a confusing framing error.
- `SNP_SYNC_CONNECT_TIMEOUT` (default 10 s) and `SNP_SYNC_REQUEST_TIMEOUT`
  (default 30 s) bound the connect and the per-request timeout
  (`src/sync.rs:1230-1244`; constants at `src/sync.rs:38-39`). When
  `SyncRunLimits` supplies a `request_timeout`, the smaller of the two wins.

## Retry and Backoff

### The `retry_grpc_unified!` Macro

`src/sync.rs:194-256` is the **sole** implementation of retry/backoff/deadline
control flow. It replaced the former `retry_grpc!` / `retry_grpc_limited!` pair
and the hand-rolled `push_snippets_batch` / `sync_with_retry` loops.

It is a macro, not a generic async helper, because Tonic clients borrow
`&mut self` per RPC; a closure-based helper would need complex lifetime
machinery for no policy benefit. The macro expands inline, so
`self.client.<rpc>()` reborrows correctly.

Signature:

```rust
retry_grpc_unified!($limits, $op, $name, $backoff)
```

| Aspect | Value | Source |
|--------|-------|--------|
| Total attempts | 1 initial + `max_retries` retries = **4** | `src/sync.rs:35`, `:200-251` |
| Initial delay | 100 ms | `src/sync.rs:36` |
| Delay cap (Standard) | 5 s | `src/sync.rs:37` |
| Jitter multiplier | `0.5 + (combined % 1000) / 1000` → **[0.5, 1.5)** | `src/sync.rs:123-131` |
| Deadline expiry | `SyncFailureKind::Timeout` | `src/sync.rs:206-209`, `:215-218`, `:244-247` |

Attempt count math: `attempt` starts at 0 and the loop breaks when
`attempt >= config.max_retries` (`src/sync.rs:224-226`), so with
`max_retries = 3` the RPC is attempted 4 times.

With `$limits = None` (manual sync, register, premade) the macro runs
unbounded except for the transport request timeout (`src/sync.rs:213`). With
`$limits = Some` (automatic sync) each RPC is wrapped in
`tokio::time::timeout(remaining)`, and a backoff sleep that would overrun the
deadline is refused rather than slept through.

### Retryable gRPC Codes

`SyncRetryConfig::is_retryable_grpc_error` (`src/sync.rs:106-116`) retries
**every** code except:

| Non-retryable code | Why |
|---------------------|-----|
| `InvalidArgument` | deterministic request rejection |
| `NotFound` | library missing — handled by the recovery path |
| `AlreadyExists` | deterministic |
| `PermissionDenied` | authorization |
| `Unauthenticated` | credential problem |
| `OutOfRange` | tonic's "message exceeds the size limit" verdict — a deterministic property of the payload, not a transient condition. Retrying only burns the whole ladder. |

`ResourceExhausted` stays retryable on purpose: the server uses it for rate
limiting, and `RetryBackoff::RateLimitAware` gives it a dedicated backoff.

### `RetryBackoff` Variants

```rust
// src/sync.rs:141-145
enum RetryBackoff {
    Standard,
    RateLimitAware,
}
```

- `Standard` (`src/sync.rs:155`): `delay = (delay * 2).min(max_delay_ms)` —
  100 ms → 200 → 400 → 800 … capped at 5 s.
- `RateLimitAware` (`src/sync.rs:156-162`): on `ResourceExhausted`,
  `delay = (delay * 4).min(120_000)` — 100 ms → 400 → 1 600 → 6 400 … capped
  at 120 s. Any other failure falls back to the Standard progression.

Only the `Sync` RPC uses `RateLimitAware` (`src/sync.rs:889`); every other RPC
(`PushSnippets`, `Health`, library/premade calls) uses `Standard`.

### Terminal Error Mapping

`grpc_error_to_snip_error` (`src/sync.rs:262-283`) maps the final failure:

| gRPC status | Result |
|-------------|--------|
| `NotFound` | `SyncFailureKind::LibraryNotFound` |
| `DeadlineExceeded` | `SyncFailureKind::Timeout` |
| `InvalidArgument` with a `CLOCK_SKEW:` prefix | `SyncFailureKind::ClockSkew` |
| `InvalidArgument` (other) | `SnipError::Runtime` |
| anything else | `SnipError::Runtime` |

### Run Limits

```rust
// src/sync.rs:64-74
pub(crate) struct SyncRunLimits {
    pub deadline: std::time::Instant,
    pub request_timeout: Duration,
}
impl SyncRunLimits {
    pub(crate) fn remaining(self) -> Option<Duration>
}
```

`remaining()` returns `None` once the deadline has passed (checked with
`checked_duration_since`), which the macro treats as "expired" rather than
"unbounded".

## Upload Path

### Byte-Bounded Batching

```rust
// src/sync.rs:1408-1414
pub(crate) fn build_upload_batches(
    mut encrypted_snippets: Vec<crate::proto::Snippet>,
    library_id: &str,
    last_sync: i64,
    sync_limit: i32,
    byte_ceiling: usize,
) -> SnipResult<Vec<Vec<crate::proto::Snippet>>>
```

The client ceiling is **3.5 MiB** (`DEFAULT_CLIENT_REQUEST_CEILING =
3_584 * 1024`, `src/sync.rs:46`), deliberately below the server's 4 MiB gRPC
limit (`snip-sync/src/lib.rs:104`).

Accounting is by **Prost encoded length**, not by item count:

- `sync_request_base_encoded_len` (`src/sync.rs:1386-1388`) measures the
  envelope once — `SyncRequest::encoded_len()` with an empty snippet list
  (`src/sync.rs:1360-1375`).
- `snippet_field_encoded_len` (`src/sync.rs:1390-1395`) adds
  `1 + varint_len + snippet.encoded_len()` per item, because `local_snippets`
  is a repeated message field (tag + length varint + payload).

Batches are built incrementally: tentatively append one snippet, measure, and
either keep it or finalize the batch. The snippet list is sorted by id first so
retry ordering is deterministic (`src/sync.rs:1416`). If a **single** snippet
exceeds the ceiling, the function returns `SyncFailureKind::RequestTooLarge`
*before any batch is sent* (`src/sync.rs:1429-1441`). After an overflow split
the new singleton is re-validated immediately, so an oversized item that
follows a small one is caught before any remote mutation
(`src/sync.rs:1463-1474`).

### Upload Strategy — Zero / One / Many Batches

`sync_prepared_encrypted_inner` (`src/sync.rs:441-621`) owns all three cases:

| Batches | Transport | Rationale |
|---------|-----------|-----------|
| 0 | empty-upload `Sync(offset = 0)` | pull-only path — nothing local to upload |
| 1 | `Sync(batch, offset = 0)` | upload and first response page in one RPC |
| ≥ 2 | each batch via `PushSnippets` (upload only), then empty-upload `Sync(offset = 0)` | the authoritative first page must describe server state *after* all uploads |

All upload batches are sent before any response page is requested, so
`has_more == false` on the first response can never truncate uploads
(`src/sync.rs:365-376`).

### Per-Record Refusals Do Not Abort the Batch

`push_snippets_batch` (`src/sync.rs:815-861`) returns the number of records the
server refused:

- `!inner.success && rejected == 0` → whole-request failure (auth, unknown
  library, transport refusal) → `SyncFailureKind::SyncRequestFailed`, which
  aborts the run.
- `rejected_count > 0` → individual records were refused (validation failure,
  cross-scope id collision) while the rest of the batch was stored. This is
  **not** an error: it returns `Ok(rejected)` and the run continues. Previously
  one bad record turned into a permanent whole-library failure that also
  blocked every remaining batch.

`PushSnippetsResponse` carries counts but no per-record ids, so those refusals
fold into `skipped_count` only (`src/sync.rs:772-775`).

### Skipped-Id Accounting

`accumulate_page` (`src/sync.rs:736-759`) extends the client's own skipped-id
list with the server's `response.skipped_ids` before decrypting the page. The
server reports rows it refused to store (validation failure, upsert failure, or
an id already held under a different user or library); discarding them made
`skipped_count` dishonest.

`build_sync_response` (`src/sync.rs:762-790`) aggregates:

```
total_skipped = all_skipped_ids.len() + push_rejected_count
```

`success` is false only when *every* record was skipped
(`all_skipped_local || all_skipped_server`), i.e. a total failure; a partial
skip keeps `success == true` and is surfaced through `skipped_count`.

## Download Path

### Client Pagination

`paginate_remaining` (`src/sync.rs:628-733`) fetches the remaining pages after
the first response page. It stops when `has_more` is false or a page comes back
empty. A hard bound of `MAX_PAGINATION_PAGES = 10_000`
(`src/sync.rs:53`) aborts a buggy or malicious server that answers
`has_more = true` forever.

The offset advances by the number of rows actually returned
(`src/sync.rs:541`, `:599`), not by the requested page size.

### Server-Side Page Trimming

Downloads are paged by **row count**, but tonic hard-fails an over-size
encoding with `OUT_OF_RANGE` instead of truncating. Without a byte bound, a
library whose rows happen to be large could be pushed successfully and then
**never downloaded again**, by any client, for any page size.

The server therefore trims each response page to the encoded byte budget:

- `page_byte_ceiling` (`snip-sync/src/lib.rs:117-121`) =
  `grpc_ceiling − RESPONSE_ENVELOPE_RESERVE_BYTES (64 KiB) − envelope_bytes`.
- `fit_page_to_byte_ceiling` (`snip-sync/src/lib.rs:131-156`) drops trailing
  rows until the page fits, **always keeping at least one row** (returning an
  empty page would stall the client's pagination loop rather than make
  progress), and reports whether anything was dropped.
- `has_more` is then `has_more_by_rows || truncated`
  (`snip-sync/src/lib.rs:1536`), and the timestamp watermark is withheld
  whenever rows remain (`snip-sync/src/lib.rs:1538-1541`).

## Merge and Conflict Resolution

### Synced vs Local-Only Fields

| Field | Synced | Notes |
|-------|--------|-------|
| `id` | yes | deterministic `legacy-<sha256>` id, see [IDENTITY_CONTRACT](../docs/IDENTITY_CONTRACT.md) |
| `description` | yes | encrypted in transit |
| `command` | yes | encrypted in transit |
| `tags` | yes | encrypted in transit |
| `created_at` | yes | earliest of the two copies wins |
| `updated_at` | yes | merge clock |
| `device_id` | yes | tie-breaker |
| `deleted` | yes | tombstone flag |
| `output` | **no** | local-only; not in `ProtoSnippet` at all |
| `folders` | **no** | local-only; preserved from the local copy on merge |
| `favorite` | **no** | local-only; preserved from the local copy on merge |

`ProtoSnippet` is produced from a local `Snippet` by
`impl From<&Snippet> for ProtoSnippet` (`src/sync_commands.rs:423-437`); the
conversion drops `output`, `folders`, and `favorite` outright.

### Fingerprint

```rust
// src/sync_commands.rs:1140-1163
fn fingerprint(
    id: &str, description: &str, command: &str, tags: &[String],
    created_at: i64, updated_at: i64, device_id: &str, deleted: bool,
) -> [u8; 32]
```

SHA-256 over length-prefixed fields (`hash_field`, `src/sync_commands.rs:1135-1138`)
plus `created_at`, `updated_at`, and the `deleted` flag. **`output`, `folders`,
and `favorite` are not part of the fingerprint** — they are local-only and must
not influence conflict resolution.

### `choose_version` Ordering

```rust
// src/sync_commands.rs:1121-1126
struct VersionKey {
    updated_at: i64,
    device_id: String,
    fingerprint: [u8; 32],
}

// src/sync_commands.rs:1199-1226
fn choose_version(local: &Snippet, remote: &ProtoSnippet) -> VersionWinner
```

`VersionWinner` is `Local | Remote | Equivalent`
(`src/sync_commands.rs:1128-1133`). The decision order is:

1. **Deletion wins outright.** `(local.deleted, remote.deleted)`:
   - `(true, false)` → `Local` (a local deletion is never resurrected)
   - `(false, true)` → `Remote` (a remote deletion removes the local record)
   - `(true, true)` → `Equivalent`
   The tombstone arms deliberately do **not** compare timestamps.
2. **Timestamp first.** `(false, false)`: larger `updated_at` wins.
3. **Device id.** On a timestamp tie, larger `device_id` wins.
4. **Fingerprint.** On a full tie, the larger `VersionKey` wins (the fingerprint
   is computed lazily, only at this point).

The remote-tombstone arm of `merge_snippets` uses
`updated_at: local.updated_at.max(server.updated_at)`
(`src/sync_commands.rs:1281`), matching the local-tombstone arm
(`src/sync_commands.rs:1307`). Taking the server timestamp verbatim would
lower the local timestamp whenever the tombstone is older, regressing the
record below a watermark other clients already synced past. No resurrection is
implied — the record is still marked deleted.

### Tombstone Handling in Both Directions

`merge_snippets` (`src/sync_commands.rs:1252-1376`) has four arms per id:

| Situation | Result |
|-----------|--------|
| Remote tombstone wins over a live local | record kept as a local tombstone, `updated_at = max(local, remote)`, local `output`/`folders`/`favorite` preserved (`src/sync_commands.rs:1264-1284`) |
| Remote live copy wins over a live local | remote fields win; local `output`/`folders`/`favorite` preserved; `created_at = min(local, remote)` (`src/sync_commands.rs:1285-1297`) |
| Local tombstone wins over a live remote | local tombstone retained, `updated_at = max(local, remote)` (`src/sync_commands.rs:1298-1310`) |
| Both tombstones | `Equivalent if local_snip.deleted` → the record is retired (`src/sync_commands.rs:1312`) |

**Server-only tombstones are dropped.** A tombstone for an id the local device
never observed carries no local deletion to preserve, so it is discarded
(`src/sync_commands.rs:1316-1322`) — the no-resurrection policy for deletions
that were never synced to this device.

**Local tombstones absent from the server response are retained.** The server
response is a *delta* (`updated_at >= since`), so a tombstone is missing
whenever the server row sits below the `last_sync` watermark — guaranteed in
pull-only mode, where nothing is uploaded at all. Dropping it erased the
record of the intent: the deletion could never be propagated, and the server's
live copy would resurrect on the next sync that did cross the watermark. The
tombstone is retained so it is re-uploaded until the server acknowledges it, at
which point the id appears in `seen_ids` and the `Equivalent if
local_snip.deleted` arm retires the record
(`src/sync_commands.rs:1339-1358`).

### Conflict Counting

Conflict counts come from the merge itself, not from the presence of a foreign
`device_id`:

```rust
// src/sync_commands.rs:1235-1250
fn overwritten_by_remote(local: &Snippets, server_snippets: &[ProtoSnippet]) -> Vec<String>
```

It returns the ids whose **server copy won** — i.e. `choose_version` returned
`VersionWinner::Remote`. A stale or byte-identical remote copy leaves the local
version in place and is not a conflict. A remote tombstone over a live local
record *does* resolve to `Remote`, so it is counted.

The previous helper, `sync::detect_device_conflict`, has been **removed**; no
public or test-only entry point remains. `docs/PUBLIC_API.md` still lists it
and is out of date.

## Orchestration — `src/sync_commands.rs`

```rust
// src/sync_commands.rs:650-665
pub fn run_sync(
    sync_settings: &SyncSettings,
    library_name: Option<&str>,
    push_only: bool,
    pull_only: bool,
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()>

// src/sync_commands.rs:667-674
pub(crate) fn run_sync_with_limits(
    sync_settings: &SyncSettings,
    library_name: Option<&str>,
    push_only: bool,
    pull_only: bool,
    runtime: &tokio::runtime::Runtime,
    limits: Option<sync::SyncRunLimits>,
) -> SnipResult<()>
```

`run_sync` is `run_sync_with_limits(..., None)`. The direction is derived from
the CLI flags: `push_only` → `Push`, `pull_only` → `Pull`, otherwise
`Bidirectional` (`src/sync_commands.rs:676-682`). Push-only mode logs a warning
that remote changes from other devices will not be downloaded
(`src/sync_commands.rs:684-689`).

The run is: validate configuration → connect → health check → build the
library manager → resolve the library list → server pre-flight → recovery
marker scan → per-library upload/pull/merge → aggregate counters.

### Configuration Gate

`ensure_sync_configured` (`src/sync_commands.rs:469-479`) requires both
`enabled` and a non-empty `api_key`; otherwise the run fails with
`SyncFailureKind::NotConfigured` (`src/sync_commands.rs:691-696`).

### Server Pre-Flight

`src/sync_commands.rs:749-814`. For every library with no server linkage, the
client creates the server library (name normalized to lowercase with spaces
replaced by `-`, `src/sync_commands.rs:767`) and links it locally.

Pre-flight failures are **recorded as failed results**, not silently skipped:

| Pre-flight failure | Result |
|--------------------|--------|
| `create_library` failed | `preflight_failures` → `status.failed += 1` (`src/sync_commands.rs:793-800`, `:811-814`) |
| `link_server_library` failed | `preflight_failures` → `status.failed += 1` (`src/sync_commands.rs:779-785`) |
| library file missing | recorded in the per-library loop (`src/sync_commands.rs:855-864`) |
| library not linked to the server | recorded in the per-library loop (`src/sync_commands.rs:868-877`) |
| fail-closed `load_library` | recorded in the per-library loop (`src/sync_commands.rs:879-887`) |

A silent `continue` used to leave `status.failed` at zero, exit 0, clear the
pending generation, and never upload or retry the library. The per-library
loop skips libraries already present in `results` so a library is not counted
twice (`src/sync_commands.rs:844-847`).

### Recovery Marker Scan

`check_and_complete_recovery_markers` (`src/sync_commands.rs:356-421`) scans
the libraries directory for `<library>.sync_recovery` markers
(`recovery_marker_path`, `src/sync_commands.rs:50-52`) and replays recovery for
each. A marker whose local library is missing is **preserved**, not deleted
(`src/sync_commands.rs:391-394`); a corrupt marker is preserved and logged
(`src/sync_commands.rs:414-417`).

Libraries finished by the recovery path still increment the progress counter
so the final line reaches its own denominator (`src/sync_commands.rs:832-840`).

### Remote-Library Recovery

`handle_library_not_found` (`src/sync_commands.rs:79-354`) runs when the server
answers `LibraryNotFound`:

1. Locate or create the `<library>.sync_recovery` marker
   (`SyncRecoveryMarker`, `src/sync_commands.rs:24-33`; phases
   `Creating | RemoteCreated | Linked`, `src/sync_commands.rs:36-48`).
2. **Verify identity.** The marker's schema, normalized local library name,
   file stem, and local library id must all match the current local library
   (`src/sync_commands.rs:146-164`). A mismatch fails closed rather than
   recreating a remote library for the wrong local one.
3. **Match the remote library.** If the marker has no server id, list the
   remote libraries and match by normalized name. Exactly one match is
   accepted; **ambiguity fails closed** (`src/sync_commands.rs:234-261`); zero
   matches create a new remote library.
4. **Relink and persist.** `relink_server_library` updates the local linkage
   and the marker advances to `Linked` (`src/sync_commands.rs:273-287`).
5. **Retry the sync** against the new server id (`src/sync_commands.rs:290-293`).
6. **Gate the watermark and the marker removal on `skipped_count == 0`.**
   `success` does not imply a clean upload: the server reports refused rows in
   `skipped_ids` without flipping it, and a freshly re-created library returns
   no server rows at all. Advancing `last_sync` or removing the marker there
   would leave no durable trace that records were dropped, so they would never
   be retried (`src/sync_commands.rs:299-319`).

A failed `update_last_sync` during recovery is itself recorded as a failed
result (`src/sync_commands.rs:310-312`).

### Merge and Save

```rust
// src/sync_commands.rs:610-615
fn merge_and_save(
    lib_path: &std::path::Path,
    lib_name: &str,
    snippets: &Snippets,
    server_snippets: &[ProtoSnippet],
) -> SnipResult<(Snippets, Option<String>, Vec<String>)>
```

The conflict list is computed first (`overwritten_by_remote`), then
`merge_snippets` runs, then `library::save_library` writes the result.
`save_library` uses an atomic rename, so the original file is always safe on
failure — no explicit backup/restore is needed here. The returned backup slot
is always `None` (`src/sync_commands.rs:633-642`).

The merge is sorted by `(Reverse(updated_at), Reverse(local_version_key))` with
`sort_by_cached_key`, which computes each snippet's version key once (O(n)
hashes) instead of re-running the full SHA-256 fingerprint on every comparison
(O(n log n) hashes per sync) (`src/sync_commands.rs:1360-1370`).

## Result and Counter Semantics

```rust
// src/sync_commands.rs:586-591
struct SyncStatus {
    pushed: u32,
    pulled: u32,
    conflicts: u32,
    failed: u32,
}
```

| Counter | Meaning | Source |
|---------|---------|--------|
| `pushed` | libraries whose upload completed (Push mode) | `src/sync_commands.rs:916` |
| `pushed` | **also** incremented in Pull mode for every successfully merged library, even though nothing was uploaded — a log-label quirk; the counter is only consumed by the `tracing::info!("Sync complete")` line (`src/sync_commands.rs:1030-1032`, `:1103-1109`) | |
| `pulled` | server snippets merged into the local library | `src/sync_commands.rs:603-607`, `:945`, `:1033` |
| `conflicts` | ids whose **server copy won** the merge | `src/sync_commands.rs:949-955`, `:1037-1043` |
| `failed` | libraries that failed, were skipped, or dropped records | see below |

`failed` is incremented for:

- every pre-flight failure (`:812`)
- a missing library file (`:857`)
- an unlinked library (`:870`)
- a fail-closed `load_library` (`:883`)
- any recovery-path failure (`:98`, `:112`, `:131`, `:142`, `:161`, `:173`, `:183`, `:191`, `:203`, `:228`, `:240`, `:257`, `:268`, `:276`, `:284`, `:310`, `:322`, `:340`, `:346`, `:350`)
- a `sync_encrypted` transport error (`:1006`, `:1089`)
- a `response.success == false` response (`:983`, `:1066`)
- a `merge_and_save` error (`:977`, `:1061`)
- **`skipped_count > 0`** (`:922` Push mode, `:968` Bidirectional, `:1052` Pull mode)

### The `skipped_count` Rule

`has_failures = response.skipped_count > 0`.

- **Push mode**: the library counts as `pushed` (the upload reached the server
  either way) *and* as `failed` when records were skipped; the per-library
  result is `false` (`src/sync_commands.rs:910-932`).
- **Bidirectional mode**: the library counts as `failed` and the per-library
  result is `false` (`src/sync_commands.rs:957-974`). `conflicts` is **not**
  incremented for skips — a skipped record is an encryption failure, a
  server-refused row, or a decryption failure, not two devices racing.
- **Pull mode**: same as bidirectional for the failure accounting
  (`src/sync_commands.rs:1045-1058`).

`last_sync` is only advanced when `!has_failures`
(`src/sync_commands.rs:917-920`, `:939-943`, `:1025-1029`), so skipped records
are retried on the next sync.

A run with `status.failed > 0` returns
`SyncFailureKind::PartialSyncFailure` (`src/sync_commands.rs:1111-1118`).

## Failure Classification

`FailureClass` lives in the sync-client layer so the gRPC client can classify
its own errors without depending upward on `crate::auto_sync`; the
application layer re-exports it and layers scheduling policy on top
(`src/sync_failure.rs:1-11`).

```rust
// src/sync_failure.rs:24-33
pub enum FailureClass {
    Transient,
    Configuration,
    LocalFailure,
    Internal,
}
```

`FailureClass::from_error` (`src/sync_failure.rs:36-119`) maps every
`SyncFailureKind` (21 variants, `src/error.rs:30-73`) onto exactly one class:

| `SyncFailureKind` | `FailureClass` |
|-------------------|----------------|
| `NotConfigured` | Configuration |
| `AuthenticationFailed` | Configuration |
| `CreateLibraryFailed` | Configuration |
| `RegistrationFailed` | Configuration |
| `LibraryNotFound` | Configuration |
| `RequestTooLarge` | Configuration |
| `ClockSkew` | Configuration |
| `ConnectFailed` | Transient |
| `HealthCheckFailed` | Transient |
| `SyncRequestFailed` | Transient |
| `GetPremadeLibraryFailed` | Transient |
| `PartialSyncFailure` | Transient |
| `PremadePartialFailure` | Transient |
| `Timeout` | Transient |
| `LibraryManagerInitFailed` | LocalFailure |
| `LibraryModeInitFailed` | LocalFailure |
| `LibrariesDirReadFailed` | LocalFailure |
| `SaveMergedLibraryFailed` | LocalFailure |
| `EncryptionFailed` | Internal |
| `DecryptionFailed` | Internal |
| `NoLibrariesToSync` | Internal |

`SnipError::Runtime` is classified by substring matching on the message
(`src/sync_failure.rs:61-104`) — configuration/auth keywords → Configuration,
network/timeout keywords → Transient, persistence keywords → LocalFailure,
otherwise Internal. `SnipError::Io` → Transient for connection-ish operations,
otherwise LocalFailure. `SnipError::Toml` → LocalFailure; everything else →
Internal.

Only `Transient` and `Internal` allow automatic retry
(`src/sync_failure.rs:147-149`); `Configuration` is *deferred* — it waits for
the user to fix something (`src/sync_failure.rs:151-153`).

`add_batch_context` (`src/sync.rs:1492-1510`) adds a `batch N/M` prefix while
**preserving the original classification**. Re-wrapping every kind as
`SyncRequestFailed` would classify persistent server-side faults (e.g. gRPC
`internal`, which maps to `Runtime` → `Internal`) as `Transient` and retry them
forever instead of escalating.

## Field and Size Limits

| Limit | Value | Source |
|-------|-------|--------|
| Client request byte ceiling | 3.5 MiB (`3_584 * 1024`) | `src/sync.rs:46` |
| Server gRPC message ceiling | 4 MiB | `snip-sync/src/lib.rs:104` |
| Server max request page size | 1 000 rows | `snip-sync/src/lib.rs:108` |
| Server max encrypted payload | 1 MiB | `snip-sync/src/lib.rs:93`, enforced at `:880-885` |
| Response envelope reserve | 64 KiB | `snip-sync/src/lib.rs:112` |
| Client pagination page bound | 10 000 pages | `src/sync.rs:53` |
| Client default page size | `sync_limit_value()` → 1 000 | `src/config/sync_settings.rs:196-198` |

The encrypted-payload cap exists so a single record can always be transported:
anything larger than the gRPC message ceiling could be stored but never
downloaded again.

## Key Files

- `src/sync.rs` — client, TLS channel, retry macro, batching, pagination.
- `src/sync_commands.rs` — orchestration, merge, recovery, counters.
- `src/sync_failure.rs` — `FailureClass` and its mapping.
- `src/error.rs` — `SnipError`, `SyncFailureKind`.
- `src/encryption.rs` — the crypto the client applies before upload.
- `snip-proto/proto/sync.proto` — the wire contract.
- `snip-sync/src/lib.rs` — server-side limits and page trimming.

## Invariants

- **Deletion always wins.** No code path resurrects a deleted snippet, and a
  tombstone survives until the server acknowledges it.
- **Conflict order is fixed**: `(updated_at, device_id, SHA-256 fingerprint)`,
  with the fingerprint excluding `output`, `folders`, and `favorite`.
- **A skipped record is a failure, not a conflict.** `skipped_count > 0`
  increments `failed`, never `conflicts`, and blocks the `last_sync` advance.
- **The watermark only advances on a clean run.** `last_sync` moves only when
  `skipped_count == 0` and the library's merge and save succeeded.
- **Never trust `response.success` alone** — the server reports refused rows
  in `skipped_ids` without flipping `success`.
- **The retry macro is the only retry policy.** Do not reintroduce a
  closure-based generic retry helper; Tonic's `&mut self` borrow makes it fail
  to compile.
- **`OutOfRange` is terminal.** It means the payload is too large, and
  retrying only burns the backoff ladder.
- **The client ceiling must stay below the server ceiling** (3.5 MiB vs 4 MiB),
  or a request the server accepts cannot be encoded by the client.

## Gotchas

- **`detect_device_conflict` is gone.** Conflict counts come from
  `overwritten_by_remote`, i.e. from `choose_version` returning `Remote`.
  Counting rows merely *touched* by another device produced phantom
  overwrites on every sync.
- **The server response is a delta, not a snapshot.** Any code that treats a
  missing id as "does not exist" is wrong — that is exactly the bug the
  tombstone-retention rule fixes.
- **`skipped_ids` arrive without ids on the push path.**
  `PushSnippetsResponse` only carries counts, so push refusals contribute to
  `skipped_count` but not to `skipped_ids`.
- **A remote tombstone still counts as a conflict.** `choose_version` returns
  `Remote` for `(live, deleted)` without comparing timestamps, so the id lands
  in `conflicts` even though the merge only retires the record.
- **`push_snippets_batch` returns `Ok(rejected)`, not `Err`.** Only a
  whole-request refusal (`!success && rejected == 0`) aborts the run.
- **`SyncRunLimits` is `Copy` and time-relative.** `remaining()` returning
  `None` means *expired*, not *unbounded*.
- **`create_tls_channel` pins `domain_name`** to the URI host. Removing it
  weakens certificate verification and breaks virtual-hosted setups.
