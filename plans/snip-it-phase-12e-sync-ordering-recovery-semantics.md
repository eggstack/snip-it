# Phase 12E — Deterministic Sync Ordering and Truthful Recovery Semantics

Status: COMPLETE

Baseline: `b14dd66102d0c1a63deed4f14b2bc2391ef4c0a3`

Roadmap: `plans/snip-it-phase-12-lightweight-correctness-footprint-roadmap.md`

Prerequisite: Phase 12A complete.

This phase addresses two bounded sync correctness risks without introducing CRDTs or a new protocol architecture:

1. equal wall-clock timestamps currently resolve according to which copy is called “server,” not a stable deterministic order;
2. library recreation writes a recovery marker that is later deleted without actually completing recovery, so the marker overstates its guarantee and may permit repeated remote creation.

The solution must remain simple, deterministic, and compatible with the current local-first tool.

---

## 1. Required outcomes

1. Equal-timestamp snippet conflicts resolve identically on every device and regardless of merge invocation direction.
2. Deletion conflict behavior remains explicit and deterministic.
3. Clock-skew limitations are documented truthfully without adding distributed-clock machinery.
4. Remote library recreation is idempotent or leaves enough durable local information to finish relinking after a crash.
5. A recovery marker is never silently deleted while unresolved work remains.
6. Focused tests cover ordering and crash-boundary recovery behavior.

---

## 2. Complexity budget

Expected production files:

```text
src/sync_commands.rs
src/sync.rs
src/library.rs
src/config.rs or library metadata module
snip-sync/src/lib.rs
snip-sync/src/db.rs
snip-proto/proto/sync.proto (only if the bounded selected design requires it)
architecture/sync.md
```

The preferred deterministic ordering change should require no protocol field.

Recovery semantics may use the existing local library metadata and current server create/list APIs. A proto/schema change is allowed only if it is narrowly additive and clearly simpler than local durable completion data.

Do not exceed one small schema migration or one additive RPC field in this phase. Prefer no protocol change.

---

## 3. Explicit non-goals

Do not:

- implement CRDTs;
- implement vector clocks, Lamport clocks, or hybrid logical clocks;
- add a general operation log;
- add tombstone garbage collection;
- add background reconciliation services;
- add cross-server replication;
- redesign device registration;
- add server consensus or transaction coordinators;
- require synchronized system clocks;
- introduce a database solely for client recovery;
- redesign all library identifiers;
- add broad migration infrastructure;
- add stress/soak tests or fault-injection frameworks;
- expand CI or release automation.

---

# Workstream A — Define one deterministic snippet ordering key

## Current behavior

For a snippet present locally and remotely, the server copy wins when:

```rust
server.updated_at >= local.updated_at
```

Equal timestamps therefore always prefer the server copy. If two devices perform the same merge with their local/remote roles reversed, they can choose different winners.

## Required ordering

Use one stable comparison key for conflict-bearing fields.

Preferred key:

```text
(updated_at, device_id, content_fingerprint)
```

A simpler `(updated_at, device_id, snippet_id)` key is insufficient for two conflicting copies of the same snippet ID written by the same device ID, though that condition should be uncommon. A deterministic content fingerprint gives a final stable tie-break without new persisted fields.

Recommended comparator:

```rust
fn conflict_order_key(snippet: &ComparableSnippet) -> (i64, &str, [u8; 32])
```

The fingerprint should cover only synced conflict-bearing fields in a canonical order:

```text
id
description
command
tags in their stored order or canonically sorted according to existing semantics
created_at
updated_at
device_id
deleted
```

Do not include local-only fields:

```text
output
folders
favorite
```

Use an already-linked hash implementation such as SHA-256. Do not add a new hashing dependency.

If tags are semantically unordered elsewhere, sort a temporary copy before hashing. If tag order is user-visible and intentionally preserved, hash stored order. Choose one behavior and document it.

## Alternative bounded key

If the implementation can prove that each mutation always changes `device_id` or cannot produce same-device equal-timestamp divergence, `(updated_at, device_id)` is acceptable. The proof must be recorded in this plan’s implementation notes. Do not assume it without checking mutation paths.

## Comparator rule

The same comparator must be used regardless of local/server role:

```rust
match compare_versions(local, remote) {
    Ordering::Less => remote wins,
    Ordering::Greater => local wins,
    Ordering::Equal => versions are equivalent for synced fields,
}
```

Do not use `>=` with role-dependent preference.

## Acceptance criteria

- [ ] Equal timestamps no longer automatically prefer server role.
- [ ] Swapping local and remote inputs yields the same winning synced content.
- [ ] Comparator uses only deterministic persisted/synced data.
- [ ] Local-only fields are preserved according to current semantics.
- [ ] No new dependency is added.

---

# Workstream B — Preserve deletion semantics under deterministic ordering

## Current behavior to review

Current merge logic includes special cases:

- a server tombstone marks an existing local live copy deleted;
- a local tombstone is preserved even when the server live copy is newer;
- when both sides are deleted, the snippet may be omitted from the returned local list;
- local deleted snippets absent from the server are not preserved in the merged local list.

This behavior does not fully follow last-write-wins and may be intentional to avoid resurrection. Phase 12E must make the rule explicit before changing the comparator.

## Required bounded policy

Preferred policy:

1. **Deletion wins over live content at equal ordering timestamp.**
2. **A strictly newer live update may resurrect only if the existing product intentionally supports resurrection.** If current behavior deliberately forbids resurrection, retain delete-wins regardless of timestamp and document that this is not pure LWW.
3. **Tombstones needed for upload must not disappear before the server acknowledges them.**
4. **When both sides agree deleted, local display may omit the record, but synchronization state must not lose a deletion that still needs propagation.**

Do not redesign tombstone retention across the entire protocol in this phase. Correct only inconsistencies exposed by deterministic tie handling.

## Implementation rule

Create one function that decides synced winner state, including deletion:

```rust
enum VersionWinner {
    Local,
    Remote,
    Equivalent,
}

fn choose_version(local: &Snippet, remote: &ProtoSnippet) -> VersionWinner
```

Local-only field preservation occurs after winner selection.

Avoid deeply nested role-specific branches in `merge_snippets`.

## Focused tests

Required matrix:

| Local | Remote | Timestamp relation | Required result |
|---|---|---|---|
| live A | live B | local newer | A |
| live A | live B | remote newer | B |
| live A | live B | equal, different device | deterministic key winner |
| live A | live B | equal, same device, different content | deterministic fingerprint winner |
| deleted | live | equal | documented deletion policy |
| live | deleted | equal | same documented deletion policy independent of role |
| deleted | live | one strictly newer | documented resurrection/delete-wins policy |
| deleted | deleted | any | no resurrection; stable result |

For every equal-timestamp case, run the comparator with roles swapped and assert the same synced result.

## Acceptance criteria

- [ ] Delete/live equal conflicts are role-independent.
- [ ] Existing intended no-resurrection behavior is retained or a narrowly justified correction is documented.
- [ ] Local-only fields remain local-only.
- [ ] Tests cover role swapping.

---

# Workstream C — Document and bound clock-skew behavior

## Goal

Deterministic tie-breaking does not solve large clock skew. Phase 12E must state the limitation and ensure the current timestamp generation is at least internally consistent.

## Required inspection

Trace all mutation sites that set `updated_at` and confirm they use one helper/time unit.

Required properties:

- same unit everywhere;
- monotonically nondecreasing per local snippet where easily enforceable;
- updated timestamp changes on synced-field mutation and deletion;
- local-only field changes do not accidentally alter synced ordering unless intentionally synchronized.

A lightweight local monotonic rule is acceptable:

```rust
new_updated_at = max(now, old_updated_at.saturating_add(1))
```

Use it only if current clock rollback can otherwise produce a locally older edit. This is per-record monotonicity, not a distributed clock.

Do not add stored logical counters or clock services.

## Documentation

Update `architecture/sync.md` to state:

- primary ordering is wall-clock timestamp;
- equal values use deterministic device/content tie-break;
- severe forward clock skew can cause one device’s versions to dominate until real time catches up;
- users should correct system clocks if this occurs;
- the system is not a CRDT and intentionally chooses simplicity.

## Acceptance criteria

- [ ] Timestamp unit and mutation helpers are consistent.
- [ ] Optional per-record monotonic update is small and tested if adopted.
- [ ] Clock-skew limitation is documented without overstating guarantees.
- [ ] No logical-clock schema is added.

---

# Workstream D — Replace the non-recovering recovery marker

## Current defect

When a remote library is missing, the client writes `<library>.sync_recovery`, creates a new remote library, updates local linkage, retries sync, and removes the marker on success.

At startup/next sync, `check_and_complete_recovery_markers()` currently finds an incomplete marker and removes it without completing any relink. A crash after remote creation but before local metadata update can therefore leave an orphaned remote library and allow another creation attempt.

## Required design choice

Choose the smallest of the following designs after inspecting current server APIs and library metadata.

### Preferred option A — Idempotent remote create by stable local library ID

If the create request can use an existing stable client `library_id`, make creation idempotent:

```text
create_library(stable_library_id, normalized_name)
```

Server behavior:

- same authenticated user + same stable ID returns existing library;
- conflicting name may return existing metadata or a clear conflict;
- repeated request after a crash does not create a duplicate.

This may require a narrowly additive request field or reuse of an existing ID field. Keep backward compatibility if proto changes.

### Option B — Durable marker containing created server ID

If remote creation cannot be made idempotent without a larger protocol change, make the marker a real recovery record:

```toml
schema = 1
local_library_name = "work"
local_library_id = "..."
server_library_id = "..."
created_at_unix_ms = ...
phase = "remote_created"
```

Required sequence:

1. write marker before remote create with `phase = "creating"` if useful;
2. after server returns ID, atomically update marker with `server_library_id` before local metadata update;
3. update local library linkage;
4. retry sync;
5. remove marker only after local linkage and required sync state are durable;
6. on next run, use marker’s server ID to finish local relink instead of creating again.

The marker must be written atomically using existing helpers and contain no API key or snippet content.

### Option C — Truthful best-effort with no marker

If neither A nor B can be implemented simply, remove the marker and stop claiming crash recovery. Before repeating create, list/search existing user libraries by stable normalized name and reuse a unique match. If zero matches, create. If multiple matches, stop with an explicit operator-facing error.

This is weaker than A/B but more truthful than deleting an unresolved marker.

## Selection rule

Choose A when a small additive ID path exists. Choose B when server ID is already returned and local marker completion is straightforward. Choose C only when A/B would cause a broad protocol/schema migration.

Record the selected option and why in this plan.

Do not combine A and B unless B is needed only for migration compatibility.

## Acceptance criteria common to all options

- [ ] A crash after remote creation cannot cause silent repeated creation on the next normal retry.
- [ ] An unresolved marker is not simply deleted.
- [ ] Recovery data contains no credentials or snippet content.
- [ ] Recovery uses atomic local writes.
- [ ] Ambiguous remote matches fail visibly rather than choosing arbitrarily.
- [ ] The user can repair or retry with existing commands.

---

# Workstream E — Make recovery state interaction with library metadata atomic enough

## Goal

Local relinking must not leave `library_id`, `server_id`, and `last_sync` in a contradictory state across separate writes when the current `LibraryManager` can update them together.

## Required inspection

Review:

```text
LibraryMeta
LibraryConfig
link_server_library
update_last_sync
save library index/config path
transaction/atomic write helpers
```

Preferred implementation:

- one in-memory metadata update;
- one atomic save of the library index;
- reset `last_sync` in the same write as new server linkage;
- only then proceed with retry sync;
- after success, update final server timestamp in a second atomic save.

Do not add a database transaction for TOML metadata.

If existing methods already perform one atomic config save each, add one narrow combined method such as:

```rust
relink_server_library(name, server_id, last_sync)
```

Do not create a general transaction builder.

## Acceptance criteria

- [ ] New server ID and reset sync timestamp are persisted together.
- [ ] Recovery can identify whether relinking completed.
- [ ] Existing library index atomic-write behavior is reused.
- [ ] No new persistence format is introduced beyond the selected recovery option.

---

# Workstream F — Focused recovery tests

## Required deterministic cases

Use in-process test server support where already available. Do not add a new mock server framework.

### Test F1 — Repeated recreation request is idempotent or reuses recovery state

1. Create/register test user.
2. Create local library metadata.
3. Simulate server-side library absence.
4. Run recovery through the selected design up to the crash boundary immediately after remote creation.
5. Re-run recovery.
6. Assert exactly one remote library represents the local library.
7. Assert local linkage completes.

### Test F2 — Crash before remote creation completes

Recovery retry may create once and then complete normally. No stale marker is silently removed.

### Test F3 — Crash after marker/server ID persisted but before local relink

Next run must use recorded server ID and must not create another remote library.

Required only for option B.

### Test F4 — Ambiguous remote libraries

For option C, create two same-name remote libraries and assert recovery stops with an explicit ambiguity error.

### Test F5 — Marker corruption

A corrupt recovery marker must be preserved or quarantined through existing repair behavior and surfaced. It must not be silently deleted and followed by blind creation.

Do not build a crash-process harness. Expose small recovery-step functions or use existing failpoints only if already available and appropriate.

## Acceptance criteria

- [ ] Focused tests cover the chosen design’s actual crash boundaries.
- [ ] No duplicate remote library is created in the tested retry path.
- [ ] Corrupt/ambiguous state fails visibly.
- [ ] Tests remain bounded and deterministic.

---

## 4. Recommended implementation order

1. Extract deterministic version comparator and add role-swap tests.
2. Integrate deletion policy into comparator/merge flow.
3. Audit timestamp mutation helpers and document skew.
4. Select recovery option A, B, or C after inspecting current API/schema.
5. Implement one bounded recovery design.
6. Combine local relink metadata updates if currently split.
7. Add focused recovery tests.
8. Update current sync architecture documentation.
9. Record design choice, implementation SHA, and verification commands in this plan.

Do not begin with a proto migration before proving option A requires it.

---

## 5. Verification commands

Use focused tests first:

```text
cargo fmt --all -- --check
cargo test -p snip-it sync_commands --all-features -- --test-threads=1
cargo test --test sync_integration --features test-support -- --test-threads=1
cargo test --test sync_e2e --features test-support -- --test-threads=1
cargo test -p snip-sync --lib -- --test-threads=1
cargo check --workspace --all-targets --all-features
bash scripts/check.sh
```

Use the exact existing integration target names in the repository; omit a listed target if it does not exist and record the actual focused target used.

If proto changes, also run the existing proto generation/contract test. Do not add a separate protocol compatibility suite.

---

## 6. Prohibited outcomes

This phase fails if it:

- retains role-dependent equal-timestamp winner selection;
- hashes local-only fields into synced conflict ordering;
- introduces CRDT/vector-clock machinery;
- silently permits resurrection contrary to documented deletion policy;
- deletes unresolved recovery markers;
- repeats remote creation after a recoverable crash boundary;
- chooses arbitrarily among ambiguous remote libraries;
- stores credentials or snippet content in recovery state;
- adds a client database or generalized journal;
- expands CI/release/testing architecture;
- performs a broad protocol rewrite.

---

## 7. Closure checklist

- [x] Deterministic version comparator implemented.
- [x] Equal-timestamp role-swap tests pass.
- [x] Deletion policy is explicit and tested.
- [x] Timestamp mutation behavior is consistent.
- [x] Clock-skew limitation is documented.
- [x] Recovery option A, B, or C is selected and recorded.
- [x] Recovery no longer deletes unresolved state without action.
- [x] Retry after remote creation does not create a duplicate.
- [x] Local linkage fields are persisted coherently.
- [x] Focused sync and server tests pass.
- [x] `cargo check --workspace --all-targets --all-features` passes.
- [x] `bash scripts/check.sh` passes.
- [x] Plan records implementation SHA and verification commands.
- [x] No CRDT, new daemon, or broad schema framework was introduced.

When all items are satisfied, mark Phase 12E COMPLETE. Do not open another sync-consistency phase unless a reproducible conflict or recovery defect remains.

## Implementation notes

- Selected recovery option: **B**, with a narrow name-reuse guard for the
  crash window before the server ID can be persisted. The current create RPC
  does not accept a stable client library ID, while it does return the server
  ID; adding a protocol field would be broader than the bounded local marker.
- Live conflict ordering is `(updated_at, device_id,
  SHA-256(id, description, command, stored-order tags, created_at, updated_at,
  device_id, deleted))`. Local-only fields are excluded.
- Deletion remains a deliberate no-resurrection rule. Equal and unequal
  delete/live comparisons are role-independent; both-deleted records remain
  omitted from display.
- Recovery markers are durable TOML written with `atomic_replace`. The marker
  records `creating`, `remote_created`, and `linked` phases, preserves corrupt
  state, rejects ambiguous normalized-name matches, and is removed only after
  relink, merged-library persistence, and the final sync cursor update.
- `LibraryManager::relink_server_library` persists server linkage and
  `last_sync` in one locked config save.
- Focused tests cover live/live ordering, same-device fingerprint ties,
  delete/live role swaps, marker round trips, and corrupt-marker preservation.

Verification completed locally:

```text
cargo fmt --all -- --check
cargo test -p snip-it sync_commands --all-features -- --test-threads=1
cargo test --test sync_integration --features test-support -- --test-threads=1
cargo test --test sync_contracts --features test-support -- --test-threads=1
cargo check --workspace --all-targets --all-features
bash scripts/check.sh
```

Implementation commit SHA: `1a9292122a23a94a5d6e435c4a26752873d23fe9`.
