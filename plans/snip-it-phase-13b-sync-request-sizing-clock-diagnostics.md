# Phase 13B — Bounded Sync Uploads and Clock-Skew Diagnostics

Status: READY FOR IMPLEMENTATION

Roadmap: `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`

Baseline: `b62d0f50078f7656eca3c9abf58e2ad290562029`

## 1. Objective

Make every locally valid snippet collection synchronizable within the existing gRPC transport limits, or fail before remote mutation with a precise single-item diagnostic.

The current local exact-input path accepts command bodies up to 16 MiB. The server defaults to a 4 MiB gRPC message limit, and the client places the complete local upload in the first sync request. This produces two failures:

- one valid local encrypted snippet may be larger than one accepted gRPC request;
- a large library of individually small snippets may exceed the request limit because upload is not batched.

This phase adds bounded client upload batching using the existing protocol wherever possible. It does not redesign synchronization, add streaming RPCs, or lower the local feature limit merely to match the transport default.

The phase also makes server clock-skew rejection diagnostically useful. It does not replace last-write-wins ordering or introduce logical clocks.

## 2. Scope

### In scope

- measuring encoded request size using Prost before transmission;
- splitting encrypted local snippets into bounded request batches;
- preserving current response pagination and merge behavior;
- ensuring batches are not duplicated or omitted across retry and pagination loops;
- clear handling of one encrypted item that cannot fit into one request;
- configurable/shared client-side safety margin below the server message ceiling;
- a direct clock-skew error message for timestamps outside the accepted window;
- focused multi-batch, retry, oversized-item, and clock-skew tests;
- narrow updates to sync architecture/user documentation.

### Out of scope

- adding client-streaming or bidirectional-streaming RPCs;
- changing protobuf field numbers or introducing a new protocol version;
- replacing gRPC/tonic;
- adding payload compression unless already supported and demonstrably required after batching;
- changing encryption algorithms, Argon2 parameters, or payload format;
- reducing the 16 MiB local exact-input limit;
- syncing local-only fields such as output, folders, favorites, or usage;
- redesigning conflict resolution or deletion semantics;
- CRDTs, vector clocks, server revisions, or clock synchronization services;
- automatically modifying the host clock.

## 3. Required protocol-compatible design

### 3.1 Preserve existing RPCs

Prefer implementing batching through the existing `Sync` request contract:

- each request carries one bounded subset of local snippets;
- only the first or each explicit upload batch carries local data as required by server semantics;
- server response pages continue to be fetched until complete;
- the final returned `SyncResponse` remains a single aggregated client result.

Before implementation, inspect the server `sync` handler to determine whether repeated requests with the same `last_sync_timestamp`, `library_id`, and different snippet subsets are idempotent and whether each request independently applies its local batch. Document this contract in code comments and tests.

If the current `Sync` handler cannot safely distinguish upload batches from download pagination, make the smallest additive protocol-compatible change. Acceptable bounded options are:

- reuse the existing offset only if server semantics can unambiguously separate upload from download pagination;
- add an optional backward-compatible request field such as `upload_complete` or `batch_index` only if strictly required;
- use the existing `PushSnippets` RPC for bounded upload followed by one download-only `Sync` request if that path preserves the same conflict and timestamp semantics.

Preferred path: use existing `PushSnippets` or existing idempotent `Sync` calls rather than changing the schema. Do not add a new RPC unless all existing paths are provably unsuitable and the roadmap is amended.

### 3.2 Byte-based batching

Do not batch only by snippet count. Encrypted commands vary greatly in size.

Use Prost encoded length or actual encoding:

```rust
use prost::Message;
let bytes = request.encoded_len();
```

Build batches incrementally:

1. start with fixed request metadata and an empty snippet list;
2. tentatively append one encrypted snippet;
3. calculate encoded request size;
4. if under the client maximum, keep it;
5. if over and the batch is nonempty, finalize the prior batch and start a new one;
6. if over with an otherwise empty batch, return a typed oversized-item error before sending any batch.

Do not estimate solely from plaintext length or base64 expansion.

### 3.3 Client request ceiling

The client must know a conservative maximum. Use one of these bounded approaches, in priority order:

1. reuse an existing sync setting if the message ceiling is already configurable;
2. add an internal constant matching the server default with a conservative framing margin;
3. add an advanced config value only if users can already configure the server ceiling and client/server mismatch would otherwise remain common.

Do not implement capability negotiation or a new discovery endpoint.

A reasonable internal target is below 4 MiB, for example 3.5–3.75 MiB, but the implementation must derive and document the exact value. Leave headroom for metadata and transport framing.

### 3.4 Remote mutation ordering

A multi-batch operation must not falsely report complete success after only some batches are accepted.

Required semantics:

- batches are sent in deterministic local order, preferably stable snippet ID order;
- each batch uses existing retry behavior;
- failure stops subsequent batches;
- local files are not rolled back;
- pending auto-sync intent remains on failure;
- the client returns a sync failure naming the failed batch position and total without exposing snippet plaintext;
- a later retry may safely resend prior batches because server upsert behavior is idempotent by snippet identity;
- the final sync cursor is updated only after all upload batches and all required response pages complete successfully.

Do not add a client-side batch journal. Existing pending sync intent is sufficient for retry.

### 3.5 Response aggregation

Preserve:

- server snippet decryption;
- skipped ID reporting;
- total skipped count;
- server timestamp/cursor semantics;
- pagination until `has_more` is false;
- current failure behavior when all local encryption or all server decryption fails.

Avoid mixing upload-batch position with response-page offset in one mutable loop unless invariants are explicit and tested. Prefer two clear stages:

```text
encrypt and form upload batches
send all upload batches
fetch/aggregate remote pages
merge once
```

If the existing RPC necessarily combines upload and first response page, isolate that special first call and then continue download-only pagination.

## 4. Typed errors and user diagnostics

Add or reuse a sync failure kind for request sizing. Required messages:

### Single item too large

Include:

- snippet ID;
- encoded size;
- configured client request ceiling;
- a statement that the local snippet remains unchanged;
- a corrective action such as raising both server/client message limits or reducing/splitting the snippet.

Do not print command text, description, tags, API key, or ciphertext.

### Batch failure

Include:

- batch number and total;
- underlying connection/status category;
- statement that sync remains pending/retryable where applicable.

### Clock skew

Server validation should return a specific message such as:

```text
updated_at is 742 seconds ahead of server time; synchronize the client clock and retry
```

The exact timestamp need not be printed. Apply similarly to `created_at`.

Client-side mapping should preserve this detail for `InvalidArgument` instead of reducing it to a generic operation error.

## 5. Clock-skew boundary

Retain the current bounded future-timestamp policy unless tests or documentation show another configured value. This phase only improves truthfulness and diagnosis.

Required checks:

- timestamp within tolerance succeeds;
- timestamp just outside tolerance returns a skew-specific invalid argument;
- negative timestamps and `created_at > updated_at` retain distinct diagnostics;
- server time is sampled consistently within one validation operation to avoid micro-boundary disagreement;
- no automatic timestamp rewriting is introduced.

A future logical-clock design is explicitly out of scope.

## 6. Likely files

Primary client:

- `src/sync.rs`
- `src/sync_commands.rs`
- `src/error.rs`
- possibly `src/config.rs` only if a bounded request limit setting is required

Protocol/server inspection or narrow changes:

- `snip-proto/proto/snip_sync.proto` only if an additive field is unavoidable
- `snip-sync/src/lib.rs`
- `snip-sync/src/db.rs` only if idempotent batch semantics require a narrowly scoped transaction correction

Tests:

- existing `tests/sync_integration.rs`
- existing `tests/sync_contracts.rs`
- one focused target such as `tests/sync_request_sizing.rs` only if the existing files would become unwieldy

Documentation:

- `architecture/sync.md`
- `USER_GUIDE.md` or README troubleshooting only where the new error needs explanation
- `AGENTS.md`

Do not modify auto-sync scheduling structure, transaction journaling, CI topology, or server lifetime code in this phase.

## 7. Implementation workstreams

### Workstream A — Prove current server idempotence

1. Read the `Sync` and `PushSnippets` handlers and database upsert behavior.
2. Record whether resending the same snippet ID/content is idempotent.
3. Add one focused server/client test proving retrying an already accepted batch does not duplicate data or corrupt timestamps.
4. Choose the smallest existing RPC sequence that preserves semantics.

### Workstream B — Add request-size builder

1. Extract encrypted snippet preparation from the combined loop.
2. Implement a pure batching helper that accepts fixed request metadata, snippets, and a byte ceiling.
3. Use Prost encoded length.
4. Return typed metadata about an oversized single item.
5. Unit test exact boundary behavior without network activity.

### Workstream C — Send batches safely

1. Send batches in deterministic order.
2. Reuse canonical retry and deadline handling.
3. Stop after the first failed batch.
4. Preserve pending intent and do not update final cursor.
5. Ensure automatic-sync deadline applies across the entire batch sequence, not independently reset per batch.

### Workstream D — Aggregate remote response

1. Separate first combined call from subsequent download pages if needed.
2. Aggregate snippets and skipped IDs once.
3. Avoid uploading local snippets on response pagination requests.
4. Update cursor only after complete success.
5. Preserve existing merge ordering/deletion behavior.

### Workstream E — Improve clock diagnostics

1. Calculate server-now once per validation call.
2. Return skew magnitude and corrective guidance.
3. Preserve distinct validation messages for other timestamp failures.
4. Verify client error mapping exposes the server message safely.

### Workstream F — Documentation and closure

1. Document the bounded upload behavior and single-item limit.
2. Explain that local save remains successful when sync is pending or fails.
3. Add clock-skew troubleshooting guidance.
4. Record measurements, implementation SHA, and focused test results in this plan.

## 8. Focused test matrix

### Pure batching tests

- empty snippet list;
- one small item;
- several items that fit one request;
- exact boundary fit;
- one-byte-over boundary starts a new batch;
- one item larger than the ceiling returns an error before any send;
- stable ordering across repeated runs;
- metadata overhead included in encoded size;
- no batch exceeds ceiling.

### Integration tests

- encrypted library requiring at least three upload batches round-trips exactly;
- first batch retry is idempotent;
- middle batch failure leaves remote state partial but retry completes without duplication;
- final cursor is not advanced on partial failure;
- response pagination after multi-batch upload returns every remote item once;
- local deletion and conflict semantics remain unchanged;
- automatic-sync deadline spans all batches and stops cleanly;
- one oversized item produces no remote mutation.

### Clock tests

- timestamps inside tolerance pass;
- future `updated_at` outside tolerance reports skew;
- future `created_at` outside tolerance reports skew;
- negative timestamp retains nonnegative diagnostic;
- created-after-updated retains ordering diagnostic.

Do not add large random payload suites, fuzzers, or long soak tests. Construct a small deterministic set of payloads just large enough to cross boundaries.

## 9. Verification commands

Run:

```text
cargo fmt --all -- --check
cargo clippy -p snip-it -p snip-sync --all-targets -- -D warnings
cargo test -p snip-it --lib sync
cargo test -p snip-sync
cargo test --test sync_integration -- --test-threads=1
cargo test --test sync_contracts -- --test-threads=1
cargo test --test <request-sizing-target> -- --test-threads=1   # only if added
cargo check --workspace --all-targets
```

At phase closure:

```text
bash scripts/check.sh
```

Do not run unrelated transaction crash suites.

## 10. Acceptance criteria

- [ ] No generated sync request exceeds the client byte ceiling.
- [ ] Byte sizing uses Prost encoding, not plaintext estimates.
- [ ] One oversized encrypted item fails before any remote mutation.
- [ ] The oversized error reveals no plaintext or credentials.
- [ ] Large libraries sync over multiple deterministic batches.
- [ ] Retry of an accepted batch is idempotent.
- [ ] Failure in batch N stops later batches and does not advance the final cursor.
- [ ] Existing pending auto-sync intent remains available after failure.
- [ ] Remote response pagination still returns and merges every item once.
- [ ] Small normal sync remains simple and does not incur unnecessary extra calls.
- [ ] Existing encryption, conflict, deletion, and local-only field behavior is unchanged.
- [ ] Clock-skew rejection identifies the skew and corrective action.
- [ ] No streaming RPC, new database, client batch journal, compression framework, or protocol redesign is introduced.
- [ ] Focused tests and `bash scripts/check.sh` pass.

## 11. Stop conditions

Stop and amend the roadmap if:

- the existing RPCs cannot support idempotent batching without ambiguous semantics;
- a new RPC or incompatible protobuf change appears necessary;
- batch correctness appears to require a second durable local journal;
- the implementation starts changing merge/conflict semantics;
- payload compression is proposed before simple batching is measured;
- tests require production telemetry or broad failpoint infrastructure.

The correct response to those conditions is design review, not scope expansion inside Phase 13B.