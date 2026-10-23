# Locking Protocol and Ordering Convergence Milestone 001 — Owned-record lock protocol unification

Status: ready

Repository baseline: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Source roadmap:

- `plans/subsystems/locking-convergence-roadmap.md#m001--owned-record-lock-protocol-unification`

Long-term requirements:

- `plans/000-long-term-specification.md#5-durability-invariants`
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules`
- `plans/001-terminology-and-domain-model.md#4-library-and-persistence-terms`
- `plans/001-terminology-and-domain-model.md#5-sync-and-execution-terms`

Applicable ADRs: none.

Primary class: infrastructure

## 1. Objective

Implement the owned-record lock protocol once and have both the local-data
lock and the transaction lock use it, expressing the difference between them
as an explicit wait policy rather than as two divergent copies of the
protocol.

## 2. Why this milestone is ready

- No hard dependencies. The locking subsystem has no upstream milestone, and
  the six registered subsystem roadmaps are closed.
- No unresolved decision requires an ADR: the milestone consolidates an
  existing protocol onto one implementation. It does not change which
  resources are locked, how long a lock is held, who may reclaim a stale
  lock, or the auto-sync and sync authority relationship.
- The two callers' contracts are already known and differ in exactly one
  respect: the wait policy on a live owner. That difference is expressible
  as a parameter, so the milestone is one coherent pass.
- The lock barrier and contention suites already exist and already pin the
  behavior this milestone must preserve.

## 3. Current implementation evidence

`src/transaction.rs` and `src/local_data.rs` each implement the same
owned-record lock protocol independently.

Both follow the same sequence. Both create the lock file with
`OpenOptions::new().write(true).create_new(true)`, treating
`AlreadyExists` and the transient contention kinds as "the lock exists". Both
pre-serialize the record with `toml::to_string_pretty` before entering the
acquisition loop, and write the record to the open file handle immediately to
minimize the empty-file window between the `create_new` success and the
content write. Both call `sync_all` on the record and downgrade a sync
failure to a warning rather than an error. Both remove the file when the
write fails, because leaving an empty file behind would make every later
acquirer spin through the empty-content ladder. Both then read the existing
record and classify it: empty content retries, malformed content retries
with a bounded counter, a dead owner is reclaimed, and a live owner is
handled by the caller's wait policy. Both use the same conservative
ownership rule: reclaim only when the recorded start token and the observed
start token are both present and differ, which is the PID-reuse case;
otherwise the lock is treated as held. Both quarantine a reclaimed lock
through a helper that takes the path and the observed record, and both
release on `Drop` without unlinking.

The helper `is_transient_lock_contention` exists in both files with
byte-identical bodies: the Windows variant returns true for
`PermissionDenied`, and the Unix variant returns false, because Unix has no
pending-delete aliasing.

The two implementations have diverged in the wait policy, which is the one
behavior that is contract-visible:

- `src/transaction.rs` fails fast. When the owner is alive and ownership
  cannot be disproved, it returns a "Transaction lock held" error naming
  the operation and the owning PID. Its deadline is a flat
  `LOCK_ACQUIRE_TIMEOUT` measured from the start of the acquisition loop.
- `src/local_data.rs` waits. When the owner is alive, it extends its
  deadline each time the lock changes hands — measuring time without
  progress rather than elapsed time — and sleeps with an exponential
  backoff capped at 250 ms. It returns a "Local data lock held" timeout
  error only when the lock fails to change hands within the window.

The consequence is that the same repository contains two implementations of
one protocol whose only difference is the parameter that ought to have been
a parameter. A future fix to the protocol — a new field in the record, a
change to the empty-file window, a change to the ownership rule — must be
made twice, and nothing in the type system or the test suite prevents the
two copies from drifting further.

Known gaps at the baseline: the divergence above; the duplicated helper; and
the absence of any guard that would notice a third copy of the protocol
appearing.

## 4. Invariants that must not regress

- Kernel locks are authoritative. A lock file's presence proves nothing.
- `Drop` releases a lock without unlinking it. Lock files may hold stale
  metadata by design.
- `kill(pid, 0)` proves absence only for `ESRCH`. `EPERM` and unknown
  errors mean the process is alive.
- Linux start tokens are read from `/proc/<pid>/stat` field 22.
- The conservative ownership rule is preserved exactly: reclaim a lock only
  when the recorded start token and the observed start token are both
  present and differ. An unverifiable owner is a live owner.
- The transaction lock fails fast on a live owner. The local-data lock waits
  with a progress-extended deadline and bounded backoff. Both contracts are
  observable and must not change.
- The empty-file window is minimized: the record is written to the open
  handle immediately after `create_new` succeeds, and the file is removed
  if that write fails.
- A record write failure surfaces as an error; a record sync failure is a
  warning and does not fail the acquisition.
- Lock acquisition is bounded. No error kind may wedge the acquisition loop
  forever.
- The mutation gate still runs before every local mutation, and a corrupt
  or multi-journal state still fails closed.
- No diagnostic message contains a credential.

## 5. Scope

### In scope

- Extract the shared owned-record lock protocol into one implementation.
- Parameterize the wait policy so the transaction lock fails fast and the
  local-data lock waits with a progress-extended deadline.
- Keep the two lock record types distinguishable — the transaction record
  carries an `operation` field that the local-data record does not — while
  sharing the protocol.
- Have `acquire_transaction_lock` and `acquire_local_data_lock` become
  thin wrappers over the shared implementation.
- Collapse `is_transient_lock_contention` to one definition.
- Keep each caller's error messages, so "Transaction lock held" and "Local
  data lock held" continue to distinguish the two conditions.
- Preserve the on-disk record format and the lock file names and locations
  exactly.
- Add a static guard that fails when a second owned-record lock
  implementation appears in the client crate.

### Explicitly out of scope

- The kernel-lock protocol used by auto-sync and the server singleton.
  That belongs to
  `plans/subsystems/locking-convergence-roadmap.md` M002.
- The acquisition-order contract and its static guard. That belongs to M003
  of the same roadmap.
- The `WorkerLock` tier's retirement. That belongs to M003 as well.
- Any change to the lock file names, directories, record schemas, or the
  `schema_version` value inside a record.
- Any change to which resources are locked or to the mutation gate.
- Any change to the quarantine path, the stale-detection policy, or the
  PID-reuse rule.
- The `unix_now_ms` duplication across modules, which is not part of this
  protocol.
- The server's separate lock implementation in `snip-sync`.

## 6. Required production changes

### Core/domain

One shared owned-record lock implementation owns the protocol. Its shape is
an ordinary Rust type, not a trait hierarchy and not a framework. It
carries: the lock path; the serialized record content; the acquisition
deadline policy; the wait policy; the empty-retry and malformed-retry
counters; and the record-specific error message.

The record payload type is supplied by the caller, because the transaction
record carries an `operation` field that the local-data record does not.
The shared implementation must not grow a field that only one caller needs;
where a caller needs extra record content, the caller supplies it.

The wait policy is a required parameter, not a default, so that a caller
cannot silently inherit the other's contract. The two policies are:

- fail-fast: a live owner returns the caller's held-lock error immediately.
- wait-for-progress: a live owner extends the deadline each time the lock
  changes hands, sleeps with the existing bounded backoff, and returns the
  caller's timeout error when the lock stops changing hands.

### Storage and migrations

No schema change. The lock records keep their current fields, their current
`schema_version` value, and their current serialization. The lock file
names and directories are unchanged, so a lock taken by a build from before
this milestone is recognizable to a build from after it.

### Protocol and DTOs

No protocol change. `snip-proto` and `snip-sync` are unaffected.

### Runtime and concurrency

The acquisition loop keeps its current shape and its current bounds. The
shared implementation preserves, in order: create with `create_new`;
classify `AlreadyExists` and the transient contention kinds as "the lock
exists"; write the pre-serialized record to the open handle; `sync_all`
with a warning on failure; remove the file and error if the write fails;
read and classify the existing record; empty content retries; malformed
content retries with a bounded counter; a dead owner is reclaimed; a live
owner is handled by the wait policy.

The quarantine step keeps its current ownership verification: the observed
record is passed to the quarantine helper, and a reclaim happens only when
the start tokens are both present and differ.

The `Drop` behavior is unchanged: release without unlinking.

### Frontend or operator surface

No CLI change, no new flag, no new output field. The error messages the
user sees are unchanged, including the distinction between "Transaction lock
held" and "Local data lock held".

### Security and authorization

No credential handling change. A lock record contains only process identity:
the PID, a nonce, a creation timestamp, and a start token. The shared
implementation must not add a field that carries anything else.

Reclaiming a lock whose owner is still alive would let two processes write
the same file concurrently, so the ownership verification is a security
property here, not only a liveness one. It must survive the refactor
unchanged.

### Documentation and static guards

- `architecture/process_file_lock.md`: the owned-record protocol section is
  rewritten to describe the single shared implementation and to state that
  the wait policy is a parameter.
- `architecture/overview.md` §7: the cross-cutting invariant about
  kernel-backed locks is updated to point at the shared implementation.
- `architecture/persistence.md`: if it names either lock implementation
  separately, update it.
- `tests/architecture.rs`: a static guard asserting that the owned-record
  acquisition protocol exists in exactly one place in the client crate.

## 7. Ordered work packages

### Work package A — Extract the shared protocol

Intent: move the duplicated protocol body into one implementation without
changing what it does.

Required changes: create the shared implementation carrying the acquisition
loop, the record write and sync, the empty and malformed ladders, the
owner classification, and the quarantine call; parameterize it with the
record payload, the caller-supplied held-lock and timeout errors, and the
wait policy; keep the `Drop`-without-unlink behavior in the shared type.

Acceptance evidence: the shared implementation contains the protocol body
exactly once; the transaction and local-data acquisition functions contain
only their own record construction and their call into the shared
implementation; `cargo clippy --workspace --all-targets -- -D warnings`
passes.

### Work package B — Wire both callers to the shared implementation

Intent: give the two callers their correct, distinct contracts through the
shared implementation.

Required changes: `acquire_transaction_lock` passes the fail-fast policy and
its own held-lock error; `acquire_local_data_lock` passes the
wait-for-progress policy, the base and maximum wait windows, and the poll
backoff ceiling, and its own timeout error; collapse
`is_transient_lock_contention` to one definition used by both; keep the two
record types and their `operation` field difference intact.

Acceptance evidence: the transaction lock still fails fast with its
original error text when the owner is alive; the local-data lock still
extends its deadline when the lock changes hands and still times out with
its original error text when it does not; both record formats are
byte-identical to the baseline for a given identity.

### Work package C — Add the static guard

Intent: prevent a third copy of the protocol from appearing.

Required changes: add to `tests/architecture.rs` an assertion that the
owned-record acquisition protocol — identified by its distinguishing
operations, such as the `create_new` acquisition and the record write
followed by `sync_all` — appears in exactly one module in `src/`;
make the assertion tolerant of the shared implementation's exact location
but strict about the count.

Acceptance evidence: the guard passes as committed; the guard fails when a
second copy is introduced; the guard's negative case is proven in the test
itself rather than assumed.

### Work package D — Update the architecture documents

Intent: make the documents describe the shared implementation.

Required changes: rewrite the owned-record protocol section of
`architecture/process_file_lock.md`; update `architecture/overview.md` §7;
check `architecture/persistence.md` and `architecture/commands/` deep dives
for references to either implementation separately.

Acceptance evidence: no architecture document describes two independent
owned-record protocol implementations; the deep-dive index in
`architecture/overview.md` is consistent with the file set.

## 8. Failure, cancellation, restart, and contention semantics

**Partial failure.** If the record write fails, the created file is removed
and the error is returned; this behavior is preserved exactly, because
leaving an empty file behind makes every later acquirer spin through the
empty-content ladder. If the record sync fails, the acquisition still
succeeds and the failure is logged as a warning.

**Duplicate delivery.** Not applicable: a lock is acquired by creating the
file, and the create is atomic.

**Process restart.** A lock whose owner process is dead is reclaimed on the
next acquisition. A lock file left behind by a crash is therefore not a
permanent blocker. This behavior is unchanged.

**Cancellation races.** The acquisition loop is bounded by a deadline that
the shared implementation computes from the supplied policy. Neither caller
may end up with an unbounded loop: the flat deadline for the transaction
lock and the progress-extended deadline with a hard cap for the local-data
lock both bound the wait.

**Stale generations.** Not applicable to this milestone; the local-data
lock's generation handling is unchanged.

**Concurrent callers.** Two concurrent acquirers of the same lock path: one
wins the `create_new` and the other takes the "lock exists" branch and
classifies the owner. The classification and the wait policy decide whether
the second caller waits or fails. This behavior is preserved exactly for
both callers.

**Stale owner reclamation.** A reclaimed lock is quarantined by renaming,
and the rename is ownership-verified against the observed record. A
concurrent acquirer that recreates the lock between the rename and the
retry must not have its new lock quarantined. That TOCTOU handling is
preserved exactly.

## 9. Compatibility and migration

No data migration. No stored artifact changes. No lock record format
changes.

No configuration fallback is needed. No protocol negotiation is needed.

The removal criteria for the legacy paths are the milestone's exit
conditions: once both callers use the shared implementation and the static
guard exists, the duplicated bodies are deleted rather than left in place
behind a suppression. There is no deprecation window, because the
duplicated bodies are private implementation, not a published surface.

## 10. Required tests

### Focused unit tests

- A unit test asserting that the fail-fast policy returns the caller's
  held-lock error immediately when the owner is alive and ownership cannot
  be disproved.
- A unit test asserting that the wait-for-progress policy extends its
  deadline when the lock changes hands and times out with the caller's
  timeout error when it does not.
- A unit test asserting that an unverifiable owner — a record with no start
  token, or an observed identity with no start token — is treated as a live
  owner, so the lock is never reclaimed in that case.
- A unit test asserting that a record whose recorded and observed start
  tokens both differ is reclaimed as a PID reuse.
- A unit test asserting that a failed record write removes the created file.

### Integration tests

- `tests/local_data_lock_barriers` — the local-data lock barrier suite,
  run with `--test-threads=1`.
- `tests/repair_transactions` — the transaction repair suite, run with
  `--test-threads=1`.
- A test that a lock file survives the release of its owner, since release
  does not unlink.

### Restart and recovery tests

- A test that a lock left behind by a dead process is reclaimed by the next
  acquirer.
- A test that a lock held by a live process is not reclaimed.

### Contention and cancellation tests

- `tests/process_lock_concurrency` — run with `--test-threads=1`.
- `tests/local_data_lock_barriers` — run with `--test-threads=1`.
- `tests/auto_sync_concurrency` — run with `--test-threads=1`, because the
  auto-sync path takes the local-data lock beneath the execution lock.

### Security and negative tests

- A test that a lock record whose owner is alive but unverifiable is never
  reclaimed.
- A test that a corrupt or malformed lock record is quarantined rather than
  silently overwritten, for both callers.
- The static guard's negative case, proving it fails when a second
  implementation is introduced.

### Migration and compatibility tests

- A test that a lock record written by the baseline implementation parses
  under the shared implementation, and vice versa. This proves the record
  format is unchanged.

## 11. Required verification commands

```bash
# narrow tests first
cargo test --workspace --lib
cargo test --test local_data_lock_barriers --features test-support -- --test-threads=1
cargo test --test repair_transactions --features test-support -- --test-threads=1
cargo test --test process_lock_concurrency --features test-support -- --test-threads=1
cargo test --test auto_sync_concurrency --features test-support -- --test-threads=1
cargo test --test architecture --features test-support

# lint gates
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings

# authoritative gate
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

Do not claim commands that were not actually run in the closure record.

The barrier, contention, and repair suites are serial. Never run them with
parallel test threads, and never run a build while a test run is in flight.

## 12. Documentation updates

- `architecture/process_file_lock.md` — the owned-record protocol section.
- `architecture/overview.md` §7 — the kernel-backed and ordered locks
  invariant.
- `architecture/persistence.md` — if it names either implementation
  separately.
- `architecture/commands/repair_cmd.md` — if it describes which lock a
  repair holds.
- `docs/LOGICAL_LAYERS.md` — only if module membership changes; it and
  `tests/architecture.rs` must be updated together.

## 13. Acceptance criteria

1. The owned-record acquisition protocol exists exactly once in the client
   crate.
2. `acquire_transaction_lock` and `acquire_local_data_lock` contain only
   their own record construction and their call into the shared
   implementation.
3. `is_transient_lock_contention` exists exactly once.
4. The transaction lock still fails fast with its original error text when
   the owner is alive; the local-data lock still waits with a
   progress-extended deadline and still times out with its original error
   text.
5. The wait policy is a required parameter, so neither caller can inherit
   the other's contract by omission.
6. The conservative ownership rule is preserved exactly, and the reclaim
   path remains ownership-verified against the observed record.
7. The lock record formats, file names, and locations are byte-identical to
   the baseline, proven by a cross-version record parse test.
8. `Drop` still releases without unlinking.
9. A static guard fails when a second owned-record protocol implementation
   is introduced.
10. `cargo clippy --workspace --all-targets -- -D warnings` passes.
11. `bash scripts/check.sh` and `bash scripts/ci/test-production-seams.sh`
    pass, with the serial suites green under `--test-threads=1`.
12. No architecture document describes two independent owned-record
    implementations.

## 14. Stop conditions

The agent must stop and report rather than improvise when:

- the shared implementation cannot express one caller's contract without
  changing the other's observable behavior;
- preserving a contract would require a lock file name, record field, or
  record format change;
- the conservative ownership rule cannot be expressed once for both
  callers;
- a hard dependency is absent;
- the scope would expand into the kernel-lock protocol, the ordering
  contract, or the `WorkerLock` retirement, all of which belong to other
  milestones;
- repository evidence contradicts the claim that the two implementations
  differ only in their wait policy.

## 15. Closure evidence required

- The `cargo clippy --workspace --all-targets -- -D warnings` and
  `--all-features` results.
- The `bash scripts/check.sh` and
  `bash scripts/ci/test-production-seams.sh` results, including the serial
  suites.
- The record-format cross-version parse test result.
- The fail-fast and wait-for-progress contract test results, showing each
  caller's error text.
- The static guard's negative-case result.
- A statement listing the modules that now contain the protocol, with a
  search command proving the count is one.
- A statement that the ownership rule and the quarantine verification are
  unchanged, with the test that proves each.

## 16. Handoff notes

- The working tree was clean at the baseline commit
  `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`. Preserve any user changes
  that appear; do not reset unrelated work.
- Never run `cargo build`, `cargo test`, or `cargo clippy` while another
  test run is in flight. The integration tests execute
  `target/debug/snp`; a rebuild mid-run replaces that binary and produces
  false failures.
- `local_data_lock_barriers`, `repair_transactions`,
  `process_lock_concurrency`, and `auto_sync_concurrency` are serial. Use
  `--test-threads=1` for each.
- `SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on all test commands and must
  never be removed.
- In edition 2024, `std::env::set_var` requires `unsafe`.
- Do not change the lock file names or directories. A lock file may hold
  stale metadata by design, and the kernel lock is authoritative.
- `kill(pid, 0)` proves absence only for `ESRCH`; `EPERM` means the process
  is alive.
