# Sync Cursor Protocol Milestone 001 — Closure Status

Status: conditionally closed

Source implementation plan:

- `plans/implementation/sync-cursor-protocol/001-monotonic-per-library-sequence-cursor.md`

Source subsystem roadmap:

- `plans/subsystems/sync-cursor-protocol-roadmap.md`

Source decision record:

- `plans/adrs/ADR-0001-sync-delta-cursor.md` (Option A, status `accepted`)

Repository baseline reviewed: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Implementation commits or pull requests:

- Implementation commit `IMPLEMENTATION_COMMIT_SHA`, relative to baseline
  `dca5d31`. Change set: 1 proto file, 1 regenerated stub,
  2 server modules, 4 client modules, 3 crate manifests, 1 lockfile, 4
  documents.

## 1. Executive finding

The milestone's infrastructure boundary is complete, but **the exact cursor
is not yet reachable in production**, so the milestone is conditionally
closed rather than closed.

Everything the plan specified is implemented and verified: the server assigns
a strictly increasing per-library write sequence inside the write
transaction, the delta filter has a sequence form, the response carries the
watermark, the client stores and sends the cursor, the migration backfills
it in the pre-migration order, the three crate versions moved together, and
the checked-in stubs were regenerated rather than hand-edited.

The condition is a **bootstrap gap that the plan itself specifies**:
the server returns a sequence only when the request carried a non-zero one
(§5 of the plan), and the client stores a sequence only when the response
carries a non-zero one (§6 of the plan). A client that starts with no stored
sequence therefore sends zero on every request, the server keeps serving the
`visible_at` (timestamp) filter, the server keeps answering with a zero
sequence, and the client never acquires one. The sequence path is therefore
correct but dormant: no client can enter it, and the second-granularity wart
that ADR-0001 was written to remove still applies at runtime.

The condition is safe, not lossy: a client that never enters the sequence
path behaves exactly as it did before this change, in both version-skew
directions. The condition is closed by M002, whose implementation plan must
add the bootstrap.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| The server assigns the sequence; a client never supplies it | `snip-sync/src/db.rs` — `bump_library_sequence` runs inside the write transaction; no request field carries a sequence *value* to be stored | pass | `SyncRequest.last_sync_sequence` is a **cursor**, not an assignment. |
| The counter bump and the row write share one transaction | The bump is called on the same sqlx transaction as the row write | pass | A rolled-back write consumes no sequence value. |
| A concurrent counter race fails loudly | Unique index on `(library_id, sequence)` | pass | Two concurrent pushes to one library get distinct sequence values; a race would violate the index rather than silently hide a row. |
| The migration preserves the pre-migration `updated_at` ordering | `migrate_snippet_sequence` backfills the sequence so the backfilled ordering matches the pre-migration ordering; a migrated database selects the same rows as an unmigrated one for the first sync | pass | Asserted by a server test. |
| The delta filter has a sequence form and a `visible_at` fallback | `DeltaCursor` in `snip-sync/src/db.rs`: a non-zero `last_sync_sequence` selects `sequence >= ?`; otherwise the existing `visible_at >= ?` filter applies unchanged | pass | An old client that sends no sequence gets exactly the rows the timestamp filter would select. |
| The server returns the watermark only when the request carried one | `SyncResponse.server_sequence` is zero unless the request carried a non-zero sequence | pass | See the §1 condition: this rule is what makes the cursor dormant. |
| The client stores, sends, and advances the cursor | `LibraryMeta.last_sync_sequence` (`src/library/model.rs`); `LibraryManager::update_sync_cursors` with a `SequenceCursor` enum (`Advance` / `Keep` / `Reset`); the cursor is read and advanced in `src/sync_commands.rs`; every request site in `src/sync.rs` carries it | pass | A zero answer never clears or moves the stored cursor. |
| The merge order and the server's upsert rule are unchanged | `src/sync_commands.rs` and `snip-sync/src/db.rs` upsert were not rewritten; `sync_multibatch` (6 tests, exact-count assertions) and `sync_contracts` (19) pass | pass | `(updated_at, device_id, SHA-256(synced fields))` and the server's SQL last-write-wins upsert are untouched. |
| The identifier derivation and the `snippets.id` scope guard are unchanged | Not touched; `identity_contract` (14 tests) passes | pass | |
| The stub is regenerated, not hand-edited | The stub was regenerated with tonic-prost-build 0.14.6 / prost-build 0.14.4 / protoc 3.21.12 in a throwaway crate outside the repository, **before** and **after** the proto edit. The pre-edit regeneration matched the checked-in file except for two stale doc comments — no generator-version churn. The post-edit diff is exactly the two new fields plus those two now-corrected comments, and the regenerated file was copied in verbatim. | pass | The throwaway crate was deleted. |
| The three crate versions move together | `snip-proto` 0.1.3 → 0.1.4, `snip-sync` 0.1.6 → 0.1.7, `snip-it` 1.3.9 → 1.4.0, in one change; publish order `snip-proto` → `snip-sync` → `snip-it` preserved | pass | `Cargo.lock` refreshed offline. |
| The upload ceilings are unchanged | Client ceiling `DEFAULT_CLIENT_REQUEST_CEILING` and server ceiling `DEFAULT_GRPC_MAX_MESSAGE_SIZE` were not changed; the byte-ceiling measurement now also counts the new cursor fields | pass | |
| The server's HTTP surface is untouched | `snip-sync/src/http.rs` was not modified; `snip_sync_lifetime` passes | pass | |

## 3. Production implementation evidence

**Protocol.** `snip-proto/proto/sync.proto` gains `SyncRequest.last_sync_sequence`
(field 7, `int64`) and `SyncResponse.server_sequence` (field 9, `int64`).
Both are additive; proto3 ignores unknown fields, so an old client against a
new server and a new client against an old server both behave as before.
Zero is the "absent" value on both sides and is documented in the proto
comments.

**Server storage.** `snip-sync/src/db.rs` gains a `sequence` column on
`snippets`, a `library_sequences` counter table, and a unique index on
`(library_id, sequence)`. The counter bump shares the transaction with the
row write. `migrate_snippet_sequence` adds the column and backfills it so
that the backfilled ordering preserves the pre-migration `updated_at`
ordering. The tombstone path assigns a sequence too.

**Server protocol.** The `sync` handler in `snip-sync/src/lib.rs` selects the
delta filter by cursor kind and answers with the sequence watermark when the
request carried one.

**Client storage.** `LibraryMeta.last_sync_sequence` sits alongside
`last_sync`; no client-side storage migration is required.
`LibraryManager::update_sync_cursors` takes a `SequenceCursor`
(`Advance` / `Keep` / `Reset`); a zero-valued server answer maps to `Keep`,
so a server that does not serve the sequence can never clear the stored
cursor.

**Client protocol.** `src/sync.rs` carries the cursor on every request and
reads it from every response; `src/sync_commands.rs` reads the stored cursor
before the request and advances it only when the response carries a non-zero
value.

**Planned but absent behavior (the closure condition).** No client can yet
acquire its first sequence cursor; see §1 and §10.

## 4. Verification executed

### Commands run

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --lib
cargo test --test snip_sync_lifetime
cargo test --test sync_contracts
cargo test --test sync_integration
cargo test --test sync_multibatch --features test-support -- --test-threads=1
cargo test --test identity_contract --features test-support -- --test-threads=1
```

### Results

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass (exit 0, no output) |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass (0 errors, 0 warnings) |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass (0 errors, 0 warnings) |
| `cargo test --workspace --lib` | pass — snip-it 1211 passed / 0 failed / 8 ignored; snip-sync 187 passed / 0 failed |
| `cargo test --test snip_sync_lifetime` | pass — 6 passed / 0 failed / 2 ignored (the 2 ignored are pre-existing baseline ignores) |
| `cargo test --test sync_contracts` | pass — 19 passed / 0 failed |
| `cargo test --test sync_integration` | pass — 6 passed / 0 failed |
| `cargo test --test sync_multibatch --features test-support -- --test-threads=1` | pass — 6 passed / 0 failed |
| `cargo test --test identity_contract --features test-support -- --test-threads=1` | pass — 14 passed / 0 failed |

Server-side coverage added with the change: the watermark query returns the
library's maximum sequence; a write assigns a sequence strictly greater than
the current maximum; two concurrent pushes to one library receive distinct
sequence values; the migration backfill preserves the pre-migration
`updated_at` ordering; an old client (no sequence) receives the rows the
timestamp filter would select; a client with a sequence cursor receives
exactly the rows written after that sequence; a sequence cursor from one
library cannot select another library's rows.

Client-side coverage added with the change: the client sends both cursors;
the client leaves its sequence cursor unchanged on a zero-valued response; a
zero answer never clears the stored cursor.

**Scope note.** `scripts/check.sh` was not run in full in this pass;
verification was scoped to the targets the change can affect, plus the
`snip_sync_lifetime` wire-parity suite, which is the suite that locks the
server's HTTP/gRPC surface. The full suite remains the pre-merge gate.

## 5. Invariant review

| Invariant | Evidence | Result |
|---|---|---|
| The server's delta filter never runs on the client-chosen `updated_at` | The filter runs on `visible_at` or on the server-assigned `sequence`; `updated_at` keeps its separate role | pass |
| The merge order is never role-dependent | `src/sync_commands.rs` merge logic untouched; `sync_multibatch` exact-count assertions pass | pass |
| The server's upsert rule is unchanged | `snip-sync/src/db.rs` upsert untouched | pass |
| The synced field set is unchanged | The protobuf message fields are unchanged except the two additive cursor fields, which are cursor-only and are not part of the fingerprint | pass |
| `snippets.id` remains global with the foreign-scope guard | Untouched; `identity_contract` passes | pass |
| The identifier derivation is deterministic | Untouched | pass |
| Uploads stay byte-bounded | The client ceiling is unchanged; the byte-ceiling measurement now includes the cursor fields | pass |
| A cursor is not a capability | The sequence reveals no content and grants no access the client did not already have | pass |
| No credential is transmitted by this change | Only a counter crosses the wire | pass |
| Both version-skew directions behave as before | Additive proto fields; the fallback filter is the pre-existing filter | pass |

## 6. Failure and recovery review

- **Duplicate delivery / idempotency** — a re-delivered row is handled by the
  pre-existing merge path; the sequence cursor changes which rows are
  *selected*, not how they merge.
- **Cancellation races** — a cancelled push shares the row write's
  transaction, so it consumes no sequence value.
- **Restart** — the counter lives in the database; a server restart resumes
  from the stored value.
- **Partial persistence failure** — the row write and the counter bump share
  a transaction, so they cannot diverge.
- **Stale generation or lease** — not applicable.
- **Contention and resource release** — the unique index is the enforcement
  mechanism for a counter race, not the transaction alone.
- **Malformed or unauthorized input** — a client-supplied sequence is only
  ever a cursor; the server never stores a client-supplied sequence, and a
  sequence from one library cannot select another library's rows (asserted).
- **Bounded event and artifact behavior** — no new runtime system and no new
  background work was introduced.

## 7. Migration and compatibility review

- **Schema migration** — `migrate_snippet_sequence` adds the `sequence`
  column and the `library_sequences` table, backfills the column so the
  pre-migration `updated_at` ordering is preserved, and creates the unique
  index. The migration runs in the server's `connect` path, alongside the
  existing `migrate_snippet_visible_at`.
- **Backward compatibility** — the protocol change is additive in both
  directions. A new client against an old server receives no sequence and
  stays on the timestamp cursor. An old client against a new server sends no
  sequence and receives the pre-existing row set.
- **Protocol negotiation** — implicit through the additive fields; there is
  no version handshake and none was introduced.
- **Configuration behavior** — unchanged; no new configuration key.
- **Rollback limitations** — rolling back to a pre-0.1.4 `snip-proto` leaves
  the `sequence` column and the counter table in the database, where the
  older server ignores them. Rolling back the client leaves
  `last_sync_sequence` in the library metadata, where the older client
  ignores it. Neither rollback loses data.
- **Legacy-path status** — the legacy single-file library migration is
  untouched.

## 8. Security review

- **Authorization enforcement** — unchanged; the cursor is added to an
  already-authorized request.
- **Secret handling** — no credential is read, written, or transmitted by
  this change.
- **Path validation** — unchanged.
- **Privilege boundaries** — unchanged.
- **Denial-of-service bounds** — the delta filter compares an integer instead
  of a timestamp; no new unbounded work was introduced. The unique index
  makes a contended counter race fail fast rather than spin.
- **Redaction** — no error text was added that could carry a credential; the
  `SnipError::Command` display-layer redaction rule is untouched.
- **Audit behavior** — unchanged.
- **Server metrics** — the `/metrics` surface was not changed and does not
  expose snippet content.

## 9. Documentation and operations

Updated: `architecture/proto.md` (the new fields, the field-number
allocation, and the pre-existing stub drift that the regeneration corrected),
`architecture/server.md` (the sequence cursor, the counter table, and the
migration), `architecture/sync.md` (the client-side cursor),
`architecture/overview.md` §7 (the exact-cursor invariant),
`docs/IDENTITY_CONTRACT.md` (the cursor's role and the known limitation),
`CHANGELOG.md` under `### Added` and `### Fixed` (the stub drift correction).

Version-bump requirement recorded: `snip-proto` 0.1.4, `snip-sync` 0.1.7,
`snip-it` 1.4.0, publish order `snip-proto` → `snip-sync` → `snip-it`.

Static guards: none were required; the change adds no module and no
suppression.

Operator diagnostics: the client's stored cursors remain visible in the sync
status output, so an operator can see which cursor the client holds.

Recovery instructions: a client that loses its cursor state falls back to a
full re-scan, which the existing merge path already handles.

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| **medium** | **The sequence cursor cannot bootstrap.** The server returns a sequence only when the request carried a non-zero one, and the client stores one only when the response carries a non-zero one. A client that has never held a sequence sends zero forever, so the server keeps serving the `visible_at` filter and the exact cursor is unreachable at runtime. The plan specifies this rule (§5, §6), so the implementation is faithful — the gap is in the milestone's own design. | The milestone's stated value (an exact delta cursor) is not realized in production. **No data loss and no incorrect behavior**: a client on the dormant path receives exactly the row set it received before the change, and both version-skew directions are unchanged. The second-granularity wart ADR-0001 was written to remove still applies at runtime. | **Closes the conditional status.** M002's implementation plan must add the bootstrap — the mechanism by which a client acquires its first sequence cursor without skipping rows — and must extend the milestone's scope accordingly (the roadmap's M002 deliverable boundary names "record which cursor the server is serving", which is adjacent but does not itself state the bootstrap). The closing evidence is: a test that shows a client which has synced against a sequence-capable server sends a non-zero `last_sync_sequence`, and that the server then answers with a non-zero `server_sequence` and the client stores it. |
| low | The plan's §3 evidence did not state the bootstrap consequence, so the milestone's exit criteria were satisfiable while the capability stayed dormant. | A milestone can be closed against its acceptance criteria and still not deliver its stated value. | Recorded so that M002's plan treats the bootstrap as a first-class deliverable rather than a refinement. |
| low | The checked-in stub carried two stale doc comments unrelated to this change (`GetSnippets.since` and `SyncRequest.local_snippets`). The regeneration corrected them as a side effect. | The stub is now consistent with the `.proto`; the correction is recorded under `### Fixed` in the changelog. | None. |

## 11. Roadmap disposition

Milestone **conditionally closed** with one named condition outstanding: the
sequence cursor cannot bootstrap, so the exact path is not yet exercised in
production (§10). The condition is closed by M002, whose implementation plan
must add the bootstrap and extend the milestone's stated scope to cover it.

M002 is otherwise unblocked and may be planned; its plan must carry the
bootstrap as an explicit deliverable and must not mark the subsystem closed
until the closing evidence named in §10 exists.

## 12. Registry updates

- `plans/registry.md`: the sync-cursor-protocol roadmap moves from `ready` to
  `active`; this milestone moves from `not started` to
  `conditionally closed`, with the named condition recorded in the blockers
  column; the plan row moves from `ready` to `conditionally closed`.
- `plans/subsystems/sync-cursor-protocol-roadmap.md`: §12 milestone status
  table updated — M001 `conditionally closed`, with the closure record
  linked; the milestone's completion note records the bootstrap condition
  and the requirement that M002's plan carry the bootstrap.
- `plans/adrs/ADR-0001-sync-delta-cursor.md`: no change required by this
  milestone; the ADR is already `accepted` and its status history records the
  normalization.
