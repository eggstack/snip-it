# Sync Cursor Protocol Roadmap

Status: active

> **2026-10-22 — M001 conditionally closed.** The milestone's implementation
> plan (`plans/implementation/sync-cursor-protocol/001-monotonic-per-library-sequence-cursor.md`)
> was executed and its closure record is
> `plans/closure/sync-cursor-protocol/001-status.md`. The roadmap changed only
> in its status field and in §12: the M001 objective, deliverable boundary,
> exit conditions, and milestone history are unchanged. **Named condition:** the
> sequence cursor cannot bootstrap — the server returns a sequence only when
> the request carried one, and the client stores one only when the response
> carried one — so no client enters the exact path yet. The behavior is safe
> (a client on the dormant path receives exactly the pre-change row set), but
> the milestone's stated value is not yet realized at runtime. M002's
> implementation plan must add the bootstrap and extend that milestone's
> scope to cover it.

> **2026-10-22 — M002 closed; M001's condition discharged.** The milestone's
> implementation plan
> (`plans/implementation/sync-cursor-protocol/002-cursor-bootstrap-and-precision-convergence.md`)
> was executed and its closure record is
> `plans/closure/sync-cursor-protocol/002-status.md`. M002 adds the bootstrap:
> the server answers the sequence watermark for a request that carries no
> cursor at all — the only request shape that delivers the complete library —
> and the client adopts it, sends it from then on, and records which cursor the
> server is serving in a new additive `sync_cursor_kind` metadata field. No
> protocol field was added, removed, or retyped, so `snip-proto` stays at
> 0.1.4; `snip-sync` is 0.1.8 and `snip-it` is 1.4.1. The M001 objective,
> deliverable boundary, exit conditions, and milestone history are unchanged.

Long-term references:

- `plans/000-long-term-specification.md#4-crate-and-module-ownership`
- `plans/000-long-term-specification.md#6-sync-invariants`
- `plans/000-long-term-specification.md#9-distribution-and-release-invariants`
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules`
- `plans/001-terminology-and-domain-model.md#5-sync-and-execution-terms`
- `plans/002-long-term-roadmap.md#cross-phase-execution-rules`
- `plans/003-planning-process.md`

Related ADRs:

- `plans/adrs/ADR-0001-sync-delta-cursor.md`

## 1. Purpose and ownership boundary

This subsystem owns the sync delta cursor: the value the client uses to ask
the server "what changed since I last synced", and the value the server
hands back to advance that cursor.

It consumes the protobuf schema, the server's database schema and upsert
rule, the client's merge order, and the client/server version-compatibility
contract. It MUST NOT own the client's merge order
(`(updated_at, device_id, SHA-256)`), the server's last-write-wins upsert
rule, the snippet identity contract, the delta cursor's
neighboring lock protocol
(`plans/subsystems/locking-convergence-roadmap.md`), or the release
publication pipeline (`plans/subsystems/distribution-release-roadmap.md`).

The roadmap exists because implementing ADR-0001's Option A requires a
coordinated change across three published crates with a version bump in
each, and because that change is the only remaining registered decision
that crosses the client/server protocol boundary.

## 2. Work classification

### Invariants

- A row written to the server is always delivered to every client that
  syncs after the write, regardless of the timestamp the client chose.
- The delta cursor is monotone. It never moves backwards, so the
  `>=`-versus-`>` ambiguity on boundary rows does not select or omit data.
- The client's merge order is unchanged: `(updated_at, device_id,
  SHA-256(synced fields))`, and deletion still beats live content in the
  client merge.
- The server's upsert rule is unchanged: SQL last-write-wins on
  `(updated_at, device_id, command, description)`, with `deleted` an
  ordinary column. The cursor change must not make the server's rule
  stronger or weaker.
- A cursor reveals nothing about snippet content.
- Proto3 additive fields keep both version-skew directions working: an old
  client against a new server, and a new client against an old server.
- A proto change requires bumping the `snip-proto` version and the versions
  of both dependents in the same change.

### Capabilities

- The delta cursor becomes a server-assigned write counter, so second
  granularity stops mattering and the boundary-row re-delivery wart is
  removed.
- The client keeps its `updated_at` logical-clock semantics and its merge
  order exactly as they are.

### Infrastructure

- A per-library write counter maintained transactionally with the row write.
- A database migration that adds the sequence column and backfills it
  without changing which rows a cursor selects.

### Polish

- The two cursor columns are clearly named and separately documented, so
  that "the timestamp cursor" and "the sequence cursor" are never confused
  in code review.

## 3. Non-goals

- No change to the client's merge order or the server's upsert rule.
- No change to snippet identity, the fingerprint field set, or the
  deterministic identifier derivation.
- No change to the local-only field split: `output`, `folders`, and
  `favorite` remain unsynced.
- No removal of the `visible_at` column or of the timestamp cursor. Both
  remain, because old clients still send the timestamp cursor.
- No change to the gRPC service surface beyond additive fields.
- No change to the HTTP surface (`/health`, `/metrics`) of `snip-sync`.
- No new crate, no protocol framework, and no streaming or long-polling
  sync mode.
- No re-encryption, key-rotation, or credential-store change.
- No change to the publish order `snip-proto` → `snip-sync` → `snip-it`.

## 4. Current state

The sync delta cursor is a timestamp. The server's `snippets` table carries
two timestamp columns with distinct roles, established by ADR-0001's
amendment:

- `updated_at` is the **client's logical modification time**, stored
  verbatim. The server's last-write-wins comparison and the client's merge
  order both read it.
- `visible_at` is the **delta cursor position** of the row. It is clamped to
  the library's current watermark on every write and never moves backwards,
  so the delta filter `visible_at >= since` and the cursor handed to the
  client (`MAX(visible_at)`, returned as `server_timestamp`) are monotone by
  construction.

That change closed the data-loss defect: a row is no longer unreachable
because a client chose a timestamp below the library maximum. The existing
server test `test_insert_below_watermark_is_still_delivered` proves it.

What the amendment deliberately did not remove is that the cursor is still a
timestamp. Two consequences remain:

1. The `>=` filter re-delivers boundary rows. A client whose cursor equals
   a row's `visible_at` receives that row again on every subsequent sync.
   This is redundant work, not data loss, and the client merge treats a
   re-delivered row as idempotent.
2. Second granularity still matters. Two writes within the same second
   share a `visible_at` value, so the boundary between "rows the client has
   seen" and "rows the client has not" is not exact.

ADR-0001 records Option A — a server-assigned, strictly increasing
per-library write sequence — as the accepted decision that removes both
consequences, together with the design for the additive protocol change,
the server-side counter, the migration, and the version bumps. It records
that the decision is not yet implemented and that it must land as its own
change with its own version bump rather than being folded into an unrelated
batch.

This is the only open decision in `plans/registry.md`. Everything else the
registry lists is closed, and the six previously registered subsystem
roadmaps are all closed, so the protocol change has no other workstream
sharing its release boundary.

## 5. Target architecture

One server-assigned per-library write sequence, assigned inside the same
transaction as the row write, stored on the row alongside the two existing
timestamp columns.

The client stores both cursors and sends both. The server uses the sequence
cursor when the request carries a non-zero one and falls back to the
timestamp filter otherwise, so a new client against an old server behaves
exactly as it does today.

The sequence replaces the timestamp as the *precision* of the cursor while
the timestamp remains the *position* of the cursor, and both are reported so
that the client's stored state is never ambiguous.

## 6. Dependency graph

```text
M001 Monotonic per-library sequence cursor
    |
    `--> M002 Cursor-precision convergence (hard: M001)
```

M001 alone delivers the accepted decision. M002 is a follow-up that removes
the residual timestamp-precision behavior once the sequence is proven in
production; it is not required for the ADR to be satisfied.

## 7. Milestones

### M001 — Monotonic per-library sequence cursor

Class: invariant

Objective: implement ADR-0001's Option A — a server-assigned, strictly
increasing per-library write sequence that the delta filter and the client
cursor use, while the timestamp cursor continues to work unchanged for
clients that do not send one.

Dependencies: none.

Deliverable boundary: a new sequence column on the server's `snippets`
table plus a per-library counter; a sequence assigned transactionally with
each row write; additive sequence fields on the sync request and response;
the client storing, sending, and advancing both cursors; the server
selecting on the sequence when the request carries one and falling back to
the timestamp filter otherwise; a migration that adds the column and
backfills it; the version bumps for `snip-proto` and both dependents.

User or operator value: the cursor becomes exact, so the delta filter stops
re-delivering boundary rows and stops depending on second granularity, and
the durability of a client's view no longer rests on timestamp arithmetic.

Exit conditions: the sequence is unique per library, enforced by a unique
index so that a concurrent counter race fails loudly rather than silently
reintroducing invisibility; a row pushed with an `updated_at` below the
library maximum is delivered to a third client syncing from the current
sequence cursor; the client's merge order and the server's upsert rule are
unchanged, proven by the existing conflict-resolution tests; an old client
that sends no sequence cursor still receives the rows the timestamp filter
would have selected; a migrated database selects the same rows as an
unmigrated one for the first sync; `snip-proto`, `snip-sync`, and `snip-it`
versions are bumped with the proto change; `bash scripts/check.sh` passes.

Deferred work: cursor-precision convergence (M002).

Implementation plan:

- `plans/implementation/sync-cursor-protocol/001-monotonic-per-library-sequence-cursor.md`

Closure record: `plans/closure/sync-cursor-protocol/001-status.md`
(conditionally closed).

Predecessor flat plan: none.

### M002 — Cursor-precision convergence

Class: polish

Objective: retire the timestamp cursor's residual precision behavior once
the sequence cursor is proven, by making the sequence the only cursor a
current client relies on and by documenting the boundary-row re-delivery
that remains for old clients.

Dependencies: hard M001.

Deliverable boundary: the client's cursor state records which cursor the
server is actually serving, so a client can distinguish "the server sent
no sequence" from "the sequence did not advance"; the boundary-row
re-delivery behavior for old clients is documented rather than silently
relied upon; no removal of the timestamp cursor in this milestone.

User or operator value: operators can tell from a client's state whether it
is on the exact cursor path or the compatibility path.

Exit conditions: the client's stored cursor state distinguishes the two
cursor sources; `docs/IDENTITY_CONTRACT.md` and `architecture/sync.md`
describe the boundary-row behavior for old clients; no regression in the
exact-count sync assertions.

Deferred work: none within this subsystem.

Implementation plan:

- `plans/implementation/sync-cursor-protocol/002-cursor-bootstrap-and-precision-convergence.md`

Closure record:

- `plans/closure/sync-cursor-protocol/002-status.md`

Predecessor flat plan: none.

## 8. Cross-cutting requirements

### Storage and migration

The server's `snippets` table gains a sequence column, and the server gains
a per-library counter. The migration adds the column and backfills it so
that the backfilled ordering preserves the pre-migration `updated_at`
ordering, which means a migrated database selects the same rows as an
unmigrated one for the first sync. A unique index on
`(library_id, sequence)` turns a concurrent-counter race into a loud
failure. The counter is updated in the same transaction as the row write;
if it is not, two concurrent writes can share a sequence value and one of
them becomes invisible again, which is the original defect in a narrower
window.

No client-side storage migration is required: the client stores the new
cursor alongside the existing one.

### Protocol and compatibility

The request carries a new field alongside `last_sync_timestamp`; the
response carries a new field alongside `server_timestamp`. Proto3 ignores
unknown fields, so an old client against a new server and a new client
against an old server both behave exactly as before. The server uses the
sequence cursor only when the request carries a non-zero one.

A proto change requires bumping the `snip-proto` version and the versions
of `snip-sync` and `snip-it` in the same change, and the publish order
`snip-proto` → `snip-sync` → `snip-it` is preserved.

### Security and authorization

The cursor reveals nothing about content; it is a write counter. A client
cannot use the cursor to skip rows it has not seen: the counter advances on
every write and the filter selects every row written after the cursor value
the client holds. No credential is transmitted by this change. The cursor
is not a capability and grants no access the client did not already have.

### Concurrency, cancellation, and recovery

The sequence is assigned inside the same transaction as the row write.
Concurrent pushes to the same library must not share a sequence value, and
the unique index is the enforcement mechanism, not the transaction alone.
A cancelled or failed push must not consume a sequence value, because a
skipped value is harmless but a reused one is not; the counter increment and
the row write share a transaction so that they cannot diverge.

A client that loses its cursor state falls back to a full re-scan, which the
existing merge path already handles; the sequence cursor does not introduce
a new recovery mode.

### Observability and audit

The server's metrics surface (`/metrics`) may report the cursor kind in use,
but must not expose snippet content. Client-side sync status continues to
report the stored cursors so an operator can see which cursor path is
active.

### Performance and resource use

The delta filter compares an integer sequence instead of a timestamp. No
new runtime systems and no new background work are introduced.

### Documentation and operations

`docs/IDENTITY_CONTRACT.md` (known limitations), `docs/COMMANDS.md` if the
sync status output changes, `architecture/sync.md`, `architecture/proto.md`,
`architecture/server.md`, `architecture/status.md`, `CHANGELOG.md`, and
`RELEASING.md` are updated with the change. The version-bump requirement is
recorded in the closure evidence.

## 9. Verification strategy

A server test that pushes a row with an `updated_at` below the library
maximum and proves a third client, syncing from the current sequence
cursor, receives it; a server test that a second push with a newer
`updated_at` still wins the last-write-wins comparison; a server test that
an old client sending no sequence cursor receives the rows the timestamp
filter would have selected; a concurrency test that two simultaneous pushes
to one library receive distinct sequence values; a migration test that a
migrated database's backfilled ordering matches the pre-migration ordering;
a client test that the merge order is unchanged, exercised through the
existing multi-batch and conflict-resolution suites; the `snip_sync_lifetime`
wire-parity suite; `scripts/check.sh`.

## 10. Risks and decision points

- Three crate versions move together. A partial publish leaves a client
  that sends a sequence to a server that ignores it, which the fallback
  path handles, but a client that expects a sequence from a server that
  does not send one must also tolerate its absence. Decision: the client
  treats a missing sequence as "use the timestamp cursor", which the
  protocol's additive design already permits.
- The counter is a second source of truth next to `visible_at`. Decision:
  the sequence is authoritative for precision once the server returns one;
  `visible_at` remains for old clients and for the fallback filter.
- ADR-0001's status word was outside the ADR status lifecycle. Decision:
  normalize it to `accepted` when this roadmap is registered, with the
  transition recorded in the ADR's status history. The ADR body is not
  rewritten.
- No further decision in this subsystem requires an ADR. A decision that
  would change the merge order, the upsert rule, or the identity contract is
  an ADR trigger and stops the milestone.

## 11. Completion definition

This workstream closes when M001 and M002 each have accepted closure
evidence showing that the delta cursor is a server-assigned monotone
sequence, that both version-skew directions behave as before, that the
client's merge order and the server's upsert rule are unchanged, that the
three crate versions moved together, and that no unresolved
medium-or-higher finding remains.

## 12. Milestone status

| Milestone | Status | Implementation plan | Closure record | Blockers |
|---|---|---|---|---|
| M001 monotonic per-library sequence cursor | conditionally closed (condition discharged by M002) | `plans/implementation/sync-cursor-protocol/001-monotonic-per-library-sequence-cursor.md` | `plans/closure/sync-cursor-protocol/001-status.md` | Named condition — the sequence cursor cannot bootstrap, so the exact path is not exercised in production — **discharged on 2026-10-22 by M002**, which supplies the closing evidence. The M001 closure record is left immutable. |
| M002 cursor-precision convergence | closed | `plans/implementation/sync-cursor-protocol/002-cursor-bootstrap-and-precision-convergence.md` | `plans/closure/sync-cursor-protocol/002-status.md` | — |
