# snip-it Active Planning Registry

This file is the compact control surface for active interim planning.
Detailed requirements and completed history remain in source roadmaps,
implementation plans, `plans/closure/`, and Git history.

Canonical direction remains in:

- `plans/000-long-term-specification.md`
- `plans/001-terminology-and-domain-model.md`
- `plans/002-long-term-roadmap.md`
- `plans/003-planning-process.md`

Planning-convention migration: the pre-convention flat sequence (plans
000–020) was promoted via `git mv` into
`plans/implementation/<subsystem>/` (history preserved; bodies unchanged)
with predecessor pointers in `plans/archive/flat-*.md` and completion
gates in `plans/closure/<subsystem>/`. Pre-convention `Status: complete`
maps to `closed`.

## Status vocabulary

- **proposed** — roadmap or plan exists but is not approved for execution.
- **ready** — dependencies and interfaces are satisfied; plan may be handed off.
- **active** — implementation or closure work is in progress.
- **blocked** — a named dependency or evidence requirement prevents progress.
- **closing** — implementation landed and closure evidence is being gathered.
- **closed** — closure record accepted.
- **conditionally closed** — substantial work landed, but a named correctness or operational evidence condition remains.
- **superseded** — replaced by another document.
- **archived** — no longer active and retained for traceability.
- **corrective pass required** — closure evidence is insufficient; a named
  corrective pass must land before the milestone can close.

## Active subsystem roadmaps

| Subsystem | Status | Roadmap | Current milestone | Dependencies or blockers |
|---|---|---|---|---|
| Distribution and release | closed | `plans/subsystems/distribution-release-roadmap.md` | M001-M005 closed | None. Corrective M005 closed without a second workflow or matrix duplication. |
| Server lifecycle and HTTP runtime | closed | `plans/subsystems/server-lifecycle-http-roadmap.md` | M001-M004 closed | None. Direct EggServe leaf (C) is the final architecture; parity socket-proven. |
| Updater transport | closed | `plans/subsystems/updater-transport-roadmap.md` | M001-M005 closed | None. `snp` on lean `eggfetch-core` (exact pin in `Cargo.toml`); `snip-sync` on `curl` by measurement. |
| CLI, library, selector, sync-policy consolidation | complete | `plans/subsystems/cli-library-sync-consolidation-roadmap.md` | M001-M006 closed | None. M006 closed the search-parity verification gap on 2026-10-22: all three surfaces now share one match primitive, the two open behaviors are decided and pinned, and the parity suite gates the build. M001-M005 closure history unchanged. |
| Local MCP integration | closed | `plans/subsystems/mcp-integration-roadmap.md` | M001 closed | None. Read-only stdio adapter with search parity and registration. |
| Eggstack dependency refresh | closed | `plans/subsystems/eggstack-dependency-refresh-roadmap.md` | M001 closed | None. `eggfetch-core` and `eggserve-server`/`eggserve-primitives` adopted on their exact pins in `Cargo.toml` with zero source changes; wire/lifetime/transfer parity proven, both release artifacts byte-identical. |
| Maintenance and duplicate-policy consolidation | active | `plans/subsystems/maintenance-consolidation-roadmap.md` | M001 closed | None. M001 dead subsystem removal and documentation reconciliation landed; M002 and M003 depend on M001 and on each other only softly. |
| Locking protocol and ordering convergence | active | `plans/subsystems/locking-convergence-roadmap.md` | M001 closed | None. M001 owned-record lock protocol unification landed (one shared protocol, the wait policy as a parameter); M002 kernel-lock protocol convergence; M003 ordering contract and static guard, hard on M001 and M002. |
| Snippet CRUD surface | active | `plans/subsystems/snippet-crud-surface-roadmap.md` | M001 closed | None. M001 non-interactive snippet delete landed; M002 folder and favorite write path and M003 non-interactive field update, both hard on M001. |
| Sync cursor protocol | active | `plans/subsystems/sync-cursor-protocol-roadmap.md` | M001 conditionally closed (condition discharged by M002); M002 closed | Implements `plans/adrs/ADR-0001-sync-delta-cursor.md` Option A. The coordinated `snip-proto` 0.1.4 / `snip-sync` 0.1.8 / `snip-it` 1.4.1 version bumps landed in one change. The bootstrap now exists: a cursorless request is answered with the sequence watermark, and the client adopts it. **M001's named condition is discharged by M002.** |

## Dependency-ready implementation plans

| Plan | Status | Subject | Dependencies or blockers |
|---|---|---|---|
| `plans/implementation/maintenance-consolidation/001-dead-subsystem-removal-and-doc-reconciliation.md` | closed | Maintenance and duplicate-policy consolidation M001 — delete unreachable policy (`src/migration.rs` and the rest of the dead set) and reconcile the canonical documentation that governs it | None. One coherent pass: a deletion sweep plus the documentation reconciliation it forces. Requires a `command_behavior()`-neutral CLI change (the `snp backup --format` flag removal) and a new static guard in `tests/architecture.rs`. |
| `plans/implementation/locking-convergence/001-owned-record-lock-protocol-unification.md` | closed | Locking protocol and ordering convergence M001 — implement the owned-record lock protocol once, with the transaction lock's fail-fast and the local-data lock's wait-for-progress as explicit policies | None. Both callers' contracts are already known and differ only in the wait policy. Preserves the conservative ownership rule and the ownership-verified quarantine exactly. |
| `plans/implementation/snippet-crud-surface/001-non-interactive-snippet-delete.md` | closed | Snippet CRUD surface M001 — give a script a deterministic way to tombstone a snippet, producing the same persisted state as the existing interactive delete path | None. The persisted behavior already exists in the TUI selection loop and is extracted and reused rather than redesigned. Requires a `command_behavior()` entry, a `snp data` mirror arm, and a `docs/COMMAND_CONTRACTS.md` row. |
| `plans/implementation/sync-cursor-protocol/001-monotonic-per-library-sequence-cursor.md` | conditionally closed | Sync cursor protocol M001 — implement ADR-0001 Option A, a server-assigned strictly increasing per-library write sequence used by the delta filter and the client cursor | None. ADR-0001 is normalized to `accepted`. Additive proto fields, so both version-skew directions keep working. Requires coordinated `snip-proto`, `snip-sync`, and `snip-it` version bumps in one change. |
| `plans/implementation/sync-cursor-protocol/002-cursor-bootstrap-and-precision-convergence.md` | closed | Sync cursor protocol M002 — add the bootstrap that makes the write-sequence cursor reachable, and record in the client which cursor the server is serving | None. Discharges M001's named condition. No new protocol field: the bootstrap reuses `last_sync_sequence` and `server_sequence` as they are, so `snip-proto` stays 0.1.4 and is not republished. Requires `snip-sync` 0.1.8 and `snip-it` 1.4.1 in one change. |
| `plans/implementation/locking-convergence/002-kernel-lock-protocol-convergence.md` | ready | Locking protocol and ordering convergence M002 — one kernel-backed file-lock implementation per platform, one busy-versus-error classification, and the `croncheck.lock` copy removed | None. M001 closed the owned-record protocol; this milestone is the kernel-lock family, which is a different thing and stays separate. The plan's proposal of a new shared crate is out of bounds under the standing no-new-crates rule and must be replaced before execution. |
| `plans/implementation/locking-convergence/003-lock-ordering-contract-and-static-guard.md` | ready | Locking protocol and ordering convergence M003 — one stated acquisition order over the lock tiers, the code reconciled to it, and a static guard that fails on a violating acquisition site | Hard on M002: both milestones touch `src/auto_sync/execution_lock.rs`. Removes the retired `WorkerLock` tier, which is a breaking `status --json` change. |
| `plans/implementation/maintenance-consolidation/002-backup-and-snapshot-policy-convergence.md` | ready | Maintenance and duplicate-policy consolidation M002 — one shared copy policy for every snapshot, backup, and rollback-point path: symlink refusal and fsync become properties of the primitive | None. M001 closed the dead subsystem. The shared primitive belongs in the atomic/persistence layer under a named durability class. |
| `plans/implementation/maintenance-consolidation/003-diagnostic-vocabulary-unification.md` | ready | Maintenance and duplicate-policy consolidation M003 — one shared diagnostic vocabulary and one shared library scan across the validate, doctor, and repair surfaces | Soft on M002. No diagnostic code, message, or exit code changes. |
| `plans/implementation/snippet-crud-surface/002-folder-and-favorite-write-path.md` | ready | Snippet CRUD surface M002 — give the user a way to set and clear the snippet `folders` and `favorite` fields that the product already reads, sorts on, and displays | Hard on M001: the delete primitive it reuses landed in M001. Requires a `command_behavior()` entry, a `snp data` mirror arm, and a `docs/COMMAND_CONTRACTS.md` row. |
| `plans/implementation/snippet-crud-surface/003-non-interactive-field-update.md` | ready | Snippet CRUD surface M003 — allow a snippet's description, command, and tags to be changed non-interactively, generalizing the existing `snp edit --output` path | Hard on M002: it extends the shared field-update primitive M002 introduces. Extends `snp edit` rather than adding a command, because `snp update` is the self-update command. |
| `plans/implementation/cli-library-sync-consolidation/006-search-parity-verification-corrective.md` | closed | CLI, library, selector, sync-policy consolidation M006 — close the search-parity verification gap and gate the parity suite in `scripts/check.sh` | None. Corrective pass over the closed M004; M001-M005 closure history unchanged. Closed 2026-10-22: the three surfaces now share one match primitive, the empty-query and cross-library-order behaviors are decided and pinned, and the parity suite runs in `scripts/check.sh`. |
| `plans/adrs/ADR-0001-sync-delta-cursor.md` | accepted | Monotonic per-library sequence as the sync delta cursor | The **defect** is closed by the implemented server-side cursor column (`snippets.visible_at`, clamped to the library watermark): the cursor is server-assigned, `updated_at` keeps the client's logical time, and no protocol change was needed for that closure. What remains is the second-granularity wart the `>=` filter still has, which needs a `snip-proto` field, a server schema migration, and a client cursor change — one change with a version bump for `snip-proto`, `snip-sync`, and `snip-it`. The status was normalized from the off-lifecycle word `amended` to `accepted` on 2026-10-22 with the transition recorded in the record's status history; the implementation is tracked by the sync cursor protocol roadmap's M001. |

Register new work here only after dependency and handoff review per
`plans/003-planning-process.md` §10.

## Current execution order and dependency gates

**Distribution gate:** M001-M004 closed the matrix, installers, platform
gate, and publication evidence. Corrective M005 is closed
(`snip-it 1.3.8` five binaries plus checksums; `v1.3.7` untouched;
five-target consumer smoke; deterministic installer contracts). The current
crate version is `snip-it 1.3.9` (Argon2 0.6 release, tag `v1.3.9`); M005's
publication evidence remains the gate record for the release workflow.

**Server gate:** M001 lifecycle baseline closed. M002 recorded the A/B
measurement (A 3,833,152; B 3,963,968). M003 selected C (3,898,408) as the
single remaining architecture. M004 closed parity (CORS `Vary`, empty
404/405, preflight `Allow`, header boundary) with socket tests plus the
M002/M003 evidence record (implementation `10aeb994`, hosted run
`36011491151`).

**Updater gate:** M001 self-update closed. M002 recorded the transport
split (`snp` lean eggfetch vs `snip-sync` curl). M003 closed the
end-to-end timeout corrective with slow-drip tests and partial-file
cleanup. M004 adopted lean 0.1.7 (-2.82%). M005 adopted 0.2.0 for `snp`
(byte-identical 6,776,320) and discarded the `snip-sync` trial (+34.23%).

**Consolidation gate:** M001-M006 closed CLI/outcome dedup, readonly
resolution, boundary-respecting splits, selector/search/MCP parity, retry
unification with compatibility preserved, and the search-parity verification
gap. The roadmap was reopened on 2026-10-22 for M006, a corrective pass over
M004, and closed again the same day. M006 found that the parity fixtures
compared `snp list --filter` against MCP `snippets_search` but never
exercised `snp get --query`, and that the parity target did not run in
`scripts/check.sh` or in CI. Both are closed: the suite now exercises all
three surfaces and gates the build. The two newly discovered defects M006
carried — divergent empty-query semantics across the three surfaces, and a
cross-library ordering claim that was commented but not asserted — are
decided and pinned: an empty query is "no filter" on all three surfaces, and
the cross-library order differs by surface family on purpose (the selector
family orders by library → description → id; the merged-set family orders by
global relevance; they coincide within one library).

**Maintenance gate:** M001 removes unreachable policy and reconciles the
canonical documentation that governs it. The dead `src/migration.rs`
module is currently retained behind a blanket `#![allow(dead_code)]`, and
both `AGENTS.md` and `plans/000-long-term-specification.md` §5 carry a
durable rule about its `write_schema_version` function, which nothing
calls. M002 then converges the four backup and snapshot paths onto one
copy policy with symlink refusal and fsync; M003 unifies the validate,
doctor, and repair diagnostic vocabularies onto one shared vocabulary and
one shared library scan.

**Locking gate:** M001 unifies the transaction and local-data owned-record
lock protocols, whose only difference today is a wait policy that had been
duplicated into two diverging copies along with a byte-identical
`is_transient_lock_contention` helper. M002 converges the four copies of
the kernel-backed lock protocol — the canonical one, the server mirror, an
undocumented `croncheck.lock` copy in `snip-sync/src/process.rs`, and the
copy-pasted Windows `LockFileEx` block — which already disagree on whether
`EACCES` means "busy" or "fatal". M003 states one total acquisition order
over the lock tiers, adds a static guard for it, and retires the
`WorkerLock` tier that has no production acquirer but whose metadata the
status surface still reads.

**Snippet CRUD gate:** M001 gives a script a non-interactive delete path by
extracting the existing TUI delete sequence into a shared primitive. Delete
is currently reachable only from the TUI key handler. M002 then gives the
`folders` and `favorite` model fields the write path they lack — they are
read by the list outputs, the search detail output, the TUI, the sort
layer, the fingerprint, and the sync merge, and `--favorites-first` sorts
by a field no command can set — and M003 generalizes the existing
`snp edit --output` path to description, command, and tags.

**Sync cursor gate:** M001 implements ADR-0001 Option A, a server-assigned
strictly increasing per-library write sequence, so the delta cursor stops
depending on timestamp granularity. It is a coordinated three-crate change
with a version bump in each crate, and it leaves the client's merge order,
the server's last-write-wins rule, and the existing `visible_at` timestamp
cursor untouched.

**Next handoff:** The five plans registered by the 2026-10-22 research pass
have all been implemented. The CLI/library/selector/sync consolidation M006
search-parity corrective closed the verification gap against M004 and the
subsystem roadmap is now complete. The sync cursor protocol M002 added the
bootstrap, which discharges M001's named condition. Six further plans are
registered and dependency-ready — locking convergence M002 and M003,
maintenance consolidation M002 and M003, and snippet CRUD surface M002 and
M003. The natural next handoff is the locking convergence M002 kernel-lock
protocol convergence, because M003 is hard on it and both touch the same
file; the snippet CRUD surface M002 is the next user-visible capability.

**MCP gate:** M001 closed the read-only local adapter plus registration
against the canonical selector contract. No execution, mutation,
networked, or daemonized MCP surface exists.

**Dependency-refresh gate:** M001 closed the coordinated `eggfetch-core` and
`eggserve-server`/`eggserve-primitives` upgrade as a manifest-and-lockfile
change with zero source edits. Real-socket HTTP parity, transfer, and
release-binary byte-identity were re-proven on the new pins; the exact
versions are pinned in `Cargo.toml` and guarded by `tests/architecture.rs`.

**Sync cursor gate:** ADR-0001 is `accepted`. M001 landed the
server-assigned per-library sequence, the additive proto fields, and the
client cursor; M002 landed the bootstrap and the client's recorded cursor
kind. The coordinated `snip-proto` 0.1.4 / `snip-sync` 0.1.8 / `snip-it`
1.4.1 version bumps are in place — `snip-proto` is unchanged because no
protocol field was added, removed, or retyped. **The cursor is reachable**:
a request that carries no cursor at all is the bootstrap and is answered with
the sequence watermark, the client adopts it, and it sends it from then on. A
timestamp-only delta is still answered with zero, because a delta does not
deliver the library's whole history. M001's named condition is discharged.

**Next handoff:** None pending. All six workstreams are closed and no work is
blocked; new work starts via `plans/README.md` lifecycle step 1 (identify
canonical sections) rather than from this registry.

## Blocked work

| Subsystem | Milestone | Blocker |
|---|---|---|
| — | — | No blocked work. |

## Closure work and current control points

| Subsystem | Status | Controlling evidence |
|---|---|---|
| Distribution and release M001-M005 | closed | `plans/closure/distribution-release/001-status.md` through `005-status.md`; implementation files under `plans/implementation/distribution-release/`; predecessors `plans/archive/flat-001-release-binary-matrix-and-artifact-contract.md`, `flat-002-bootstrap-installers.md`, `flat-006-windows-ci-platform-closure.md`, `flat-007-release-publication-and-distribution-closure.md`, `flat-013-snp-binary-publication-and-installer-corrective-closure.md` |
| Server lifecycle + HTTP M001-M004 | closed | `plans/closure/server-lifecycle-http/001-status.md` through `004-status.md`; implementation `10aeb994`, hosted run `36011491151`; predecessors `flat-003`/`flat-018`/`flat-019`/`flat-020` |
| Updater transport M001-M005 | closed | `plans/closure/updater-transport/001-status.md` through `005-status.md` (21 focused updater tests at adoption; byte-identical 6,776,320; server trial +34.23% discarded); predecessors `flat-004`/`flat-014`/`flat-015`/`flat-016`/`flat-017` |
| CLI/library/selector/retry M001-M005 | closed | `plans/closure/cli-library-sync-consolidation/001-status.md` through `005-status.md`; predecessors `flat-008`-`flat-012` |
| CLI/library/selector/retry M006 | closed | `plans/implementation/cli-library-sync-consolidation/006-search-parity-verification-corrective.md`; corrective pass over the closed M004; closure record `plans/closure/cli-library-sync-consolidation/006-status.md` |
| Maintenance consolidation M001 | closed | `plans/closure/maintenance-consolidation/001-status.md`; implementation `plans/implementation/maintenance-consolidation/001-dead-subsystem-removal-and-doc-reconciliation.md`; roadmap `plans/subsystems/maintenance-consolidation-roadmap.md`; M002 and M003 plans are authored when those milestones become dependency-ready |
| Locking convergence M001 | closed | `plans/closure/locking-convergence/001-status.md`; implementation `plans/implementation/locking-convergence/001-owned-record-lock-protocol-unification.md`; roadmap `plans/subsystems/locking-convergence-roadmap.md`; M002 and M003 plans are authored when those milestones become dependency-ready |
| Snippet CRUD surface M001 | closed | `plans/closure/snippet-crud-surface/001-status.md`; implementation `plans/implementation/snippet-crud-surface/001-non-interactive-snippet-delete.md`; roadmap `plans/subsystems/snippet-crud-surface-roadmap.md`; M002 and M003 plans are authored when those milestones become dependency-ready |
| Sync cursor protocol M001 | conditionally closed (condition discharged by M002) | `plans/closure/sync-cursor-protocol/001-status.md`; implementation `plans/implementation/sync-cursor-protocol/001-monotonic-per-library-sequence-cursor.md`; roadmap `plans/subsystems/sync-cursor-protocol-roadmap.md`; ADR `plans/adrs/ADR-0001-sync-delta-cursor.md` (`accepted`); **named condition discharged on 2026-10-22** by `plans/closure/sync-cursor-protocol/002-status.md`, which supplies the closing evidence (a client that synced against a sequence-capable server sends a non-zero `last_sync_sequence` and the server answers with a non-zero `server_sequence`) |
| Sync cursor protocol M002 | closed | `plans/closure/sync-cursor-protocol/002-status.md`; implementation `plans/implementation/sync-cursor-protocol/002-cursor-bootstrap-and-precision-convergence.md`; roadmap `plans/subsystems/sync-cursor-protocol-roadmap.md`; no protocol field changed, so `snip-proto` stays 0.1.4 |
| MCP integration M001 | closed | `plans/closure/mcp-integration/001-status.md`; predecessors `flat-005` plus MCP direction in `flat-000` |
| Eggstack dependency refresh M001 | closed | `plans/closure/eggstack-dependency-refresh/001-status.md`; implementation `plans/implementation/eggstack-dependency-refresh/001-upgrade-eggfetch-and-eggserve.md`; roadmap `plans/subsystems/eggstack-dependency-refresh-roadmap.md` |
| Pre-convention direction plan 000 | archived | `plans/archive/flat-000-distribution-fleet-and-mcp-roadmap.md`, superseded by canonical `000`/`001`/`002` plus the `distribution-release` and `mcp-integration` roadmaps |

## Starting new work

1. Identify the relevant canonical sections and invariants.
2. Record any unresolved architectural decision in `plans/adrs/`.
3. Create or update the subsystem roadmap in `plans/subsystems/`.
4. Select one dependency-ready milestone and write a bounded handoff plan
   under `plans/implementation/<subsystem>/`.
5. Register it above (dependency-ready table) before handing it to an
   agent; move `ready` → `active` → `closing` → `closed` with a closure
   record each time.
6. On closure, audit blocked work: unblock dependents in the same commit
   and record the audit in the closure record.
