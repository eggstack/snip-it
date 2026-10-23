# Sync Cursor Protocol Milestone 002 — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/sync-cursor-protocol/002-cursor-bootstrap-and-precision-convergence.md`

Source subsystem roadmap:

- `plans/subsystems/sync-cursor-protocol-roadmap.md#m002--cursor-precision-convergence`

Repository baseline reviewed: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Implementation commits or pull requests:

- Implementation commit `IMPLEMENTATION_COMMIT_SHA`, relative to baseline
  `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`.

## 1. Executive finding

The milestone's boundary is complete, and it **discharges the named condition**
that M001's closure record attached to its conditional status.

M001 shipped the server-assigned write-sequence cursor — a strictly
increasing, per-library value the server assigns on every write, used by the
delta filter — but the path was unreachable. The server answered
`server_sequence` only when the request carried a non-zero
`last_sync_sequence`, and the client stored a sequence only when the response
carried one, so a client sent zero forever and the exact cursor was never
used. M001 accepted this as safe and recorded the gap as a named condition
with its closing evidence.

M002 implements the bootstrap that closes it. The server now answers the
sequence watermark for a request that carries **no cursor at all** — the
bootstrap full sync, which is the only request shape that delivers the
complete library and therefore the only one whose watermark a client may
adopt. The client adopts that watermark, sends it from then on, and records
which cursor the server is serving in a new persisted field, so it can tell
"the server sent no sequence" apart from "the sequence did not advance".

The protocol itself did not change: **no field was added, removed, or
retyped**, so `snip-proto` stays at 0.1.4 and is not republished.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| A client that synced against a sequence-capable server sends a non-zero `last_sync_sequence`, and the server answers with a non-zero `server_sequence` | `tests/sync_integration.rs` — `test_a_bootstrapped_client_sends_a_non_zero_sequence_cursor` | pass | This is the closing evidence M001's closure record named. |
| The server answers the watermark for a cursorless request | `snip-sync/src/lib.rs` — `test_sync_without_any_cursor_bootstraps_the_sequence_watermark`; `tests/sync_integration.rs` — `test_a_cursorless_request_bootstraps_the_sequence_watermark` | pass | The cursorless request is the bootstrap. |
| The server answers zero for a timestamp-only request | `snip-sync/src/lib.rs` — `test_sync_without_a_sequence_cursor_serves_the_timestamp_rows` (now sends a non-zero timestamp cursor); `tests/sync_integration.rs` — `test_a_timestamp_only_request_is_answered_with_no_sequence` | pass | A delta does not deliver the library's whole history, so its watermark is not adoptable. |
| The client records which cursor the server is serving | `src/library/model.rs` — `SyncCursorKind` (`unknown` / `timestamp` / `sequence`); `src/library/model.rs` — `LibraryMeta.sync_cursor_kind` | pass | Additive, `#[serde(default)]`. |
| "The server sent no sequence" is distinguishable from "the sequence did not advance" | `src/sync_commands.rs` — `apply_server_sequence`; unit tests `test_a_zero_valued_response_sequence_leaves_the_sequence_cursor_unchanged` and `test_a_non_zero_response_sequence_advances_the_sequence_cursor` | pass | Zero drops the client back to the timestamp path and clears the stored cursor; an unchanged value leaves the stored cursor alone. |
| The client's request shape follows the recorded kind | `src/sync_commands.rs` — `request_cursors`; unit test `test_the_request_shape_follows_the_recorded_cursor_kind` | pass | `unknown` → full sync, `sequence` → sequence delta, `timestamp` → timestamp delta. |
| A push-only sync never adopts or advances a sequence cursor | `src/sync_commands.rs` — the push branch stores `SequenceCursor::Keep`; unit test `test_a_push_only_sync_never_adopts_or_advances_the_sequence_cursor` | pass | Such a sync discards the server's rows, so the watermark covers rows the client never received. |
| The reset paths return the recorded kind to `unknown` | `src/library/manager.rs` — `relink_server_library`, `unlink_server_library`; `src/sync_commands.rs` — the server-library re-created recovery path; unit tests `test_the_sequence_cursor_is_cleared_for_a_recreated_library` and `test_a_relink_clears_the_sequence_cursor_and_the_cursor_kind` | pass | The next pulling sync bootstraps against the new library. |
| The boundary row is re-delivered (the bootstrap is safe) | `snip-sync/src/lib.rs` — `test_sync_bootstrap_on_an_empty_library_reports_a_sequence` (the first row written after the bootstrap is still delivered); the inclusive `sequence >=` filter in `snip-sync/src/db.rs` | pass | A row written between the bootstrap's page read and its watermark read carries the counter value the watermark reports, so the inclusive filter re-delivers it. |
| An empty library still reports a sequence | `snip-sync/src/lib.rs` — `test_sync_bootstrap_on_an_empty_library_reports_a_sequence` | pass | The reported position is the one before the library's first row, so zero never has to mean both "no sequence" and "nothing issued". |
| The merge order, the server's upsert, the fingerprint, the identifier derivation, and the synced field set are unchanged | `git diff --stat` — `src/sync_commands.rs`'s merge and fingerprint code unchanged; `snip-sync/src/db.rs` unchanged; `src/library/persistence.rs` unchanged | pass | The change is confined to the cursor plumbing. |
| The protocol remains additive | `snip-proto/proto/sync.proto` and `snip-proto/src/snip_proto.rs` untouched; `snip-proto` stays 0.1.4 | pass | No regeneration, no field change, no republish. |
| The coordinated version bumps | `snip-sync` 0.1.7 → 0.1.8; `snip-it` 1.4.0 → 1.4.1; `Cargo.toml`'s `snip-sync` dependency requirement updated; `Cargo.lock` refreshed offline | pass | `snip-proto` is not republished. |
| The client state survives a save/load cycle and defaults correctly | `LibraryMeta.sync_cursor_kind` is `#[serde(default)]`; the library file loads unchanged | pass | No migration, no rewrite on load. |
| The documented cursor contract | `architecture/sync.md`, `architecture/server.md`, `docs/IDENTITY_CONTRACT.md`, and a dated note in `plans/adrs/ADR-0001-sync-delta-cursor.md` | pass | The ADR's decision history is unchanged. |
| The full Linux CI gate | `bash scripts/check.sh` | **not run** | Deliberately scoped out for this milestone; it remains the pre-merge gate. See §4. |

## 3. Production implementation evidence

### The server's answer rule (`snip-sync/src/lib.rs`)

The sync handler now distinguishes three request shapes using the two existing
protocol fields, and answers the sequence watermark in exactly two of them:

| `last_sync_sequence` | `last_sync_timestamp` | Shape | Server filter | Server answer |
|---|---|---|---|---|
| non-zero | any | sequence delta | `sequence >= cursor` | the library's sequence watermark |
| zero | non-zero | timestamp delta | `visible_at >= timestamp` | **zero** |
| zero | zero | **full sync** (the bootstrap) | `visible_at >= 0` (every row) | the library's sequence watermark |

The only change to the rule is the third row. While the response is
paginated the server still echoes the request's own sequence cursor and does
not advance the watermark, unchanged from M001.

The watermark is reported as **at least 1**. In the protocol zero means "no
sequence", so an empty library cannot report zero without being
indistinguishable from a server that does not serve the sequence at all. The
reported position is the one before the library's first row, and the
inclusive `sequence >=` filter keeps that first row deliverable — proven by
`test_sync_bootstrap_on_an_empty_library_reports_a_sequence`.

### The client's cursor state (`src/library/model.rs`, `src/library/manager.rs`)

`LibraryMeta` gained one additive field:

```rust
#[serde(default)]
pub sync_cursor_kind: SyncCursorKind,
```

with three values — `unknown` (not probed), `timestamp` (the server does not
serve a sequence), `sequence` (the client holds a sequence cursor) —
serialized lowercase. A library file written before the field existed loads
as `unknown`; no migration runs and no file is rewritten on load.

`LibraryManager::update_sync_cursors` now takes the recorded kind alongside
the sequence cursor, and gained `LibraryManager::sync_cursor_kind` as the
read side. `relink_server_library` and `unlink_server_library` clear the
stored sequence cursor and return the recorded kind to `unknown`, because a
relink points the library at a different server-side library whose sequence
space does not apply.

### The client's state machine (`src/sync_commands.rs`)

`request_cursors` chooses the request shape from the recorded kind and
whether the sync pulls. `apply_server_sequence` interprets the server's
answer:

| Recorded state | Request | `server_sequence == 0` | `server_sequence != 0` |
|---|---|---|---|
| `unknown` | full sync | record `timestamp` | adopt the watermark → record `sequence` |
| `timestamp` | timestamp delta | stays `timestamp` | stays `timestamp` |
| `sequence` | sequence delta | record `timestamp`, drop the stored cursor | advance the stored cursor |

A push-only sync is excluded from the transition: it discards the server's
rows, so it neither bootstraps nor touches the sequence cursor. Its stored
timestamp cursor still advances, which is the pre-existing behavior.

### Deliberately unchanged

- The client merge order `(updated_at, device_id, SHA-256(synced fields))`
  and the rule that deletion beats live content.
- The server's last-write-wins upsert and its treatment of `deleted` as an
  ordinary column.
- The fingerprint, the identifier derivation, and the synced field set.
- The timestamp cursor, which remains the fallback for clients that send no
  sequence cursor.
- The protocol: no field added, removed, or retyped.

## 4. Verification executed

### Commands run

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --lib
cargo test -p snip-sync --lib
cargo test --test sync_integration --features test-support
cargo test --test sync_contracts --features test-support
cargo test --test sync_multibatch --features test-support -- --test-threads=1
cargo test --test identity_contract --features test-support
cargo test --test local_contracts --features test-support
cargo test --test selector_search_parity --features test-support
```

### Results

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass (exit 0, no diff) |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass — 0 errors, 0 warnings |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass — 0 errors, 0 warnings |
| `cargo test --workspace --lib` | pass — `snip-it` 1219 passed / 0 failed / 8 ignored; `snip-proto` 0; `snip-sync` 189 passed / 0 failed |
| `cargo test -p snip-sync --lib` | pass — 189 passed / 0 failed |
| `cargo test --test sync_integration --features test-support` | pass — 9 passed / 0 failed (3 new) |
| `cargo test --test sync_contracts --features test-support` | pass — 19 passed / 0 failed |
| `cargo test --test sync_multibatch --features test-support -- --test-threads=1` | pass |
| `cargo test --test identity_contract --features test-support` | pass |
| `cargo test --test local_contracts --features test-support` | pass — 23 passed / 0 failed |
| `cargo test --test selector_search_parity --features test-support` | pass — 29 passed / 0 failed |

Two failures occurred during execution and were fixed, both in test code
rather than production code:

1. The new integration test
   `test_a_cursorless_request_bootstraps_the_sequence_watermark` asserted
   `s.id == "boot-1" && s.id == "boot-2"`, which no single row can satisfy.
   The assertion was rewritten to require both identifiers to be present.
2. Two existing server tests asserted the **old** answer rule (a cursorless
   request answers zero). They were rewritten to the new contract, and the
   timestamp-only case — the shape the old test meant to cover but did not,
   because it sent `last_sync_timestamp: 0` — is now covered by its own
   assertion.

`bash scripts/check.sh` was **not run**. Verification was deliberately
scoped to the affected surfaces, per the instruction to keep testing minimal
and bounded to the change. `scripts/check.sh` remains the authoritative
pre-merge gate.

## 5. Invariant review

| Invariant | Evidence | Result |
|---|---|---|
| The client merge order is `(updated_at, device_id, SHA-256(synced fields))`, and deletion beats live content | `src/sync_commands.rs`'s merge and fingerprint code is untouched by this milestone | pass |
| The server's last-write-wins upsert and its `deleted` column semantics | `snip-sync/src/db.rs` untouched | pass |
| The deterministic identifier derivation and the synced field set | `src/library/persistence.rs` untouched | pass |
| The timestamp cursor remains the fallback | The server still answers a timestamp-only request with zero, and the client still sends the timestamp cursor on every request | pass |
| The protocol change is additive | `snip-proto` untouched at 0.1.4 | pass |
| The per-library sequence counter keeps its semantics | `snip-sync/src/db.rs` untouched: a strictly increasing server-assigned value, a rolled-back write consumes no value, the unique index turns a counter race into a loud failure | pass |
| `last_sync` does not advance when records were skipped | The existing `has_failures` guard is unchanged; the cursor update is still skipped | pass |
| The client never adopts a watermark it cannot interpret | A timestamp-only request is answered with zero; a push-only sync never adopts | pass |

## 6. Failure and recovery review

- **Duplicate delivery and idempotency.** The delta filter is inclusive, so
  the boundary row is re-delivered on every sync. That is the pre-existing
  at-least-once behavior and it is what makes the bootstrap safe without a
  cross-statement snapshot.
- **Cancellation races.** The stored cursor is written only after a complete
  response, so an interrupted sync leaves the previous cursor in place and
  the next sync re-requests the same range. The bootstrap is idempotent: a
  repeated full sync re-applies the same rows through the same merge.
- **Restart.** The counter and the watermark live in the database, not in
  memory, so both survive a server restart. The client's recorded kind lives
  in the library metadata and survives a client restart.
- **Partial persistence failure.** The cursor update goes through the
  existing `update_sync_cursors` path, which takes the mutation gate and the
  local-data lock and writes the config atomically. A failed write leaves the
  previous cursor in place.
- **Stale generation or lease.** The client's cursor state is not a lease and
  has no generation. The server's counter is bumped inside the row-write
  transaction, so a rolled-back write consumes no sequence value.
- **Contention and resource release.** Two clients syncing the same library
  receive distinct sequence ranges. The unique index on
  `(library_id, sequence)` turns a counter race into a loud failure rather
  than a silent collision.
- **Malformed or unauthorized input.** No new input is accepted: the two
  request fields already existed and their types are unchanged. A negative
  or absurd cursor value is treated exactly as before.
- **Bounded event and artifact behavior.** The bootstrap downloads the
  library once per client. It is not a background job and introduces no new
  scheduler state.

## 7. Migration and compatibility review

- **Schema migration.** None on the server — the database schema is
  unchanged. On the client the library metadata gains one additive field
  that defaults to `unknown`, so no migration runs and no existing file is
  rewritten.
- **Backward compatibility.** New client, old server: the client's bootstrap
  request carries no cursor, the old server ignores the unknown field and
  answers no sequence, and the client records `timestamp` and behaves exactly
  as it did before. Old client, new server: the old client sends no
  sequence, the server serves the timestamp filter, and in the bootstrap
  case the server's new answer carries a watermark that the old client
  ignores as an unknown proto field — its row set is unchanged.
- **Protocol negotiation.** None needed: the fields already existed and the
  negotiation is implicit in their values.
- **Configuration behavior.** The library metadata gains one field. Nothing
  else in the configuration changes.
- **Rollback limitations.** Reverting the server's answer rule and the
  client's state machine restores the previous behavior. The new tests would
  then fail, which is the intended signal. A client that already adopted a
  sequence cursor keeps sending it; an old server ignores it.
- **Legacy-path status.** The timestamp path is intact and is still the path
  for any client whose server does not serve a sequence.

## 8. Security review

- **Authorization enforcement.** Unchanged. The server still scopes every
  query by `(user_id, library_id)` and still verifies library ownership
  before answering.
- **Secret handling.** Unchanged. The cursor is library metadata, stored in
  the library file, never in the keychain. No credential passes through a
  `runtime_error` detail.
- **Path validation.** Unchanged. The milestone touches no filesystem path
  handling.
- **Privilege boundaries.** Unchanged.
- **Denial-of-service bounds.** The bootstrap is one full-library transfer
  per client, once. The server's page size and the client's pagination bound
  are unchanged, and the client's `MAX_SYNC_PAGES` guard still applies.
- **Redaction.** Unchanged.
- **Audit behavior.** Unchanged — the milestone writes no audit entry and
  performs no user-visible mutation.

## 9. Documentation and operations

Updated architecture documents:

- `architecture/sync.md` — the cursor state machine, the bootstrap handshake,
  the three request shapes, the zero-watermark subtlety, the boundary-row
  property, and the remaining timestamp-path limitation.
- `architecture/server.md` — the server's answer rule and the minimum-of-1
  watermark for an empty library.
- `docs/IDENTITY_CONTRACT.md` — the new `sync_cursor_kind` metadata field and
  its relationship to the identifier contract.

Updated decision records:

- `plans/adrs/ADR-0001-sync-delta-cursor.md` — a dated status-history note
  recording that the bootstrap handshake was implemented in this milestone
  and that the protocol needed no new field. The ADR's decision history is
  unchanged.

Updated changelog:

- `CHANGELOG.md` — `### Changed` entries for the reachable cursor and the
  client's recorded cursor kind, with the version facts.

Static guards and build gates: **none added**. The milestone's guard is the
behavioral test suite, not a static one.

Recovery instructions: none. The milestone is not a recovery path.

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| low | A client that stays on the timestamp path — because its server is too old to serve a sequence, or because it only ever syncs in the push direction — keeps the timestamp path's residual limitation: the cursor is a **position, not a receipt**. The server reports the library watermark and the client stores it even in a sync that discarded the rows at that position, so a push-only sync advances the timestamp cursor past rows the client never received. | Bounded and pre-existing. The original ADR-0001 defect (a row pushed with an old `updated_at` never being delivered) was already closed by M001's `visible_at` clamp; what remains is the position-not-receipt property, and the sequence path avoids it because the client advances the sequence cursor only in a sync that merged what it was served. | None. Documented in `architecture/sync.md`. A future milestone could offer an explicit re-probe for a client whose server was upgraded after the fact. |
| low | The client's recorded kind is only refreshed on a sync that reaches the server. A client whose server is upgraded after it recorded `timestamp` stays on the timestamp path until the library is relinked, unlinked, or reset. | Bounded: the client keeps working on the timestamp path. | None. The reset paths return the kind to `unknown`, so the next pulling sync bootstraps. |
| low | The bootstrap transfers the whole library once per client. A very large library pays that cost on its first sync after the upgrade. | Bounded and one-time. | None. |
| low | `bash scripts/check.sh` was not run for this milestone; verification was scoped to the affected surfaces. | The full Linux CI gate is unverified for this change set, although every component it runs was verified individually. | Run `scripts/check.sh` before merging. |

No critical, high, or medium finding remains.

## 11. Roadmap disposition

**Milestone closed, and M001's named condition is discharged.**

This milestone supplies the evidence M001's closure record named: a test
showing a client that synced against a sequence-capable server sends a
non-zero `last_sync_sequence` and the server answers with a non-zero
`server_sequence`. M001's closure record is left immutable; its status stays
`conditionally closed`, and the registry and the roadmap record that the
condition is discharged by this milestone.

The subsystem roadmap `plans/subsystems/sync-cursor-protocol-roadmap.md`
stays `active`: M001 is closed-conditionally with its condition discharged,
M002 is closed, and no further milestone is defined for this subsystem.

No corrective implementation plan is required for this milestone.

## 12. Registry updates

`plans/registry.md`:

- The `sync cursor protocol` roadmap row's milestone cell becomes
  "M001 conditionally closed (condition discharged by M002); M002 closed".
- A new row in the dependency-ready implementation-plan table for
  `plans/implementation/sync-cursor-protocol/002-cursor-bootstrap-and-precision-convergence.md`,
  status `closed`.
- The "Sync cursor gate" paragraph is rewritten: the cursor is no longer
  dormant, the bootstrap exists, and the M001 named condition is discharged.
- The closure-work table gains a row for the M002 closure record, and the
  M001 row notes that its condition is discharged.

`plans/subsystems/sync-cursor-protocol-roadmap.md`:

- The M002 milestone's status becomes `closed` with the closure-record link.
- §4's current-state text is updated to record that the bootstrap exists.
- A dated change note records the closure and that M001's completed history is
  unchanged.
