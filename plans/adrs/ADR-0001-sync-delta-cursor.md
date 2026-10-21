# ADR-0001: Monotonic per-library sequence as the sync delta cursor

Status: proposed

Date: 2026-10-21

Decision owners: project maintainers

Related specification sections:

- `plans/000-long-term-specification.md#6-sync-invariants` — sync invariants
- `plans/001-terminology-and-domain-model.md#5-sync-and-execution-terms` — sync and execution terms
- `plans/001-terminology-and-domain-model.md#synced-fields` — the synced/local-only field split

Affected subsystem roadmaps:

- `plans/subsystems/cli-library-sync-consolidation-roadmap.md` (closed; this decision reopens the sync-cursor contract only)

## Context

The sync server's delta filter is `updated_at >= since`
(`snip-sync/src/db.rs`), and the cursor the client receives is
`MAX(updated_at)` over the whole library (`snip-sync/src/db.rs`,
`get_latest_timestamp`; returned as `server_timestamp` in `SyncResponse`).

`updated_at` is a **logical** clock: it is the client-assigned modification
time of a snippet, and it is the same value the client uses as the first
component of its own merge order `(updated_at, device_id, SHA-256)`. The
server stores it verbatim, and the server's own upsert rule is SQL
last-write-wins on `(updated_at, device_id, command, description)`.

The server's delta cursor therefore has to advance on every write for a row
to stay reachable — but it is derived from a value the client controls, and a
client is free to send a value lower than the current maximum. The server
accepts arbitrarily old timestamps: `validate_snippet` rejects only
timestamps *ahead* of server time by more than 300 s.

The resulting defect:

1. Device A pushes with `updated_at = 100`. The library max becomes 100 and
   A's watermark becomes 100.
2. Device B — with a clock behind, or restored from an old backup — pushes
   snippet Z with `updated_at = 50`. The server accepts and stores it. The
   library max stays 100.
3. Device A syncs with `last_sync_timestamp = 100`. The filter
   `updated_at >= 100` excludes Z, so **Z is never delivered**. The same is
   true for every other client whose watermark is at or above 50.

Z now exists only on the server and on device B. The client persists and
only advances `last_sync` (`src/library/manager.rs`, `src/library/model.rs`)
and never resets it to 0 for a full re-scan, so no client pulls Z again. If
B is wiped, Z is unreachable through the normal sync path.

Clock skew on one device — laptops with wrong clocks, VM restores, timezone
bugs — is a common real-world condition. The consequence is silent, permanent
data unavailability across devices.

The defect is currently *documented* rather than fixed: it is recorded in
`docs/IDENTITY_CONTRACT.md` under "Known limitations", and the audit that
found it classified it as the one finding in the set that requires a
protocol/schema change rather than a patch.

## Decision drivers

- A row must never become unreachable because of a value the client chose.
- The fix must not change the client's merge order, which is a published
  contract (`docs/IDENTITY_CONTRACT.md`, `AGENTS.md`).
- The fix must not change the server's upsert rule in a way that lets a stale
  write overwrite a newer one.
- Old clients must keep working against a new server, and a new client must
  keep working against an old server.
- The delta cursor must be monotone, so `>=` vs `>` on second granularity
  stops mattering.

## Considered options

### Option A — Monotonic per-library sequence (recommended)

Add a server-assigned, strictly increasing per-library write sequence. The
row carries the sequence it was written at; the delta filter compares that
sequence instead of `updated_at`.

- **Benefits.** The cursor is monotone by construction, so every write is
  visible to every client regardless of the timestamps involved. The
  client's `updated_at` keeps its meaning as the logical modification time
  and the client's merge order is untouched. The second-granularity `>=`
  workaround becomes unnecessary.
- **Costs.** A schema change on the server (`snippets` gains a sequence
  column, plus a per-library counter) and a protocol change (the request
  carries the sequence cursor; the response returns the next cursor).
- **Failure modes.** The sequence is a second source of truth next to
  `updated_at`; it must be assigned inside the same transaction as the
  write, or a concurrent write could take the same sequence value.

### Option B — Clamp the stored `updated_at` to the library max

Store `updated_at = max(pushed, library_max)` and keep the pushed value in a
separate column.

- **Benefits.** No protocol change; the row becomes visible immediately.
- **Costs.** It changes the value the server's LWW comparison reads. With a
  single stored column the comparison must then run on the clamped value, so
  a stale push can be promoted above the row it targets and overwrite
  newer content. Keeping the original in a second column reintroduces the
  schema change of Option A anyway, and adds a second timestamp semantics
  to reason about.
- **Failure modes.** As above: the clamp turns "invisible" into "silently
  rejected" for every later push of the same row. It trades one data-loss
  mode for another.

### Option C — Reject a push whose `updated_at` is behind the library max

Mirror the existing clock-skew rejection (which already covers timestamps
ahead of server time) to cover timestamps behind it.

- **Benefits.** No schema or protocol change; silent data loss becomes an
  explicit, actionable error.
- **Costs.** The pushing client must re-stamp and retry, so the client needs
  a change too, and the error path has to be surfaced well — otherwise the
  client retries a permanently-rejected push.
- **Failure modes.** A device whose clock is permanently behind can never
  sync until its clock is fixed.

### Option D — Widen the delta window

Filter `updated_at >= since OR updated_at >= library_max - WINDOW`.

- **Benefits.** No schema or protocol change.
- **Costs.** Every sync re-sends the whole recent window. Does not fix skew
  larger than the window.
- **Failure modes.** Silent, partial protection only.

## Decision

**Option A — a monotonic per-library sequence — is proposed.** It is the only
option that closes the defect without changing the client's merge order or
the server's last-write-wins rule, and it removes the second-granularity
workaround from the delta filter.

The decision is **not yet implemented**. It is recorded here so the trade-off
is not re-derived, and so the implementation lands as its own change with its
own version bump rather than being folded into an unrelated batch.

## Consequences

### Positive

- A write is never invisible because of a client-chosen timestamp.
- The client keeps its own `updated_at` semantics; the merge contract in
  `docs/IDENTITY_CONTRACT.md` and `AGENTS.md` does not change.
- The `>=` vs `>` ambiguity on second granularity disappears.

### Negative

- The server gains a second ordering column and a per-library counter that
  must be maintained transactionally.
- The sync protocol gains a cursor field, and `snip-proto` gains a version
  bump (with `snip-sync` and `snip-it` bumped in the same change).

### Neutral or deferred

- The server's LWW rule on `(updated_at, device_id, command, description)`
  stays exactly as it is. It remains weaker than the client-merge rule; see
  `architecture/overview.md` § 7.

## Compatibility and migration

The sequence cursor must be additive so that both directions keep working
across a version skew:

- The request carries a **new** field alongside the existing
  `last_sync_timestamp`; the response carries a **new** field alongside
  `server_timestamp`. Proto3 ignores unknown fields, so an old client
  talking to a new server, and a new client talking to an old server, both
  behave exactly as before.
- The server uses the sequence cursor only when the request carries a
  non-zero one; otherwise it falls back to the existing
  `updated_at >= since` filter.
- The client stores the sequence cursor alongside the timestamp and sends
  both; it uses the timestamp cursor until the server starts returning a
  sequence.
- Existing server databases need a migration that adds the column and
  backfills it. Backfilled rows get sequence values that preserve the
  current `updated_at` ordering, so a migrated database behaves like an
  unmigrated one for the first sync.

## Security and reliability implications

- The cursor reveals nothing about content; it is a write counter.
- A client cannot use the cursor to skip rows it has not seen: the counter
  advances on every write, and the filter selects every row written after
  the cursor value the client holds.
- The per-library counter must be updated in the same transaction as the
  row write. If it is not, two concurrent writes can share a sequence value
  and one of them becomes invisible again — the same defect in a narrower
  window. A unique index on `(library_id, seq)` turns that into a loud
  failure rather than a silent one.

## Verification

- A test that pushes a row with an `updated_at` below the library max and
  proves a third client, syncing with the current cursor, receives it.
- A test that a second push of the same row with a *newer* `updated_at`
  still wins the LWW comparison (the sequence change must not have altered
  conflict resolution).
- A test that an old client (no sequence cursor) still receives the rows the
  timestamp filter would have selected.
- A migration test that an existing database is upgraded and the backfilled
  ordering matches the pre-migration `updated_at` ordering.

## Supersession

None.
