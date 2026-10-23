# Sync Cursor Protocol Milestone 002 — Cursor bootstrap and precision convergence

Status: ready

Repository baseline: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Source roadmap:

- `plans/subsystems/sync-cursor-protocol-roadmap.md#m002--cursor-precision-convergence`

Long-term requirements:

- `plans/000-long-term-specification.md#6-sync-invariants`
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules`
- `plans/001-terminology-and-domain-model.md#5-sync-and-execution-terms`

Applicable ADRs:

- `plans/adrs/ADR-0001-sync-delta-cursor.md`

Primary class: invariant

This milestone extends the roadmap's M002 scope. The roadmap's M002
deliverable is the client's cursor state recording which cursor the server is
serving. The M001 closure record adds a named condition that this plan must
satisfy: **the sequence cursor cannot bootstrap, so the exact path is not yet
exercised in production.** The bootstrap handshake is therefore in scope here,
and the roadmap's M002 milestone is extended to cover it. See
`plans/closure/sync-cursor-protocol/001-status.md` §10 and §11.

## 1. Objective

Make the server-assigned write-sequence cursor reachable: give the client a
way to learn the server's sequence watermark, adopt it, and then use it, and
make the client's persisted cursor state record which cursor the server is
actually serving.

## 2. Why this milestone is ready

- M001 is implemented and its closure record exists. The server assigns a
  strictly increasing per-library sequence, the protocol carries
  `last_sync_sequence` and `server_sequence`, the client stores and sends the
  cursor, and the delta filter can select on it.
- The M001 closure is conditional on exactly one named condition, and this
  plan is that condition's remedy: the bootstrap. The condition, its risk,
  and the required evidence are all recorded.
- The protocol change is already additive and shipped. This milestone needs
  **no new protocol field**: the bootstrap is expressed with the two fields
  that already exist, by changing which request the client makes and which
  answer the server gives.
- The client's cursor state, the server's watermark, and the delta filter are
  all in place, so the milestone is a state-machine change on two sides plus
  the documentation that describes it.

## 3. Current implementation evidence

### The defect

M001 shipped a working sequence path that no client can enter.

`snip-sync/src/lib.rs` decides the delta filter from the request:

```rust
let sequence_cursor = req.last_sync_sequence;
let cursor = if sequence_cursor != 0 {
    db::DeltaCursor::Sequence(sequence_cursor)
} else {
    db::DeltaCursor::Timestamp(req.last_sync_timestamp)
};
```

and answers the watermark only when the request carried one:

```rust
let sequence = if sequence_cursor == 0 {
    0
} else if has_more {
    sequence_cursor
} else {
    self.db.get_latest_sequence(&user_id, &library_id).await?
};
```

The client's side is the mirror image
(`src/sync_commands.rs::sequence_cursor_from_response`): a zero
`server_sequence` leaves the stored cursor untouched, and a non-zero one
advances it.

The two rules together are a closed loop with no entry point. A client that
has never stored a sequence sends `last_sync_sequence = 0`, so the server
serves the timestamp filter and answers `server_sequence = 0`, so the client
stores nothing and sends `0` again, forever. The exact path is unreachable at
runtime.

The M001 closure record accepted this as safe — a client in that state
receives exactly the pre-M001 row set, and both version-skew directions are
unchanged — but recorded that the milestone's stated value (an exact delta
cursor) is unrealized until the bootstrap exists.

### Why the client cannot bootstrap from a delta

The obvious fix — "always answer with the watermark" — is unsafe, and the
reason is specific enough that the milestone must record it.

A sequence cursor is only adoptable by a client that holds **every** row the
watermark covers: the delta filter is `sequence >= value`, so a row the
client never received but whose sequence is below the adopted watermark would
be skipped forever.

The timestamp cursor cannot establish that guarantee for the delta it
receives, because the timestamp cursor is a **position, not a receipt**. The
server reports the library's `visible_at` watermark and the client stores it
as its cursor — including in a sync that did not deliver the rows at that
position. A push-only sync discards the rows the server served and still
advances the timestamp cursor past them, so that client can never prove it
holds the rows below the watermark it was handed. The delta's coverage
therefore depends on what the client received in *earlier* syncs, not on what
the current response carried.

The cursorless full sync is the one request shape that delivers the complete
library in one response, so it is the only one that establishes the
guarantee directly. That is why the bootstrap is a full sync and not a delta.

A note on scope, because the two cursors are easy to confuse. The **original**
defect ADR-0001 describes — a row pushed with an old `updated_at` never being
delivered to clients whose watermark was above it — was already closed by
M001's amendment, which split the cursor's two roles: the delta filter uses
the server-assigned `visible_at` (`updated_at.max(watermark)`), while the
stored `updated_at` keeps the client's logical time for the server's
last-write-wins rule. What the write-sequence cursor adds on top of that is
a cursor that is a strict, server-assigned **total order of writes**, with no
dependence on any client's clock, and that the client advances only in a sync
that merged what it was served.

### What "holds the complete library" requires

Exactly one request shape delivers every row: a **full sync** — no timestamp
cursor and no sequence cursor. The server then serves the timestamp filter
with `visible_at >= 0`, which selects every row of the library.

The server's watermark read happens after the page read. A row written in
between carries the counter value the watermark reports, so the inclusive
sequence filter (`sequence >= value`) re-delivers it on the next sync. The
inclusive comparison is what makes the bootstrap safe without a cross-statement
snapshot; this is the same safety property the timestamp cursor already
relies on, and it must be pinned by a test.

### What the client already carries

- `LibraryMeta.last_sync_sequence: Option<i64>` (`src/library/model.rs`) —
  the stored sequence cursor. `None` means "no cursor".
- `LibraryMeta.last_sync: Option<i64>` — the stored timestamp cursor.
- `SequenceCursor::{Advance, Keep, Reset}` (`src/library/model.rs`) and
  `LibraryManager::update_sync_cursors(filename, timestamp, cursor)`
  (`src/library/manager.rs`) — the write path for the cursors.
- `get_library_sync_info` (`src/sync_commands.rs`) — the read path; it returns
  `(library_id, last_sync, last_sync_sequence)` and sends zero for an absent
  cursor.

The client therefore has no way to distinguish "the server sent no sequence"
from "the sequence did not advance": both arrive as `server_sequence == 0` or
as an unchanged value, and `Keep` covers both. That is the gap the roadmap's
M002 names.

## 4. Invariants that must not regress

- The **client merge order** stays `(updated_at, device_id, SHA-256(synced
  fields))`, and deletion beats live content. No merge rule changes here.
- The **server's last-write-wins upsert** on `(updated_at, device_id, command,
  description)` and its treatment of `deleted` as an ordinary column stay
  exactly as they are. The sequence column is a cursor, never a conflict
  input.
- The **deterministic identifier derivation** and the **fingerprint field
  set** are untouched.
- The **synced field set** is untouched. The sequence is server-side
  metadata; it is never synced as snippet content.
- The **timestamp cursor stays the fallback**. A client that sends no sequence
  cursor is served by the timestamp filter, exactly as before.
- The protocol change stays **additive**. No existing field changes type,
  number, or meaning; no field is removed.
- The **per-library sequence counter** keeps its current semantics: a
  strictly increasing, server-assigned value; a rolled-back write consumes no
  sequence value; the unique index on `(library_id, sequence)` still turns a
  counter race into a loud failure.
- The client's **merge, conflict, and skip accounting** is untouched, including
  the rule that `last_sync` does not advance when records were skipped.

## 5. Scope

### In scope

- The **bootstrap handshake**: the client performs one full sync to learn the
  server's sequence watermark, and adopts it.
- The **server's answer rule**: the server answers the sequence watermark for
  the bootstrap request as well as for a sequence request, and continues to
  answer zero for a timestamp delta.
- The **client's cursor state**: the client's persisted state records which
  cursor the server is serving, so it can distinguish "the server sent no
  sequence" from "the sequence did not advance".
- The **client's request selection**: the client chooses the request shape
  from its recorded cursor state.
- The **cursor-state reset paths**: a library that is relinked, reset, or
  re-created returns to the "unknown" state and re-bootstraps.
- The **boundary-row re-delivery documentation** for clients that stay on the
  timestamp path.
- The version bumps for the crates whose behavior changes.

### Explicitly out of scope

- Removing the timestamp cursor. The roadmap defers that; this milestone
  keeps it as the fallback and never removes it.
- Changing the merge order, the conflict rules, the server's upsert, the
  fingerprint, the identifier derivation, or the synced field set.
- Any new protocol field. The bootstrap reuses `last_sync_sequence` and
  `server_sequence` as they are.
- Encrypting the cursor, changing the encryption contract, or changing the
  API-key or keychain handling.
- Changing the client's retry policy, the multi-batch push behavior, or the
  upload ceilings.
- Changing `snip-sync`'s HTTP surface, its metrics, or its singleton lock.

## 6. Required production changes

### The bootstrap handshake

**Request shapes.** The client's request carries three distinguishable
shapes, using the two existing protocol fields:

| `last_sync_sequence` | `last_sync_timestamp` | Shape | Server filter |
|---|---|---|---|
| non-zero | any | sequence delta | `sequence >= cursor` |
| zero | non-zero | timestamp delta | `visible_at >= timestamp` |
| zero | zero | **full sync** (the bootstrap) | `visible_at >= 0` (every row) |

**Server answer rule.** The server answers `server_sequence`:

- with the request's own cursor while the response is paginated
  (`has_more`), exactly as today — a paginated answer carries no
  information about the watermark;
- with the library's current sequence watermark when the response is
  complete and the request carried a **sequence** cursor (today's behavior);
- with the library's current sequence watermark when the response is complete
  and the request carried **no cursor at all** (the bootstrap — the new
  rule);
- with **zero** when the response is complete and the request carried only a
  timestamp cursor, because a timestamp delta does not deliver the library's
  whole history and its watermark is not adoptable by that client.

The zero case is the only change to the server's answer rule, and it is
strictly additive: a client that ignores `server_sequence` (any pre-0.1.4
client) is unaffected.

**Client cursor state.** The client's persisted per-library state gains one
field that records which cursor the server is serving:

- `unknown` — the client has not probed. The next sync is a bootstrap full
  sync.
- `timestamp` — the server did not serve a sequence. The client sends the
  timestamp delta and never sends a sequence cursor.
- `sequence` — the client holds a sequence cursor in `last_sync_sequence` and
  sends it on every request.

The field is additive in the library TOML and defaults to `unknown` when
absent, so every existing library file loads unchanged.

**Client request selection and answer handling.**

| Recorded state | Request | Complete response with `server_sequence == 0` | Complete response with `server_sequence != 0` |
|---|---|---|---|
| `unknown` | full sync (both cursors zero) | the server serves the timestamp path → record `timestamp` | adopt the watermark → record `sequence`, store the cursor |
| `timestamp` | timestamp delta | stays `timestamp` | (the server answers zero here by rule) |
| `sequence` | sequence delta (send the stored cursor) | the server stopped serving sequences → record `timestamp`, drop the stored cursor | advance the stored cursor to the returned value |

The client interprets the answer **only from a complete response**. A
paginated response carries no watermark information, so it never changes the
recorded state — this matters for the bootstrap, whose first page answers zero
while more pages remain.

Two distinct states are now distinguishable, which is what the roadmap's M002
requires: "the server sent no sequence" (recorded state becomes `timestamp`,
the stored cursor is dropped) versus "the sequence did not advance" (the
server answers the same value it was given, the recorded state stays
`sequence`, and the stored cursor is unchanged).

**Fallback safety.** The timestamp cursor is advanced on every sync,
including on the sequence path, so a client that falls back to the timestamp
path holds a current timestamp cursor and does not re-download the library.
The fallback is the timestamp path's known weakness (a clock-skewed write can
be skipped); the milestone does not claim to fix that for a client that never
bootstraps.

### Storage and migrations

- The server's schema is unchanged. The `sequence` column, the
  `library_sequences` counter table, and the unique index stay as they are.
- The client's library TOML gains one additive field. The loader defaults it
  to `unknown`; no migration is needed and no schema version is introduced.

### Protocol and DTOs

- **No protocol change.** `snip-proto` keeps version 0.1.4 and its checked-in
  stubs are not regenerated.
- The two crates whose behavior changes are version-bumped: `snip-sync`
  0.1.7 → 0.1.8 and `snip-it` 1.4.0 → 1.4.1. The publish order stays
  `snip-proto` → `snip-sync` → `snip-it`; `snip-proto` is not republished
  because it did not change.

### Runtime and concurrency

- The bootstrap is one extra full-sync round trip per library, once. It is
  not a background job and introduces no new scheduler state.
- The server reads the watermark after the page, in the same request, with
  the same pagination rules as today.
- No lock, journal, or transaction behavior changes. The cursor update goes
  through the existing `update_sync_cursors` write path.

### Security and authorization

- No credential handling change. The cursors are library metadata and are
  stored in the library file, not in the keychain.
- The bootstrap delivers the same rows a normal sync delivers; it changes no
  authorization rule, and the server still scopes every query by
  `(user_id, library_id)`.

## 7. Ordered work packages

### Work package A — Server: answer the bootstrap

Intent: let a client that has no cursor learn the watermark from a single
full sync.

Required changes: in the sync handler, compute the answer as: echo the
request's cursor while paginating; answer the watermark when the request
carried a sequence cursor or no cursor at all; answer zero when the request
carried only a timestamp cursor.

Acceptance evidence: a request with no cursors returns every row of the
library and a non-zero `server_sequence`; a request with only a timestamp
cursor returns its delta and `server_sequence == 0`; a request with a
sequence cursor returns the rows after it and the watermark; a paginated
response carries no new information.

### Work package B — Client: the cursor state machine

Intent: make the client able to enter, stay on, and leave the sequence path.

Required changes: add the recorded cursor state to the library metadata with
an `unknown` default; select the request shape from it; interpret the
answer only from a complete response; adopt, advance, and drop the stored
cursor as the table above specifies; keep advancing the timestamp cursor on
every sync.

Acceptance evidence: a first sync against a sequence-capable server ends with
the recorded state `sequence` and a stored cursor; the next sync sends a
non-zero `last_sync_sequence`; a server that answers no sequence moves the
client to `timestamp` and drops the stored cursor; a paginated bootstrap
does not change the recorded state.

### Work package C — Reset paths

Intent: a library that is relinked, reset, or re-created must be able to
bootstrap again.

Required changes: every path that clears `last_sync` or the library linkage
also returns the recorded cursor state to `unknown`.

Acceptance evidence: a reset library's next sync is a full sync and it
bootstraps again.

### Work package D — Documentation of the boundary-row behavior

Intent: make the remaining timestamp-path behavior explicit instead of
implicitly relied upon.

Required changes: document, for clients that stay on the timestamp path, that
the timestamp cursor is a **position, not a receipt** — the server reports the
library watermark and the client stores it even in a sync that discarded the
rows at that position — and that this, rather than the clock, is the residual
limitation the write-sequence cursor closes. State explicitly that the
original ADR-0001 defect was already closed by M001's `visible_at` clamp, so
the two are not conflated.

Acceptance evidence: the behavior is described in `architecture/sync.md` and
in `docs/IDENTITY_CONTRACT.md`, and the statement matches the code.

### Work package E — Version bumps and the release boundary

Intent: one coherent release boundary for a behavior change on both sides.

Required changes: bump `snip-sync` and `snip-it`; refresh `Cargo.lock`; record
the change in `CHANGELOG.md`.

Acceptance evidence: the workspace builds and tests at the new versions, and
the publish order is stated in the closure record.

## 8. Failure, cancellation, restart, and contention semantics

- **Interrupted bootstrap.** A bootstrap that fails or is interrupted leaves
  the recorded state unchanged (`unknown`), so the next sync simply attempts
  the bootstrap again. A full sync is idempotent: the merge applies the same
  rows again.
- **Interrupted sequence sync.** The stored cursor is written only after a
  complete response, so an interrupted sync leaves the previous cursor in
  place and the next sync re-requests the same range. The sequence filter is
  inclusive, so the boundary row is re-delivered rather than skipped.
- **Server restart.** The counter lives in the database, not in memory, so it
  survives a restart. The watermark is read from the database on every
  request.
- **Contention.** Two clients syncing the same library receive distinct
  sequence ranges; the counter is bumped inside the write transaction, so a
  rolled-back write consumes no value and the unique index turns a race into
  a loud failure rather than a silent collision.
- **Pagination.** The watermark is withheld while pages remain, exactly as
  the timestamp watermark is. The client reads the cursor state only from a
  complete response.

## 9. Compatibility and migration

- **New client, old server.** The client sends a full sync; the old server
  ignores the unknown field and answers no sequence; the client records
  `timestamp` and behaves exactly as it did before.
- **Old client, new server.** The old client sends no sequence; the server
  serves the timestamp filter; the server's new answer rule sends a
  watermark in the bootstrap case, which the old client ignores as an
  unknown field. The old client's row set is unchanged.
- **New client, new server.** The client bootstraps on its first sync and is
  on the exact path from the second sync onward.
- **Existing library files.** The new metadata field is additive and defaults
  to `unknown`; no file is rewritten on load, and no migration runs.
- **Existing databases.** The server's schema is unchanged.

## 10. Required tests

### Focused unit tests

- The server's answer rule: one test per request shape (no cursors, timestamp
  only, sequence) asserting both the rows returned and the `server_sequence`
  value.
- The client's cursor-state transitions: `unknown` → `sequence`,
  `unknown` → `timestamp`, `sequence` → `sequence` (advance),
  `sequence` → `timestamp` (the server stopped serving sequences), and
  "paginated response changes nothing".
- The metadata round trip: the new field defaults to `unknown` when absent
  and survives a save/load cycle.

### Integration tests

- A client that synced against a sequence-capable server sends a non-zero
  `last_sync_sequence` on its next sync and the server answers with a
  non-zero `server_sequence` — the closing evidence the M001 closure record
  names.
- A bootstrap delivers the complete library: a client that had a timestamp
  cursor and then bootstraps holds every row, so no row is skipped by the
  subsequent sequence delta.
- The boundary-row property: a row written between the bootstrap's page read
  and its watermark read is re-delivered by the next sequence sync.
- A client against a server that answers no sequence stays on the timestamp
  path and never sends a sequence cursor.

### Restart and recovery tests

- None beyond the interrupted-bootstrap case above; the milestone introduces
  no new state machine on disk.

### Contention and cancellation tests

- None. The milestone touches no lock.

### Security and negative tests

- A client that has not bootstrapped cannot adopt a watermark from a
  timestamp delta: the server answers zero for that shape, so the client has
  nothing to adopt.

### Migration and compatibility tests

- A library file without the new field loads with the recorded state
  `unknown`.
- The existing sync contracts and integration suites still pass.

## 11. Required verification commands

```bash
# narrow tests first
cargo test --workspace --lib
cargo test -p snip-sync --lib
cargo test --test sync_contracts --features test-support
cargo test --test sync_integration --features test-support
cargo test --test sync_multibatch --features test-support -- --test-threads=1

# lint gates
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Do not claim commands that were not actually run in the closure record.

## 12. Documentation updates

- `architecture/sync.md` — the cursor-state machine, the bootstrap handshake,
  and the boundary-row behavior of the timestamp path.
- `architecture/server.md` — the server's answer rule.
- `docs/IDENTITY_CONTRACT.md` — the cursor metadata contract and the
  boundary-row behavior.
- `plans/adrs/ADR-0001-sync-delta-cursor.md` — a dated status-history note
  recording that the bootstrap handshake implemented in this milestone
  completes the Option A path ADR-0001 selected. The ADR's decision history
  is not rewritten.
- `CHANGELOG.md` — the bootstrap and the cursor-state change.
- `docs/COMMANDS.md` — only if a command's visible behavior changes.

## 13. Acceptance criteria

1. A client that has synced against a sequence-capable server sends a
   non-zero `last_sync_sequence` on its next sync, and the server answers with
   a non-zero `server_sequence`.
2. The server answers the watermark for a no-cursor request and answers zero
   for a timestamp-only request.
3. The client's persisted state records which cursor the server is serving,
   and distinguishes "the server sent no sequence" from "the sequence did not
   advance".
4. A client that has not bootstrapped never adopts a watermark from a
   timestamp delta.
5. The merge order, the server's upsert rule, the fingerprint, the identifier
   derivation, the synced field set, and the timestamp fallback are unchanged.
6. The protocol remains additive: no field is added, removed, or retyped, and
   `snip-proto` is not republished.
7. `snip-sync` and `snip-it` are version-bumped in one change, with the
   publish order stated.
8. The boundary-row re-delivery behavior for clients that stay on the
   timestamp path is documented.
9. `cargo fmt --all -- --check`, both clippy runs, and the focused sync
   suites pass.

## 14. Stop conditions

The agent must stop and report rather than improvise when:

- the bootstrap cannot be made safe — that is, when it cannot be shown that a
  client which adopts a watermark holds every row the watermark covers;
- making the bootstrap work would require a new protocol field, a change to
  the merge order, a change to the server's upsert, a change to the fingerprint
  or the identifier derivation, or a change to the synced field set;
- the cursor state cannot be persisted additively in the library metadata
  without a schema version or a migration;
- the existing sync suites fail for a reason that indicates a defect deeper
  than the bootstrap;
- a hard dependency is absent.

## 15. Closure evidence required

- The `cargo clippy --workspace --all-targets -- -D warnings` and
  `--all-features` results.
- The focused sync suites' results.
- The bootstrap's closing evidence: a test showing a client that synced
  against a sequence-capable server sends a non-zero `last_sync_sequence` and
  the server answers with a non-zero `server_sequence`.
- The boundary-row re-delivery evidence.
- Evidence that the merge order, the server's upsert, the fingerprint, the
  identifier derivation, and the synced field set are unchanged.
- The version-bump evidence and the publish order.

## 16. Handoff notes

- The working tree carries the uncommitted M001 implementation relative to
  `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`. Preserve it; do not reset
  unrelated work.
- Never run `cargo build`, `cargo test`, or `cargo clippy` while another test
  run is in flight. The integration tests execute `target/debug/snp`; a
  rebuild mid-run replaces that binary and produces false failures.
- `SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on all test commands and must
  never be removed.
- In edition 2024, `std::env::set_var` requires `unsafe`.
- The server's `snippets.id` is a global primary key, and a cross-library
  push with the same content-derived id writes nothing. Do not change that
  rule while touching the cursor.
- The client's merge order is `(updated_at, device_id, SHA-256(synced
  fields))`, and deletion beats live content. Do not change it.
- The `visible_at` column is the timestamp cursor and the `sequence` column
  is the write-sequence cursor. They are different columns with different
  jobs; do not conflate them.
- The M001 closure record is immutable except for factual corrections.
