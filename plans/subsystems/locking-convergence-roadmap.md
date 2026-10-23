# Locking Protocol and Ordering Convergence Roadmap

Status: active

> **2026-10-22 — M001 closed.** The milestone's implementation plan
> (`plans/implementation/locking-convergence/001-owned-record-lock-protocol-unification.md`)
> was executed and its closure record is
> `plans/closure/locking-convergence/001-status.md`. The roadmap changed only
> in its status field and in §12: the M001 objective, deliverable boundary,
> exit conditions, and milestone history are unchanged. The closure record
> corrects three statements this roadmap's evidence section inherited from
> the code: the two copies differed in **three** arms rather than one, the
> transaction quarantine has always been ownership-verified, and the
> owned-record locks release on `Drop` by ownership-verified removal — not
> "without unlinking", as this roadmap's §4 states. That wording correction
> is assigned to M003.

Long-term references:

- `plans/000-long-term-specification.md#5-durability-invariants`
- `plans/000-long-term-specification.md#6-sync-invariants`
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules`
- `plans/001-terminology-and-domain-model.md#4-library-and-persistence-terms`
- `plans/001-terminology-and-domain-model.md#5-sync-and-execution-terms`
- `plans/002-long-term-roadmap.md#phase-3-cli-library-selector-and-sync-policy-consolidation`
- `plans/003-planning-process.md`

Related ADRs: none required yet. This work converges existing locking
policy onto one implementation without changing which resources are locked,
how long a lock is held, or the semantics of a stale lock. If a milestone
would change locking authority, fencing, or the auto-sync/sync relationship
— the ADR threshold in `plans/003-planning-process.md` §2.2 — the milestone
stops and an ADR is opened before implementation continues.

## 1. Purpose and ownership boundary

This subsystem owns the implementation of locking policy: the owned-record
lock protocol used for local data and transaction journals, the kernel-backed
file-lock protocol used for auto-sync and server singleton locks, and the
documented order in which those locks are acquired.

It consumes the durability classes and the mutation gate. It MUST NOT own
the durability classes themselves, the mutation gate semantics, the sync
cursor protocol (`plans/subsystems/sync-cursor-protocol-roadmap.md`), the
backup/snapshot copy policy
(`plans/subsystems/maintenance-consolidation-roadmap.md`), the sync merge
order, or the auto-sync scheduling policy.

Locking policy is currently owned by the modules it protects. That
ownership model is the root cause of the duplication this subsystem
addresses.

## 2. Work classification

### Invariants

- Kernel locks are authoritative. A lock file's presence proves nothing; the
  kernel's answer does.
- `Drop` releases a lock without unlinking it. Lock files may therefore hold
  stale metadata by design.
- `kill(pid, 0)` proves absence only for `ESRCH`. `EPERM` and unknown errors
  mean the process is alive.
- Linux start tokens are read from `/proc/<pid>/stat` field 22.
- The mutation gate runs before every local mutation, and a corrupt or
  multi-journal state fails closed.
- Lock acquisition order is total and documented, so no two acquisition
  paths can form a cycle.

### Capabilities

- Unchanged observable locking behavior from the user's and the operator's
  perspective. Contention, staleness, and recovery outcomes are preserved.

### Infrastructure

- One owned-record lock implementation used by both the local-data lock and
  the transaction lock.
- One kernel-lock protocol used by the client and the server.
- One documented acquisition order that matches the code.

### Polish

- Duplicated helpers and duplicated self-copies removed rather than
  tolerated.

## 3. Non-goals

- No new locking crate, distributed lock service, consensus protocol, or
  lease manager.
- No change to which resources are locked, to lock scope, or to the
  auto-sync/sync execution relationship.
- No change to the `SyncExecutionLock` shared by manual sync and the
  detached auto-sync worker.
- No relaxation of the mutation gate or of the fail-closed behavior on
  corrupt or multiple journals.
- No change to lock file names, locations, or on-disk formats except where a
  milestone explicitly names the change.
- No unification of the server's separate `flock` protocol with the
  client's `LockFileEx` implementation beyond removing the duplicated copy
  of the client protocol.

## 4. Current state

Locking is the most heavily duplicated subsystem in the repository, and the
duplication has already produced behavioral divergence.

**The owned-record protocol is implemented twice.** `src/transaction.rs` and
`src/local_data.rs` each implement the same protocol: create a lock record
with `create_new`, write an identity payload, `sync_all`, treat an empty
file as a lost race and retry, treat a malformed file as corruption, obtain
`ProcessIdentity::observe` on the owner, quarantine a stale record, and
release on `Drop` without unlinking. The two copies share a helper whose
bodies are byte-identical — `is_transient_lock_contention`, whose Windows
variant treats `PermissionDenied` as transient contention and whose Unix
variant returns false — but the copies have since diverged in the one
behavior that matters most: the transaction lock fails fast when it finds a
live owner, while the local-data lock waits with bounded backoff. A caller
that cannot distinguish "the lock is held by a live process" from "the lock
implementation differs from the one you read" will handle the same contention
differently depending on which lock it asked for.

**The kernel-lock protocol is implemented four times.** The canonical
implementation is `src/process_file_lock.rs`. `snip-sync/src/server_lock.rs`
carries a mirror of it. `snip-sync/src/process.rs` carries a third copy used
for the server's `croncheck.lock`, which has no entry in
`architecture/process_file_lock.md`'s protocol contract. The Windows
`LockFileEx` block is copy-pasted three times. The copies have already
diverged on what counts as contention: the canonical implementation treats
`EWOULDBLOCK`, `EAGAIN`, and `EACCES` as busy; the server mirror retries
`EINTR` instead and treats only `EWOULDBLOCK` as busy; the `process.rs`
copy accepts only `WouldBlock`. `EACCES` is therefore busy in one copy and
a hard error in another, and a signal-interrupted acquisition is retried in
one copy and fatal in another.

**`src/auto_sync/execution_lock.rs` duplicates itself.** `inspect` and
`worker_inspect` have identical bodies, as do `is_stale` and
`worker_is_stale`. The `process_alive` helper is defined twice with
identical bodies under mutually exclusive `cfg` attributes. Three separate
error-mapping functions re-implement the same `UnsupportedPlatform` to
`Unsupported` conversion that `src/auto_sync/pending_lock.rs` also
implements.

**A documented tier has no acquirer.** `auto_sync::execution_lock::WorkerLock`
has no production acquisition path — `try_acquire_worker` and
`wait_acquire_worker` are called only from tests — yet its metadata is read
by `src/status_snapshot.rs`, and its path is surfaced by
`src/auto_sync/mod.rs` and `src/commands/sync_cmd.rs`. The status view can
therefore never report a live owner for a lock that its own surface offers
to the user. The architecture documents disagree with each other:
`architecture/process_file_lock.md` lists the tier as a live consumer while
`architecture/auto_sync.md` correctly records that no production path
acquires it.

**The acquisition order is documented in two disconnected places and does
not match the code.** `architecture/process_file_lock.md` records the
`LocalDataLock` → `TransactionLock` → writes order, and
`architecture/auto_sync.md` records three separate auto-sync acquisition
paths. Nothing merges them, and neither states the order that actually
occurs when a manual sync runs under the execution lock. The code takes the
execution lock, and beneath it the local-data lock, and beneath that the
transaction lock — `Execution → LocalData → Transaction` — and separately
`Execution → Pending` on the worker path. `src/library/persistence.rs`
acquires the transaction lock before the local-data lock, the reverse tier
order. The two orders do not deadlock because the guards are not nested
across those two sites, but the documented tiers do not describe the
acquisition order that the code performs.

**One documented lock behavior is stale.** `architecture/overview.md` and
`architecture/process_file_lock.md` describe the transaction lock's stale
reclaim as taking only a path and renaming unconditionally. The code is
ownership-verified: it reads the lock record and compares it against the
observed owner before reclaiming.

## 5. Target architecture

One owned-record lock implementation, parameterized by the wait policy the
caller requires, used by both the local-data lock and the transaction lock.
The two callers then differ only where their contracts genuinely differ:
how long they wait, and what they do on timeout.

One kernel-lock protocol, implemented once per platform with a shared
interface, used by the client's auto-sync locks, the server's singleton
lock, and the server's `croncheck.lock`. The busy/error classification is
defined once.

One documented acquisition order, stated as a single total order over the
lock tiers, that matches the code and is checked by a static guard.

No lock tier whose acquisition path does not exist. A tier that is retired
is deleted from the code, from the status surface, and from the
architecture documents together.

## 6. Dependency graph

```text
M001 Owned-record lock protocol unification
    |
    +--> M002 Kernel-lock protocol convergence (hard: none; soft: M001)
    |
    `--> M003 Lock ordering contract and static guard (hard: M001, M002)
```

M001 first: it establishes the shared primitive that M003's ordering
contract must describe. M002 is independent of M001 — the two protocols
have no code in common — and may proceed in either order. M003 is last
because the ordering guard can only be written once the set of locks and
their acquisition sites are final.

## 7. Milestones

### M001 — Owned-record lock protocol unification

Class: infrastructure

Objective: implement the owned-record lock protocol once and have both the
local-data lock and the transaction lock use it, with the difference
between them expressed as an explicit wait policy rather than as two
divergent code paths.

Dependencies: none.

Deliverable boundary: one shared owned-record lock primitive carrying the
record creation, identity write, `sync_all`, empty/malformed ladder,
`ProcessIdentity::observe`, quarantine, and `Drop`-release steps; the
transaction lock keeps its fail-fast-on-live-owner contract and the
local-data lock keeps its bounded-wait contract, both expressed as
parameters of the shared primitive; the byte-identical
`is_transient_lock_contention` helper exists once.

User or operator value: contention behaves the same way for both locks
except where the contract says it must differ, and a future fix to the
protocol cannot land in one copy only.

Exit conditions: no second owned-record protocol implementation exists;
the two callers select their wait policy explicitly; the existing
transaction, local-data, and repair integration tests remain green; the
lock barrier suites still pass serially.

Deferred work: kernel-lock convergence (M002), ordering contract (M003).

Implementation plan:

- `plans/implementation/locking-convergence/001-owned-record-lock-protocol-unification.md`

Closure record: `plans/closure/locking-convergence/001-status.md` (closed).

Predecessor flat plan: none.

### M002 — Kernel-lock protocol convergence

Class: infrastructure

Objective: implement the kernel-backed file-lock protocol once per platform
and use it for the client's auto-sync locks, the server's singleton lock,
and the server's `croncheck.lock`, with one definition of which errors mean
contention.

Dependencies: hard none; soft M001 (shared convention for duplicate
removal).

Deliverable boundary: one Windows `LockFileEx` implementation and one Unix
`flock` implementation, each used by every caller that needs it; one
busy-versus-error classification shared by all callers; the `croncheck.lock`
copy in `snip-sync/src/process.rs` and the mirror in
`snip-sync/src/server_lock.rs` removed in favor of the shared protocol; the
self-duplicated `inspect`/`worker_inspect` and `is_stale`/`worker_is_stale`
pairs collapsed to one implementation each.

User or operator value: a signal-interrupted acquisition behaves the same
everywhere, and `EACCES` no longer means "busy" in one place and "fatal" in
another.

Exit conditions: exactly one implementation of each platform's lock
primitive exists in the workspace; the busy classification is defined once
and referenced by every caller; the server singleton, auto-sync, and
`croncheck` paths all pass their existing lifecycle and concurrency tests;
the server lifetime wire-parity tests remain unchanged.

Deferred work: ordering contract (M003).

Implementation plan: not yet written — authored when this milestone
becomes dependency-ready.

Closure record: to be written at closure.

Predecessor flat plan: none.

### M003 — Lock ordering contract and static guard

Class: invariant

Objective: state the total acquisition order over the lock tiers in one
place, make the code match it, and add a static guard that fails when a new
acquisition site violates the order.

Dependencies: hard M001, M002.

Deliverable boundary: one merged ordering statement covering the
execution lock, the local-data lock, the transaction lock, and the pending
marker lock; the acquisition sites reconciled to that order; the retired
`WorkerLock` tier removed from the code, from the status surface, and from
the architecture documents; a static guard in `tests/architecture.rs` that
asserts the ordering.

User or operator value: a future lock acquisition cannot silently create a
cycle, and the documented order is the order the code performs.

Exit conditions: one ordering statement exists and the two documents that
previously carried partial orders now point to it; the static guard fails
when a synthetic out-of-order acquisition is introduced; the stale
"renames unconditionally" description of the transaction-lock reclaim is
corrected to the ownership-verified behavior; no lock tier in the
architecture documents lacks an acquirer, and no acquirer is missing from
the architecture documents.

Deferred work: none within this subsystem.

Implementation plan: not yet written — authored when this milestone
becomes dependency-ready.

Closure record: to be written at closure.

Predecessor flat plan: none.

## 8. Cross-cutting requirements

### Storage and migration

No schema migration. Lock file names, directories, and payload formats are
unchanged, so locks taken by an older build remain recognizable to a newer
one and vice versa. The retired `WorkerLock` tier's file, if one exists in a
user's configuration directory, is left in place and ignored; it is not
deleted, because lock files are not unlinked by design.

### Protocol and compatibility

No wire-protocol change. The server's HTTP and gRPC surfaces are untouched.
No CLI change. The status surface continues to report the same fields; a
retired tier contributes no field, because it never reported a live owner.

### Security and authorization

No credential or sync-metadata exposure. A lock payload contains only
process identity. Quarantine paths that move a stale lock record must
preserve the ownership check: reclaiming a lock whose owner is still alive
would let two processes write the same file concurrently.

### Concurrency, cancellation, and recovery

This is the subsystem's central concern. M001 must preserve both wait
policies exactly, because they are contract-visible: a caller that expects
fail-fast must not begin waiting, and a caller that expects bounded waiting
must not fail early. M003's static guard must not be satisfiable by a
change that merely reorders source lines; it must observe the acquisition
relationship.

### Observability and audit

Lock acquisition failures remain distinguishable in diagnostics: contention
is not corruption, and a malformed record is not contention. No diagnostic
message may include a credential, and lock diagnostics remain subject to
the `SnipError::Command` display-layer redaction rule.

### Performance and resource use

No new runtime systems. M002's shared protocol must not add a syscall on
the uncontended path relative to the copies it replaces.

### Documentation and operations

`architecture/process_file_lock.md`, `architecture/overview.md` §7,
`architecture/auto_sync.md`, `architecture/commands/sync_cmd.md`,
`architecture/status.md`, `docs/LOGICAL_LAYERS.md`, and the deep-dive index
are updated with the code. `docs/LOGICAL_LAYERS.md` and
`tests/architecture.rs` are updated together, per their own instruction.

## 9. Verification strategy

Contention tests that run the local-data and transaction locks against a
held owner and assert each keeps its documented wait policy; a test that
pins the busy-versus-error classification for each platform error kind; the
serial contention and barrier suites (`process_lock_concurrency`,
`local_data_lock_barriers`, `auto_sync_concurrency`) with
`--test-threads=1`; a static guard test for the acquisition order; the
`snip_sync_lifetime` wire-parity suite; `scripts/check.sh`.

## 10. Risks and decision points

- Unifying the owned-record protocol risks changing contention behavior for
  one of the two callers if the wait policy is not parameterized explicitly.
  Decision: the wait policy is a required parameter, not a default, so a
  caller cannot silently inherit the other's behavior.
- Collapsing the kernel-lock copies changes which errors are retried. If
  the `croncheck.lock` path depends on `EINTR` being fatal, unification
  changes its behavior. Decision: unification adopts the most permissive
  retry, and each caller's behavior change is recorded in the closure
  record.
- Retiring the `WorkerLock` tier removes a status field that has always
  been empty. Decision: acceptable, because it never reported a live owner
  and its removal is documented in the same change.
- No decision in this subsystem currently requires an ADR. A change to lock
  authority, fencing, or the auto-sync/sync relationship is an ADR trigger
  and stops the milestone.

## 11. Completion definition

This workstream closes when M001, M002, and M003 each have accepted closure
evidence showing that one owned-record protocol and one kernel-lock
protocol exist, that the acquisition order is documented in one place and
enforced by a static guard, that no lock tier lacks an acquirer or an
acquirer lacks documentation, and that no unresolved medium-or-higher
finding remains.

## 12. Milestone status

| Milestone | Status | Implementation plan | Closure record | Blockers |
|---|---|---|---|---|
| M001 owned-record lock protocol unification | closed | `plans/implementation/locking-convergence/001-owned-record-lock-protocol-unification.md` | `plans/closure/locking-convergence/001-status.md` | — |
| M002 kernel-lock protocol convergence | not started | — | — | M001 closed |
| M003 lock ordering contract + static guard | not started | — | — | M001 closed, M002 |
