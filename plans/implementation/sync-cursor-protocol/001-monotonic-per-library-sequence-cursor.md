# Sync Cursor Protocol Milestone 001 — Monotonic per-library sequence cursor

Status: ready

Repository baseline: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Source roadmap:

- `plans/subsystems/sync-cursor-protocol-roadmap.md#m001--monotonic-per-library-sequence-cursor`

Long-term requirements:

- `plans/000-long-term-specification.md#4-crate-and-module-ownership`
- `plans/000-long-term-specification.md#6-sync-invariants`
- `plans/000-long-term-specification.md#9-distribution-and-release-invariants`
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules`
- `plans/001-terminology-and-domain-model.md#5-sync-and-execution-terms`

Applicable ADRs:

- `plans/adrs/ADR-0001-sync-delta-cursor.md`

Primary class: invariant

## 1. Objective

Implement ADR-0001's Option A: a server-assigned, strictly increasing
per-library write sequence that the delta filter and the client cursor use,
while the timestamp cursor continues to work unchanged for clients that do
not send one.

## 2. Why this milestone is ready

- No hard dependencies. The protocol change has no upstream milestone; it
  is the only registered decision that crosses the client/server protocol
  boundary.
- The decision is recorded. ADR-0001 selects Option A and specifies the
  additive protocol shape, the server-side counter, the migration, the
  unique-index enforcement, and the version bumps.
- The two cursor columns and the existing fallback path already exist:
  the server's `visible_at` column is the timestamp cursor, the delta
  filter already reads it, and the server already has a migration
  function for adding a cursor column. This milestone adds a second,
  higher-precision cursor beside it rather than replacing the machinery.
- The milestone is one coherent release boundary: three crates, one
  protocol addition, one server schema addition, one client state
  addition, and one version bump in each crate.

## 3. Current implementation evidence

The sync delta cursor is currently a timestamp with two distinct roles,
established by ADR-0001's amendment.

**Server side.** `snip-sync/src/db.rs` defines the `snippets` table with
three ordering-relevant columns: `updated_at`, which holds the client's
logical modification time verbatim and which the server's last-write-wins
comparison reads; `deleted`, an ordinary column; and `visible_at`, which
carries the row's delta cursor position. The delta filter selects rows
with `visible_at >= ?`, and the index
`snippets(user_id, library_id, visible_at, deleted)` serves that range. On
write, the server computes the library's `visible_at` watermark with
`SELECT COALESCE(MAX(visible_at), 0)`, sets the row's `visible_at` to
`max(pushed, watermark)`, and stores it. The upsert clamps the stored
value with `MAX(snippets.visible_at, excluded.visible_at)`, so the cursor
never moves backwards. `get_latest_timestamp` returns the same watermark
and is the value the client receives as `server_timestamp`. The migration
`migrate_snippet_visible_at` adds the column to an existing database and
backfills it with `UPDATE snippets SET visible_at = updated_at`.

**Protocol.** `snip-proto/proto/sync.proto` defines `SyncRequest` with
`int64 last_sync_timestamp = 3` and `SyncResponse` with
`int64 server_timestamp = 4`. Field 7 is the next free number in the
request and field 9 the next free number in the response.

**Client side.** `src/library/model.rs` stores `last_sync: Option<i64>` on
the library. `src/sync.rs` reads it and sends it as `last_sync_timestamp`,
and reads `server_timestamp` from the response. `src/sync_commands.rs`
writes the returned timestamp back through
`mgr.update_last_sync(lib_name, response.server_timestamp)`.

**What this closes.** The timestamp cursor is monotone but not precise. Two
consequences remain. The `>=` filter re-delivers boundary rows: a client
whose cursor equals a row's `visible_at` receives that row again on every
subsequent sync, which is redundant work that the idempotent client merge
absorbs. And second granularity still matters, because two writes within
the same second share a `visible_at` value, so the boundary between rows
the client has seen and rows it has not is not exact.

ADR-0001 records that the accepted fix is a server-assigned, strictly
increasing per-library write sequence, that the row carries the sequence it
was written at, and that the delta filter compares that sequence instead
of `updated_at`. It records the costs: a schema change on the server, a
protocol change, and the requirement that the sequence be assigned inside
the same transaction as the write. It records the compatibility design:
the request carries a new field alongside `last_sync_timestamp`, the
response carries a new field alongside `server_timestamp`, the server uses
the sequence cursor only when the request carries a non-zero one and
otherwise falls back to the timestamp filter, and the client stores both
and sends both. It records the enforcement: the per-library counter must be
updated in the same transaction as the row write, and a unique index on
`(library_id, sequence)` turns a concurrent-write race into a loud failure
rather than a silent one.

**The ADR's status word.** The record's status line reads `amended`, which
is not a state in the ADR status lifecycle defined in
`plans/adrs/README.md` (`proposed -> accepted -> deprecated or superseded`,
or `-> rejected`), and its Decision section describes Option A as
proposed. The decision is in fact settled and the design is fully
specified. Normalizing the status to `accepted`, with the transition
recorded in the ADR's status history, is a precondition for this milestone
and is recorded in the closure evidence.

Known gaps at the baseline: the cursor's precision is bounded by the
timestamp's granularity; the client cannot distinguish "the server sent no
sequence" from "the sequence did not advance"; and the server has no
per-library write counter.

## 4. Invariants that must not regress

- A row written to the server is always delivered to every client that
  syncs after the write, regardless of the timestamp the client chose.
- The client's merge order is unchanged: `(updated_at, device_id,
  SHA-256(synced fields))`, and deletion still beats live content in the
  client merge.
- The server's upsert rule is unchanged: SQL last-write-wins on
  `(updated_at, device_id, command, description)`, with `deleted` an
  ordinary column. The cursor change must not make the server's rule
  stronger or weaker.
- `updated_at` keeps its meaning as the client's logical modification time
  and keeps being stored verbatim.
- `output`, `folders`, and `favorite` remain local-only and remain absent
  from the synced protobuf message.
- The server's `snippets.id` remains a global `TEXT PRIMARY KEY`, and the
  upsert's foreign-scope guard is unchanged.
- Proto3 additive fields keep both version-skew directions working: an old
  client against a new server, and a new client against an old server.
- The client byte ceiling remains below the server's message ceiling, and
  the multi-batch error kind preservation is unchanged.
- The server's HTTP surface — one EggServe leaf with `/health` and
  `/metrics` — is unchanged, including its status codes, `Allow`/`Vary`
  headers, and security headers.
- A cursor reveals nothing about snippet content.
- No credential is transmitted by this change.

## 5. Scope

### In scope

- Add a server-assigned per-library write sequence to the server's
  `snippets` table, plus the per-library counter that generates it.
- Assign the sequence inside the same transaction as the row write.
- Add a unique index on `(library_id, sequence)` so a concurrent counter
  race fails loudly.
- Add a migration that adds the sequence column and backfills it so that
  the backfilled ordering preserves the pre-migration `updated_at`
  ordering.
- Add a sequence field to `SyncRequest` and a sequence field to
  `SyncResponse`, using the next free field numbers.
- Select on the sequence when the request carries a non-zero one, and fall
  back to the existing `visible_at` filter otherwise.
- Store the sequence cursor on the client alongside the existing timestamp
  cursor, send both, and use the sequence when the server returns one.
- Bump the `snip-proto` version and the versions of `snip-sync` and
  `snip-it` in the same change.
- Update the protocol documentation and the identity contract's known
  limitations.

### Explicitly out of scope

- Removing the `visible_at` column or the timestamp cursor. Both remain,
  because old clients still send the timestamp cursor.
- Changing the server's upsert rule or the client's merge order.
- Changing the deterministic identifier derivation or the fingerprint field
  set.
- Changing the synced field set, the local-only field split, or the
  protobuf `Snippet` message.
- Changing the client's byte ceiling, the server's message ceiling, or the
  multi-batch retry policy.
- Changing the server's HTTP surface, the gRPC service surface beyond the
  two additive fields, or the metrics format beyond an optional cursor-kind
  report.
- Any new crate, streaming sync mode, long-polling sync mode, or server
  push.
- Re-encryption, key rotation, or credential-store changes.
- Changing the publish order `snip-proto` → `snip-sync` → `snip-it`.
- The cursor-precision convergence work, which is M002 of the same
  roadmap.

## 6. Required production changes

### Core/domain

The server gains a per-library write counter. The counter is bumped and the
sequence assigned in the same transaction as the row write, so two
concurrent writes cannot observe the same counter value.

The row's stored sequence is its position in that library's write order. It
is a server-assigned value, never a client-supplied one, exactly as
`visible_at` is server-assigned today.

The client gains a stored sequence cursor alongside its stored timestamp
cursor. Both are sent in every sync request. The client advances its
sequence cursor only when the response carries a sequence; if the response
does not, the client continues to use the timestamp cursor, which is the
path taken against an old server.

### Storage and migrations

The server's `snippets` table gains a sequence column and the server gains
a per-library counter table or counter column.

The migration adds the column and backfills it so that the backfilled
ordering preserves the pre-migration `updated_at` ordering. This is the
property that makes a migrated database behave like an unmigrated one for
the first sync: the cursor a client holds selects the same rows it would
have selected before the migration.

A unique index on `(library_id, sequence)` is the enforcement mechanism
for the concurrent-write race. The unique index is what turns a
shared-sequence race into a loud failure; the transaction alone is not
sufficient, because a counter read and a counter write in separate
transactions can interleave.

No client-side storage migration is required. The client stores the new
cursor alongside the existing one, and a client that has never synced with
a sequence-capable server simply has no sequence cursor.

### Protocol and DTOs

`snip-proto/proto/sync.proto` gains one field in `SyncRequest` and one
field in `SyncResponse`, both `int64`, using the next free field numbers
(7 and 9). The generated stubs in `snip-proto/src/snip_proto.rs` are
regenerated as part of this change; they are checked in and are not
edited by hand.

The server reads the request's sequence field. A zero value means "no
sequence cursor", and the server then uses the existing `visible_at` filter
exactly as it does today. A non-zero value means the client is on the
sequence path, and the server selects on the sequence.

The server returns the sequence watermark in the response's new field
when the request carried one, and returns zero otherwise, so an old client
never receives a value it cannot interpret.

### Runtime and concurrency

The counter update and the row write share one transaction. A cancelled or
failed push must not leave the counter and the row inconsistent.

The server's delta filter gains a second form, selected by whether the
request carries a sequence cursor. Both forms are served by the existing
`(user_id, library_id, visible_at, deleted)` index; the sequence path
requires its own index to remain efficient, which the milestone adds.

The client's sync path sends both cursors on every request, including
push-and-pull requests, so that the server can select the right form
without a separate negotiation round trip.

### Frontend or operator surface

The `snp status` output may report which cursor kind the server is
serving. If it does, the change is additive to the existing JSON output
shape and is documented in `docs/JSON_SCHEMAS.md`. No CLI flag changes.

### Security and authorization

The cursor reveals nothing about content; it is a write counter. A client
cannot use the cursor to skip rows it has not seen: the counter advances on
every write and the filter selects every row written after the cursor value
the client holds.

The sequence is not a capability and grants no access the client did not
already have. The existing per-user and per-library scoping in the delta
filter is unchanged, so the sequence cannot be used to read another
account's or another library's rows.

No credential is transmitted by this change.

### Documentation and static guards

- `snip-proto/proto/sync.proto` — the two new fields, with comments
  stating their semantics and their zero-value behavior.
- `architecture/proto.md` — the new fields and their compatibility
  contract.
- `architecture/sync.md` — the cursor model: two cursors, their roles, and
  the fallback.
- `architecture/server.md` — the server-side counter, the unique index, and
  the transactional requirement.
- `docs/IDENTITY_CONTRACT.md` — the known-limitation entry is updated to
  describe the sequence cursor's status.
- `architecture/status.md` — if the status output changes.
- `docs/JSON_SCHEMAS.md` — if the status output changes.
- `CHANGELOG.md` — the protocol addition and the version bumps.
- `RELEASING.md` — only if the release procedure needs a note about the
  coordinated version bumps.

## 7. Ordered work packages

### Work package A — Normalize the ADR status

Intent: make the governing decision handoff-eligible before implementation
begins.

Required changes: change ADR-0001's status line from the off-lifecycle
word `amended` to `accepted`; add a status-history note recording the
previous word, the date of the transition, and the reason; update the
record's affected-subsystem-roadmap pointer to the sync cursor protocol
roadmap; do not rewrite the ADR's body, its Decision, its Considered
options, or its amendment section.

Acceptance evidence: the ADR's status is a word in the ADR status
lifecycle; the body's decision content is unchanged; the transition is
visible in the file.

### Work package B — Add the server-side sequence and counter

Intent: give the server a monotone write order it assigns itself.

Required changes: add the sequence column to the `snippets` table
definition; add the per-library counter; assign the sequence in the same
transaction as the row write; add the unique index on
`(library_id, sequence)`; add the migration that adds the column and
backfills it preserving the `updated_at` ordering; add the sequence
watermark query.

Acceptance evidence: two concurrent pushes to one library receive distinct
sequence values, proven by a test that would fail loudly through the
unique index if they did not; the migration test proves the backfilled
ordering matches the pre-migration ordering.

### Work package C — Extend the protocol

Intent: carry the sequence cursor in both directions.

Required changes: add the request field and the response field to
`snip-proto/proto/sync.proto`; regenerate the checked-in stubs in
`snip-proto/src/snip_proto.rs`; bump the `snip-proto` version; implement
the server's selection of the sequence path when the request carries a
non-zero sequence, falling back to the `visible_at` filter otherwise;
return the sequence watermark in the response field when the request
carried one, and zero otherwise.

Acceptance evidence: an old client sending no sequence cursor receives the
rows the timestamp filter would have selected; a client sending a sequence
cursor receives the rows the sequence filter selects; the generated stubs
match the proto, proven by the build and by the existing stub-check
procedure.

### Work package D — Extend the client

Intent: store, send, and advance the sequence cursor.

Required changes: store the sequence cursor on the library alongside
`last_sync`; send both cursors in every sync request; advance the
sequence cursor only when the response carries a non-zero sequence;
continue to advance the timestamp cursor as today, so that a fallback to
an old server keeps working; keep the client's `last_sync` semantics
unchanged for the timestamp path.

Acceptance evidence: the client sends both cursors on a request; a
response with no sequence leaves the client's sequence cursor unchanged;
the client's merge order and its `last_sync` handling are unchanged, proven
by the existing sync and multi-batch suites.

### Work package E — Bump the versions and document

Intent: land the change as a coordinated release.

Required changes: bump the `snip-sync` and `snip-it` versions in the same
change as the `snip-proto` bump; update `CHANGELOG.md` with the protocol
addition and the three version bumps; update `architecture/proto.md`,
`architecture/sync.md`, `architecture/server.md`, and
`docs/IDENTITY_CONTRACT.md`; add the `CHANGELOG.md` entry required by
`RELEASING.md` for a version-bump change.

Acceptance evidence: all three versions moved together; the publish order
`snip-proto` → `snip-sync` → `snip-it` is recorded in the closure
evidence; the documentation describes the two-cursor model accurately.

## 8. Failure, cancellation, restart, and contention semantics

**Partial failure.** The counter increment and the row write share a
transaction, so a failed write does not consume a sequence value. A skipped
sequence value would be harmless, but a reused one would not be, and the
shared transaction is what prevents reuse.

**Duplicate delivery.** The delta filter is `>=`, so a row whose sequence
equals the client's cursor is delivered again. The client merge is
idempotent for an unchanged row, so the re-delivery is absorbed. This is
the same property the timestamp cursor has today.

**Process restart.** The counter lives in the database, so a server restart
does not reset it. A client that loses its cursor state falls back to a
full re-scan, which the existing merge path already handles; the sequence
cursor introduces no new recovery mode.

**Cancellation races.** A cancelled push leaves the counter and the row
consistent because both are in one transaction. A cancelled pull leaves
the client's cursor unchanged, so the next pull re-selects the same rows.

**Stale generations.** Not applicable to the server's counter. The
client's pending-sync generation handling is unchanged.

**Concurrent callers.** Two concurrent pushes to the same library
serialize on the counter row and receive distinct sequence values. Two
concurrent pushes to different libraries use different counters and do not
interfere.

**Counter race enforcement.** If a race nonetheless produced two rows with
the same `(library_id, sequence)`, the unique index rejects the second
write. That is a loud failure, not a silent one, and it is the mechanism
that prevents the original defect from reappearing in a narrower window.

**Fallback correctness.** When the server returns no sequence, the client
must continue to use the timestamp cursor. If the client instead advanced
its sequence cursor from a zero response, a later sync against a
sequence-capable server would skip rows. The fallback path is therefore
covered by a test that runs a sequence-less response and then asserts the
next request still carries the correct timestamp cursor.

## 9. Compatibility and migration

**Backward compatibility.** The new fields are additive. Proto3 ignores
unknown fields, so an old client talking to a new server behaves exactly as
it did before, and a new client talking to an old server behaves exactly as
it did before, because the old server ignores the new request field and
returns no sequence.

**Server storage migration.** The migration adds the sequence column and
backfills it so that the backfilled ordering preserves the pre-migration
`updated_at` ordering. A migrated database therefore selects the same rows
as an unmigrated one for the first sync, and every client's stored cursor
stays valid.

**Client storage.** No client-side migration. The sequence cursor is
absent until the server returns one.

**Protocol negotiation.** None is needed. The presence of a non-zero
sequence in the request is the negotiation.

**Configuration fallback.** None is needed.

**Removal criteria.** The timestamp cursor is not removed by this
milestone. It is removed only when no supported client sends it, which is
M002's subject and, ultimately, a release-policy question rather than a
code question.

## 10. Required tests

### Focused unit tests

- A server unit test that the watermark query returns the maximum sequence
  for the library.
- A server unit test that a write assigns a sequence strictly greater than
  the library's current maximum.
- A client unit test that the stored sequence cursor is sent on a request
  and that a zero-valued response sequence leaves it unchanged.

### Integration tests

- A server test that pushes a row with an `updated_at` below the library
  maximum and proves a third client, syncing from the current sequence
  cursor, receives it.
- A server test that a second push of the same row with a newer `updated_at`
  still wins the last-write-wins comparison, proving the cursor change did
  not alter conflict resolution.
- A server test that an old client sending no sequence cursor receives the
  rows the timestamp filter would have selected.
- A server test that a client sending a sequence cursor receives exactly
  the rows written after that sequence.
- A client test that the merge order is unchanged, exercised through the
  existing conflict-resolution assertions.
- A test that the two cursors are both sent on a request.

### Restart and recovery tests

- A server test that the counter survives a server restart and continues
  from where it left off.
- A client test that a full re-scan still reconciles correctly when the
  sequence cursor is absent.

### Contention and cancellation tests

- A server test that two concurrent pushes to one library receive distinct
  sequence values.
- A server test that a cancelled or failed push does not consume a
  sequence value.
- A client test that a failed pull leaves the cursor unchanged and the next
  pull re-selects the same rows.

### Security and negative tests

- A server test that a sequence cursor from one library cannot select
  another library's rows.
- A server test that the sequence cursor carries no content.

### Migration and compatibility tests

- A server migration test that an existing database is upgraded and the
  backfilled ordering matches the pre-migration `updated_at` ordering.
- A compatibility test that an old client against a new server, and a new
  client against an old server, both behave exactly as before.

## 11. Required verification commands

```bash
# narrow tests first
cargo test --workspace --lib
cargo test -p snip-sync --lib
cargo test --test sync_multibatch --features test-support -- --test-threads=1
cargo test --test snip_sync_lifetime -- --test-threads=1

# lint gates
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings

# authoritative gate
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

Do not claim commands that were not actually run in the closure record.

`sync_multibatch` and `snip_sync_lifetime` are serial. Use
`--test-threads=1` for them.

## 12. Documentation updates

- `snip-proto/proto/sync.proto` — the two new fields.
- `architecture/proto.md` — the new fields and their compatibility
  contract.
- `architecture/sync.md` — the two-cursor model and the fallback.
- `architecture/server.md` — the counter, the unique index, and the
  transactional requirement.
- `architecture/overview.md` §7 — if the cross-cutting sync invariants
  name the cursor.
- `docs/IDENTITY_CONTRACT.md` — the known-limitation entry.
- `docs/JSON_SCHEMAS.md` — only if the status output changes.
- `CHANGELOG.md` — the protocol addition and the three version bumps.
- `RELEASING.md` — only if the coordinated version bumps need a note.

## 13. Acceptance criteria

1. The server assigns a strictly increasing per-library sequence to every
   row write, inside the same transaction as the write.
2. A unique index on `(library_id, sequence)` exists, and two concurrent
   pushes to one library receive distinct sequence values.
3. The delta filter selects on the sequence when the request carries a
   non-zero sequence, and falls back to the existing `visible_at` filter
   otherwise.
4. The request and response carry the new sequence fields additively, with
   the next free field numbers, and the checked-in stubs are regenerated
   rather than hand-edited.
5. The client stores, sends, and advances the sequence cursor, and keeps
   using the timestamp cursor when the response carries no sequence.
6. The client's merge order, the server's upsert rule, the synced field
   set, the deterministic identifier derivation, and the local-only field
   split are all unchanged, proven by the existing conflict-resolution and
   identity tests.
7. An old client against a new server, and a new client against an old
   server, both behave exactly as before.
8. The migration adds the sequence column and backfills it so that the
   backfilled ordering preserves the pre-migration `updated_at` ordering,
   and a migrated database selects the same rows as an unmigrated one for
   the first sync.
9. The `snip-proto`, `snip-sync`, and `snip-it` versions are bumped in the
   same change, and the publish order is recorded.
10. The server's HTTP surface, its gRPC service surface beyond the two
    additive fields, and the wire-parity assertions are unchanged.
11. `bash scripts/check.sh` and `bash scripts/ci/test-production-seams.sh`
    pass, with the serial suites green under `--test-threads=1`.
12. ADR-0001's status is a word in the ADR status lifecycle, and its body
    is unchanged.

## 14. Stop conditions

The agent must stop and report rather than improvise when:

- the unique index cannot be added, because a shared sequence value already
  exists in a live database — that would mean the counter race is real and
  the design needs a repair migration before the feature can land;
- the sequence cannot be assigned transactionally with the write, because
  the server's write path does not support it — that would reintroduce the
  original defect in a narrower window;
- the protocol change would require modifying or removing an existing
  field number rather than adding one;
- a hard dependency is absent;
- the scope would expand into the merge order, the upsert rule, the
  identity contract, or the synced field set;
- the version bumps cannot be made in the same change;
- repository evidence contradicts the description of the current cursor
  behavior in this plan.

## 15. Closure evidence required

- The `cargo clippy --workspace --all-targets -- -D warnings` and
  `--all-features` results.
- The `bash scripts/check.sh` and
  `bash scripts/ci/test-production-seams.sh` results, including the serial
  suites.
- The concurrent-push test result showing distinct sequence values.
- The migration test result showing the backfilled ordering matches the
  pre-migration ordering.
- The old-client and new-client compatibility test results, in both
  directions.
- The fallback test result showing a sequence-less response leaves the
  client's sequence cursor unchanged.
- Evidence that the merge order and the upsert rule are unchanged, with
  the tests that prove each.
- The three version numbers after the bump, and the statement that they
  moved in the same change.
- The regenerated stub diff summary, confirming the stubs were generated
  rather than hand-edited.
- The ADR status normalization, with the transition note.

## 16. Handoff notes

- The working tree was clean at the baseline commit
  `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`. Preserve any user changes
  that appear; do not reset unrelated work.
- Never run `cargo build`, `cargo test`, or `cargo clippy` while another
  test run is in flight. The integration tests execute
  `target/debug/snp`; a rebuild mid-run replaces that binary and produces
  false failures.
- `sync_multibatch` and `snip_sync_lifetime` are serial. Use
  `--test-threads=1` for them.
- `snip-proto/src/snip_proto.rs` is checked in and is regenerated, never
  hand-edited. `protoc` is needed only for an explicit regeneration after
  editing `snip-proto/proto/sync.proto`.
- A proto change requires bumping the `snip-proto` version and the
  versions of both dependents in the same change. The publish order is
  `snip-proto` → `snip-sync` → `snip-it`.
- `snip-sync`'s HTTP surface is one concrete two-route EggServe leaf.
  Read `tests/snip_sync_lifetime.rs` before touching status codes, the
  `Allow`/`Vary` headers, or the security headers; the wire parity is
  locked there.
- The server's `snippets.id` is a global `TEXT PRIMARY KEY`, not per user
  or library. A push with the same content-derived identifier under a
  different library writes nothing and is reported as a foreign-scope
  outcome. Do not weaken that guard while adding the sequence column.
- `snip-sync` uses `sqlite::memory:` in tests. Never touch a real
  configuration directory, keychain, or port from a test.
- `SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on all test commands and must
  never be removed.
- In edition 2024, `std::env::set_var` requires `unsafe`.
