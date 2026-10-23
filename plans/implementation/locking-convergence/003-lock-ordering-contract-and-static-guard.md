# Locking Protocol and Ordering Convergence Milestone 003 — Lock ordering contract and static guard

Status: ready

Repository baseline: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Source roadmap:

- `plans/subsystems/locking-convergence-roadmap.md#m003--lock-ordering-contract-and-static-guard`

Long-term requirements:

- `plans/000-long-term-specification.md#5-durability-invariants`
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules`
- `plans/001-terminology-and-domain-model.md#4-library-and-persistence-terms`
- `plans/001-terminology-and-domain-model.md#5-sync-and-execution-terms`

Applicable ADRs: none.

Primary class: infrastructure

## 1. Objective

State the total acquisition order over the client lock tiers in one place,
make the code match it, retire the dead `WorkerLock` tier from the code, the
status surface, and the architecture documents, and add a static guard that
fails when a new acquisition site violates the order.

This milestone does **not** touch the owned-record lock *protocol*. M001
unified that in `src/record_lock.rs` and it is closed; M003 states an order
over the tiers and removes a dead tier. It also does not touch the
kernel-lock protocol — M002 owns that
(`plans/subsystems/locking-convergence/002-kernel-lock-protocol-convergence.md`).

## 2. Why this milestone is ready

- No hard dependencies. The roadmap records M003's dependency as "hard M001;
  soft M002" (`plans/subsystems/locking-convergence-roadmap.md:320-321`).
  M001 is closed (`plans/closure/locking-convergence/001-status.md`). M002 is
  not started, so M003 must not depend on it: the ordering contract and the
  `WorkerLock` retirement are independent of where the kernel primitive lives.
- The acquisition sites and their nesting are already observable (§3), so the
  order can be stated from the code rather than guessed.
- The dead tier is already provably dead: `architecture/auto_sync.md:403`
  records that "**No production path acquires it**", and the code agrees
  (§3.3).
- The static-guard pattern already exists: M001 added three source-scanning
  guards to `tests/architecture.rs` (`:425`, `:438`, `:478`), each with a
  synthetic-fixture test proving the detector can fail.

## 3. Current implementation evidence

### 3.1 The order is documented in three disconnected places

| Document | What it states |
|----------|----------------|
| `architecture/process_file_lock.md:481-482` | "The lock hierarchy is `LocalDataLock → TransactionLock → writes` (`src/commands/restore_cmd.rs:1223-1224`)." |
| `.skills/transactions-and-auto-sync.md:54` | "Acquisition order: **`LocalDataLock` → `TransactionLock` → writes**. Never invert." |
| `architecture/auto_sync.md:406-425` | An "### Acquisition order" section listing three auto-sync paths (parent mutation `:410-413`, worker `:414-419`, explicit sync `:420-422`) and the claim at `:424-425` that "No path acquires the pending lock and then the execution lock. The worker lock is never part of any of these orders because nothing acquires it." |

None of the three states a total order over all four live tiers, and the
first two omit the execution lock and the pending-marker lock entirely. The
roadmap records the same gap
(`plans/subsystems/locking-convergence-roadmap.md:155-160`).

### 3.2 The order the code actually performs

Five nesting edges are observable in the code today. Each is a case where
one tier is held while a lower tier is acquired.

| Edge | Outer tier | Inner tier | Evidence |
|------|-------------|------------|----------|
| Execution → LocalData | `SyncExecutionLock` | `LocalDataLock` | `src/commands/sync_cmd.rs:193` takes the execution lock; the merge calls `library::update_library` at `src/sync_commands.rs:675`, which takes the local-data lock at `src/library/persistence.rs:309` |
| Execution → Pending | `SyncExecutionLock` | `PendingTxnGuard` | `src/commands/mod.rs:432` takes the execution lock and `:442` calls `clear_pending_after_explicit_sync` → `worker::clear_after_explicit_sync` → `pending::clear_if_generation_matches`, which takes the pending lock at `src/auto_sync/pending.rs:241` |
| LocalData → Transaction | `LocalDataLock` | `TransactionLock` | `src/commands/restore_cmd.rs:1223` takes the local-data lock and `:1224` takes the transaction lock |
| LocalData → Pending | `LocalDataLock` | `PendingTxnGuard` | `src/commands/edit_cmd.rs:103` takes the local-data lock and `:132` calls `notify_mutation` → `pending::record_pending_mutation`, which takes the pending lock at `src/auto_sync/pending.rs:77` |
| Transaction → Pending | `TransactionLock` | `PendingTxnGuard` | `src/transaction.rs:771` takes the transaction lock and `:800` calls `pending::ensure_pending_for_transaction`, which takes the pending lock at `src/auto_sync/pending.rs:132` |

Every edge is consistent with exactly one total order:

> **`SyncExecutionLock` → `LocalDataLock` → `TransactionLock` → `PendingTxnGuard`**
> (outermost to innermost)

No site inverts it. The reverse edges are absent by construction:

- **Pending → Execution** does not occur. The worker takes the execution lock
  first (`src/auto_sync/worker.rs:64`) and the pending lock inside it;
  `schedule_and_spawn` starts a *child* that takes the execution lock, so the
  parent's pending lock is never held while the child acquires one.
  `architecture/auto_sync.md:424` states the same.
- **Pending → LocalData** does not occur. The pending lock is held only
  inside the five marker read-modify-write functions in
  `src/auto_sync/pending.rs` (`:77`, `:132`, `:241`, `:262`, `:284`), none of
  which touches the library.
- **Pending → Transaction** does not occur, and **Transaction → LocalData**
  does not occur. `src/transaction.rs` acquires only the transaction lock
  (`:664`, `:771`); the local-data lock is acquired in
  `src/local_data.rs:131` and in the callers listed in §3.4.

One precondition sits outside the order and must stay there:
`gate_mutation_on_interrupted_transactions()` runs **before** any tier is
taken (`src/library/persistence.rs:266-269`, `:302-305`). When it recovers a
journal it acquires and releases the transaction lock internally
(`src/transaction.rs:664`) — that is a *sequential* acquisition, not a nested
one, so it does not invert the order. Making the gate part of the tier order
would change its fail-closed behavior and is out of scope.

### 3.3 The `WorkerLock` tier is dead in the code

| Element | Location | Live consumer |
|---------|----------|----------------|
| `pub struct WorkerLock` | `src/auto_sync/execution_lock.rs:246` | — |
| `worker_lock_path` | `src/auto_sync/execution_lock.rs:273` | `src/auto_sync/mod.rs:43` (`paths::worker_lock`) |
| `try_acquire_worker` | `src/auto_sync/execution_lock.rs:277` | none |
| `wait_acquire_worker` | `src/auto_sync/execution_lock.rs:291` | none |
| `worker_inspect` | `src/auto_sync/execution_lock.rs:283` | `src/status_snapshot.rs` (see below) |
| `worker_is_stale` | `src/auto_sync/execution_lock.rs:287` | `src/status_snapshot.rs` (see below) |
| `WorkerLockError` | `src/auto_sync/execution_lock.rs:223-243` | — |
| `src/auto_sync/lock.rs:3-7` | re-export shim over `execution_lock` | — |

No production path calls `try_acquire_worker` or `wait_acquire_worker`. The
only live consumers of the tier are read-only:

- `src/status_snapshot.rs:321-341` (`inspect_worker_lock`) and the
  `ExecutionSummary.worker_lock` field (`:119`), which feed
  `src/status_snapshot.rs:463-489` (three `sync.worker_lock.*` diagnostics).
- `src/commands/sync_cmd.rs:913-914` — the `snp sync repair`
  permission-hardening branch for the `worker_lock` artifact.
- `src/commands/doctor_cmd.rs:98-109` — three remediations whose artifact is
  `worker_lock` (`sync.worker_lock.dead_stale`,
  `sync.worker_lock.malformed`, `sync.worker_lock.malformed`).

The JSON surface carries the field: `docs/JSON_SCHEMAS.md:170` lists
`execution.worker_lock` inside `status --json`, `:193` describes its
variants, and `:24` names it in the prose. `docs/JSON_SCHEMAS.md:30-31`
states the two schema rules: "7. New fields are additive (non-breaking)."
and "8. Breaking changes increment the `schema` version number."
`src/status_snapshot.rs:15` declares `pub const SNAPSHOT_SCHEMA: u32 = 1;`
and the unit test at `src/status_snapshot.rs:1151-1152` asserts it is `1`.

The human-readable `snp status` output does **not** render the execution
summary at all — `src/commands/status_cmd.rs:36-98` prints only the local
counts, the sync line, the pending generation, attempt timestamps, and the
log directory. Only the JSON output carries `execution.*`
(`src/commands/status_cmd.rs:21-33`), so retiring the tier changes the JSON
surface only.

`architecture/auto_sync.md:403` documents the tier as "(legacy) …
**No production path acquires it**", and
`architecture/diagnostics.md:249-250` documents the three diagnostic codes.

### 3.4 The acquisition sites that must match the order

Every direct acquisition call site, by tier:

| Tier | Acquiring function | Direct call sites |
|------|--------------------|-------------------|
| `SyncExecutionLock` | `src/auto_sync/execution_lock.rs:143` (`try_acquire`), `:152` (`wait_acquire`) | `src/auto_sync/worker.rs:64`; `src/auto_sync/status.rs:339`; `src/commands/mod.rs:432`; `src/commands/sync_cmd.rs:193`, `:541` |
| `LocalDataLock` | `src/local_data.rs:131` (`acquire_local_data_lock`); `src/library/manager.rs:721` (method wrapper) | `src/library/manager.rs:185`, `:425`, `:478`, `:548`, `:572`, `:586`, `:606`, `:622`, `:637`, `:683`, `:748`, `:813`; `src/usage.rs:74`; `src/commands/edit_cmd.rs:103`; `src/commands/backup_cmd.rs:73`; `src/commands/mod.rs:310`, `:360`; `src/commands/restore_cmd.rs:1223`; `src/config/sync_settings.rs:481`; `src/library/persistence.rs:274`, `:309` |
| `TransactionLock` | `src/transaction.rs:962` (`acquire_transaction_lock`) | `src/transaction.rs:664`, `:771`; `src/commands/restore_cmd.rs:1224` |
| `PendingTxnGuard` | `src/auto_sync/pending_lock.rs:110` (`acquire_pending_txn`) | `src/auto_sync/pending.rs:77`, `:132`, `:241`, `:262`, `:284` |

### 3.5 Two stale documentation statements that M003 owns

**The stale "not ownership-verified" description.** The roadmap says
"`architecture/overview.md` and `architecture/process_file_lock.md` describe
the transaction lock's stale reclaim as taking only a path and renaming
unconditionally"
(`plans/subsystems/locking-convergence-roadmap.md:169-172`) and assigns the
correction to M003 (`:311-312`).

**That premise is stale.** M001 already corrected both documents:
`architecture/overview.md:491` and `architecture/process_file_lock.md:399`,
`:458`, `:464` now state that the reclaim is ownership-verified, and
`plans/closure/locking-convergence/001-status.md:236` records the
correction as done in M001 ("The stale claim in
`architecture/process_file_lock.md` and `architecture/overview.md` was
corrected in this milestone's owned documents").

The stale copy that actually survives is in a document M001 did not own —
`architecture/commands/restore_cmd.md:284`:

> "The transaction lock's stale-lock quarantine is **not ownership-verified**
> (`src/transaction.rs:1268`), unlike the local-data lock's, which re-reads
> the lock record and refuses to quarantine when it has changed or become
> unparseable (`src/local_data.rs:326-354`, rename at `:356-370`)."

That sentence is wrong on three counts. `src/transaction.rs:1268` is a
path-normalization helper (`is_within`), not locking code. `src/local_data.rs`
is 451 lines long and `:326-354` / `:356-370` are `#[cfg(test)]` unit tests,
not the reclaim implementation. And both locks now share
`src/record_lock.rs:695` (`quarantine_owned_lock`), which re-reads the record
at `:703` and compares it against the observed value at `:706-724`, refusing
to quarantine when the record changed or became unparseable. The code is
symmetric — `architecture/persistence.md:929` already says so, while `:867`
still singles out the local-data lock.

**The "release on `Drop` without unlinking" wording.** The M001 closure
record asks M003 to correct it: "The plan (§4, §14) and the roadmap state
that the owned-record locks 'release on `Drop` without unlinking'. That is
true of the kernel-backed tier only; both owned-record `Drop` impls have
always performed an ownership-verified removal via `remove_owned_lock_file`"
(`plans/closure/locking-convergence/001-status.md:234`; the roadmap preamble
repeats it at `plans/subsystems/locking-convergence-roadmap.md:12-16`).
The code confirms it: `src/record_lock.rs:432-447` implements
`Drop for OwnedLockGuard<R>` as an ownership-verified removal via
`remove_owned_lock_file` (`src/record_lock.rs:440-445`).

### 3.6 A false premise in the roadmap's evidence section

The roadmap states that "`src/library/persistence.rs` acquires the
transaction lock before the local-data lock, the reverse tier order"
(`plans/subsystems/locking-convergence-roadmap.md:163-164`).

**That is false.** `src/library/persistence.rs:266-274` calls
`gate_mutation_on_interrupted_transactions(&sync_state_dir, &transaction_dir)?`
and then `acquire_local_data_lock(&transaction_dir)?`; `:302-313` does the
same. Neither path calls `acquire_transaction_lock`. The transaction lock is
taken only inside the gate's recovery path
(`src/transaction.rs:664`), and that lock is released before the local-data
lock is acquired, so there is no inversion and no reverse tier order. The
roadmap's own conclusion — "the documented tiers do not describe the
acquisition order that the code performs" (`:167-169`) — is right, but the
`persistence.rs` example it cites is wrong and must not be carried into M003.

## 4. Invariants that must not regress

Drawn from `AGENTS.md` and `architecture/overview.md` §7 (`architecture/overview.md:476-496`).

1. **The kernel is the authority for the kernel-backed tiers.** A lock file's
   presence proves nothing; only the kernel lock state does
   (`src/process_file_lock.rs:11-15`).
2. **The kernel-backed family releases on `Drop` without unlinking.** Lock
   files may hold stale metadata (`architecture/overview.md:484-486`).
   **The owned-record family does not**: both owned-record `Drop` impls
   perform an ownership-verified removal via `remove_owned_lock_file`
   (`src/record_lock.rs:432-447`; `plans/closure/locking-convergence/001-status.md:234`).
   M003's documentation pass must not merge these two behaviors (§3.5).
3. **Identity metadata never authorizes stealing.** A busy kernel lock with
   malformed, empty, or legacy metadata is still a busy lock
   (`src/process_file_lock.rs:21-24`).
4. **The owned-record reclaim is ownership-verified for both locks**
   (`src/record_lock.rs:695`, `:703-724`). The stale "not ownership-verified"
   sentence in `architecture/commands/restore_cmd.md:284` is corrected, not
   propagated.
5. **`kill(pid, 0)`: only `ESRCH` proves absence.** `EPERM` and unknown
   errnos mean live (`src/utils/process.rs:29`). Linux start tokens use
   `/proc/<pid>/stat` field 22 (`src/process_file_lock.rs:500-504`).
6. **The mutation gate stays outside the tier order and stays fail-closed.**
   `gate_mutation_on_interrupted_transactions()` runs before every local
   mutation (`src/library/persistence.rs:266-269`); one journal means
   auto-rollback, multiple or incomplete journals mean refuse and direct to
   `snp repair` (`architecture/overview.md:446-455`).
7. **The owned-record protocol is untouched.** `src/record_lock.rs` owns the
   `create_new` + ownership-record protocol; the transaction lock keeps its
   fail-fast policy and the local-data lock its wait-for-progress policy
   (`src/record_lock.rs:458`, `src/transaction.rs:962`,
   `src/local_data.rs:131`).
8. **The lock hierarchy order in the code does not change.** The five
   nesting edges in §3.2 are the order the code performs; M003 states them,
   it does not rearrange them.
9. **The Windows locked range stays a single byte at `u32::MAX`**
   (`src/process_file_lock.rs:230`, `:236`), and the Windows MSVC
   `/STACK:8388608` link flag stays (`.cargo/config.toml`).
10. **No new syscall on the uncontended path**
    (`plans/subsystems/locking-convergence-roadmap.md:364-367`).
11. **The status snapshot stays read-only.** `snp status` takes no execution
    lock (`architecture/commands/status_cmd.md:125`, `:146`, `:210`).
12. **No credential or sync metadata in a lock payload**
    (`src/process_file_lock.rs:645-670`).

## 5. Scope

### In scope

- One merged ordering statement over `SyncExecutionLock`, `LocalDataLock`,
  `TransactionLock`, and `PendingTxnGuard`, written once and pointed at from
  the documents that currently hold fragments.
- The machine-readable tier table the static guard checks against.
- The `WorkerLock` tier removed from the code: `src/auto_sync/execution_lock.rs`,
  the `src/auto_sync/lock.rs` shim, and `src/auto_sync/mod.rs`.
- The `WorkerLock` tier removed from the status surface:
  `src/status_snapshot.rs`, `src/commands/doctor_cmd.rs`, and
  `src/commands/sync_cmd.rs`.
- The `status --json` schema version bumped, because removing
  `execution.worker_lock` is a breaking change
  (`docs/JSON_SCHEMAS.md:31`).
- A static guard in `tests/architecture.rs` that fails when a new acquisition
  site violates the order.
- The stale "not ownership-verified" sentence in
  `architecture/commands/restore_cmd.md:284` corrected, and the
  "release on `Drop` without unlinking" wording corrected for the
  owned-record family (§3.5).

### Explicitly out of scope

- **The owned-record lock protocol** (`src/record_lock.rs`,
  `src/transaction.rs`, `src/local_data.rs`). M001 owns it; it is closed.
- **The kernel-lock protocol** and the shared crate it introduces. M002 owns
  that (`plans/implementation/locking-convergence/002-kernel-lock-protocol-convergence.md`).
  If M002 lands first, M003's guard must be updated to also scan
  `snip-lock/src/`; if M003 lands first, M002 must add the new crate to the
  guard's scan set.
- **Changing any acquisition site's nesting.** §3.2 shows the code already
  matches the order; M003 states it and guards it.
- **The `status --json` field set beyond `execution.worker_lock`.** No other
  field changes.
- **The human-readable `snp status` output**, which never renders the
  execution summary (§3.3).
- **Any wire-protocol, CLI-flag, or exit-code change**
  (`plans/subsystems/locking-convergence-roadmap.md:335-339`).
- **Lock fencing, lock authority changes, and the auto-sync/sync authority
  relationship** — ADR triggers that stop the milestone
  (`plans/subsystems/locking-convergence-roadmap.md:401-403`).

## 6. Required production changes

### 6.1 The merged ordering statement

Write the total order once, in `architecture/process_file_lock.md`, which
already owns the "Locks are kernel-backed and ordered" topic and already
carries the partial order at `:481-482`. The statement must name, for each
tier, the tier's file, its family (kernel-backed or owned-record), the
function that acquires it, and the tier that may be held while it is
acquired:

> **Lock tier order (outermost to innermost):**
> `SyncExecutionLock` → `LocalDataLock` → `TransactionLock` → `PendingTxnGuard`.
>
> A tier may only be acquired while the tiers above it are held, and never
> the reverse. `SyncExecutionLock` and `PendingTxnGuard` are kernel-backed
> (`src/process_file_lock.rs`); `LocalDataLock` and `TransactionLock` are
> owned-record (`src/record_lock.rs`). The mutation gate
> (`gate_mutation_on_interrupted_transactions`) runs **before** any tier is
> taken and is not a tier.

The documents that currently hold fragments must point at it rather than
restate it:

- `architecture/process_file_lock.md:481-482` — replaced by the merged
  statement.
- `.skills/transactions-and-auto-sync.md:54` — replaced by a pointer, keeping
  the two-mechanism distinction at `:56` and `:64` intact.
- `architecture/auto_sync.md:406-425` — the three-path description is kept as
  the *evidence* for the merged statement, with an added pointer and the
  `LocalDataLock`/`TransactionLock` tiers added; `:403` (the `WorkerLock`
  row) is removed.
- `architecture/overview.md:476-496` — §7 gains the pointer.
- `architecture/commands/status_cmd.md:115` — the `execution.worker_lock`
  row disappears (work package D).

### 6.2 The machine-readable tier table

Add `src/lock_order.rs` with the tier table the guard checks against, and
register it in `src/lib.rs` next to the other doc-hidden modules
(`src/lib.rs:42` shows the pattern: `#[doc(hidden)] pub mod process_file_lock;`).

The module exposes:

- `pub enum LockTier { Execution, LocalData, Transaction, Pending }` with a
  `rank()` that returns `1..=4` in the declared order.
- `pub const LOCK_TIER_ORDER: [LockTier; 4]` — the single order.
- The set of recognized acquisition call patterns, one per tier, so the guard
  and any future caller agree on what counts as an acquisition site:
  `Execution` → `execution_lock::try_acquire` / `execution_lock::wait_acquire`;
  `LocalData` → `local_data::acquire_local_data_lock` /
  `library::manager::…::acquire_local_data_lock`;
  `Transaction` → `transaction::acquire_transaction_lock`;
  `Pending` → `pending_lock::acquire_pending_txn`.
- `pub const ACQUISITION_SITES: &[(&str, LockTier)]` — the allowlist from
  §3.4, one entry per direct call site (`file:line` → tier), so a new
  acquisition site must be registered before it is allowed.

The tier order and the allowlist live in one module so the guard, the
architecture document, and any code that needs the order read the same
source.

### 6.3 The `WorkerLock` tier leaves the code

Delete from `src/auto_sync/execution_lock.rs`: `WorkerLock` (`:246`),
`WorkerLockError` and its `From` impl (`:223-243`),
`worker_lock_path` (`:273`), `try_acquire_worker` (`:277`), `worker_inspect`
(`:283`), `worker_is_stale` (`:287`), `wait_acquire_worker` (`:291`), and the
`WORKER_LOCK_NAME` / `WORKER_LOCK_PURPOSE` constants.

Delete `src/auto_sync/lock.rs` (the re-export shim at `:3-7`) and its
`pub mod lock;` registration at `src/auto_sync/mod.rs:6`. Delete
`paths::worker_lock` at `src/auto_sync/mod.rs:43`.

**M002 overlap:** M002 collapses the `inspect`/`worker_inspect` and
`is_stale`/`worker_is_stale` pairs to one implementation each while keeping
every public name
(`plans/implementation/locking-convergence/002-kernel-lock-protocol-convergence.md`
§6.4). M003 then deletes the `worker_*` names. If M002 lands first, M003
deletes the delegating shims M002 left behind. If M003 lands first, M002's
work package E shrinks to the `process_alive` split and the error-conversion
collapse. The two milestones must not both delete the same symbols in the
same change.

### 6.4 The `WorkerLock` tier leaves the status surface

- `src/status_snapshot.rs`: remove the `worker_lock` field from
  `ExecutionSummary` (`:119`), remove `inspect_worker_lock` (`:321-341`),
  remove the three `sync.worker_lock.*` diagnostics (`:463-489`), and
  remove the `WORKER_LOCK_PURPOSE` use at `:938` and `:959`.
- `src/commands/doctor_cmd.rs:98-109`: remove the three remediations whose
  artifact is `worker_lock`.
- `src/commands/sync_cmd.rs:913-914`: remove the `worker_lock` branch of the
  permission-hardening path.
- `src/status_snapshot.rs:15`: bump `SNAPSHOT_SCHEMA` from `1` to `2`,
  because removing `execution.worker_lock` is a breaking change and
  `docs/JSON_SCHEMAS.md:31` requires the version to move. Update the unit
  test at `src/status_snapshot.rs:1151-1152`.
- `docs/JSON_SCHEMAS.md:24`, `:170`, `:193`: drop `worker_lock` from the
  documented `status --json` shape and record the schema bump.

The `execution_lock` field and `src/status_snapshot.rs:293-319` stay: the
execution lock is live and its liveness is still reported.

### 6.5 The static guard

Add to `tests/architecture.rs`, following the M001 pattern (the existing
three guards are at `:425`, `:438`, `:478`):

1. **`lock_tier_order_is_declared_once`** — asserts `src/lock_order.rs`
   exists, exposes `LOCK_TIER_ORDER`, and that its order matches the order
   named in `architecture/process_file_lock.md` (the guard extracts the
   statement's tier names and compares them to `LOCK_TIER_ORDER`). If M002
   has landed, the scan set also includes `snip-lock/src/`.
2. **`acquisition_sites_respect_the_declared_tier_order`** — for every
   `.rs` file under `src/`, extract each function body, find the acquisition
   call sites inside it, map them to tiers, and assert the tier ranks in
   source order are **non-decreasing**. This catches a function that takes a
   lower tier before a higher one.
3. **`every_acquisition_site_is_registered`** — asserts that every
   acquisition call site found in `src/` appears in
   `LOCK_ORDER_ACQUISITION_SITES`. A new acquisition site that is not
   registered fails the guard, and registering it forces the author to
   declare where it sits in the order.

Each guard needs a **synthetic-fixture test** proving the detector can fail,
following `owned_record_lock_protocol_detector_reports_a_second_copy`
(`tests/architecture.rs:438-473`). A guard that cannot fail proves nothing.

The guard must be careful to match **call sites, not definitions**:
`src/auto_sync/execution_lock.rs:143`/`:152` define the acquisition
functions, and `src/auto_sync/pending_lock.rs:110` and
`src/local_data.rs:131` define theirs. Those definition sites must not be
counted as acquisition sites, or the allowlist would list the implementations
rather than the callers.

## 7. Ordered work packages

### Work package A — Write the merged ordering statement

**Required changes**

1. Write the total order in `architecture/process_file_lock.md` per §6.1,
   replacing the partial statement at `:481-482`.
2. Replace `.skills/transactions-and-auto-sync.md:54` with a pointer to it,
   keeping the two-mechanism distinction at `:56` and `:64`.
3. Extend `architecture/auto_sync.md:406-425` with a pointer and the two
   missing tiers; delete the `WorkerLock` row at `:403`.
4. Add the pointer to `architecture/overview.md` §7 (`:476-496`).

**Acceptance evidence**

- `grep -rn "LocalDataLock → TransactionLock" architecture/ .skills/` reports
  no document that states it as *the* order; every hit is a pointer to
  `architecture/process_file_lock.md`.
- `architecture/process_file_lock.md` names all four tiers, their families,
  their acquiring functions, and the gate's position outside the order.
- `architecture/auto_sync.md` no longer contains a `WorkerLock` row.

### Work package B — Add the machine-readable tier table

**Required changes**

1. Create `src/lock_order.rs` per §6.2: `LockTier`, `LOCK_TIER_ORDER`, the
   acquisition call patterns, and `ACQUISITION_SITES` populated from §3.4.
2. Register it in `src/lib.rs` as `#[doc(hidden)] pub mod lock_order;`, next
   to `src/lib.rs:42`.
3. Add a unit test in `src/lock_order.rs` asserting the ranks are
   `1, 2, 3, 4` and that `LOCK_TIER_ORDER` contains each tier exactly once.

**Acceptance evidence**

- `cargo test --workspace --lib` is green, including the new
  `src/lock_order.rs` tests.
- `ACQUISITION_SITES` has one entry per direct call site listed in §3.4, and
  every entry's `file:line` still exists in the tree.

### Work package C — Retire the `WorkerLock` tier from the code

**Required changes**

1. Delete the `WorkerLock` tier from `src/auto_sync/execution_lock.rs` per
   §6.3.
2. Delete `src/auto_sync/lock.rs` and its `pub mod lock;` registration at
   `src/auto_sync/mod.rs:6`; delete `paths::worker_lock` at
   `src/auto_sync/mod.rs:43`.
3. Confirm no caller remains: `grep -rn "WorkerLock\|worker_inspect\|
   worker_is_stale\|worker_lock_path\|WORKER_LOCK" src/` reports nothing.

**Acceptance evidence**

- `cargo build --workspace` succeeds; `cargo test --workspace --lib` is green.
- `grep -rn "WorkerLock\|worker_inspect\|worker_is_stale\|worker_lock_path\|WORKER_LOCK" src/`
  reports no hits.
- `tests/auto_sync_detached_worker.rs:208` (`test_parent_does_not_acquire_worker_lock`)
  still passes: the parent never acquires the worker lock, and after the
  retirement it never creates the file either.
- `tests/auto_sync_concurrency.rs:793`
  (`test_symlink_substitution_on_worker_lock_resisted`) is **retargeted** to
  the execution lock, which is the tier that is actually acquired. The
  existing test asserts that a symlink substituted for
  `auto-sync-worker.lock` does not break acquisition (`:828-830`); the
  retargeted test asserts the same property for
  `auto-sync-execution.lock`.

### Work package D — Retire the `WorkerLock` tier from the status surface

**Required changes**

1. Apply §6.4: remove `ExecutionSummary.worker_lock`
   (`src/status_snapshot.rs:119`), `inspect_worker_lock` (`:321-341`), the
   three `sync.worker_lock.*` diagnostics (`:463-489`), and the
   `WORKER_LOCK_PURPOSE` uses at `:938` and `:959`.
2. Remove the three `worker_lock` remediations at
   `src/commands/doctor_cmd.rs:98-109` and the `worker_lock` branch at
   `src/commands/sync_cmd.rs:913-914`.
3. Bump `SNAPSHOT_SCHEMA` to `2` (`src/status_snapshot.rs:15`) and update
   the unit test at `src/status_snapshot.rs:1151-1152`.
4. Update `docs/JSON_SCHEMAS.md:24`, `:170`, `:193`.

**Acceptance evidence**

- `snp status --json` emits `"schema": 2` and contains no `worker_lock`
  key; `snp status --json` still contains `execution.execution_lock`.
- `cargo test --workspace --lib` is green; the schema unit test at
  `src/status_snapshot.rs:1151-1152` asserts `2`.
- `tests/recovery_integration.rs:471` still passes: `snp sync repair --dry-run`
  ignores lock artifacts, including the retired tier.
- `grep -rn "worker_lock" src/` reports no hits outside comments.

### Work package E — Add the static guard

**Required changes**

Apply §6.5: the three guards plus their synthetic-fixture tests in
`tests/architecture.rs`.

**Acceptance evidence**

- `cargo test --test architecture` is green.
- The synthetic-fixture tests demonstrate that a function which acquires a
  lower tier before a higher one, and an unregistered acquisition site, are
  each detected.
- The guard does not count the *definition* sites
  (`src/auto_sync/execution_lock.rs:143`, `:152`;
  `src/auto_sync/pending_lock.rs:110`; `src/local_data.rs:131`;
  `src/transaction.rs:962`) as acquisition sites.
- `tests/architecture.rs` still carries no module-level
  `#![allow(dead_code)]` (the existing guard at `tests/architecture.rs:247`).

### Work package F — Correct the stale documentation

**Required changes**

1. Correct `architecture/commands/restore_cmd.md:284` per §3.5: the
   transaction lock's quarantine **is** ownership-verified, it shares
   `src/record_lock.rs:695` with the local-data lock, and the reclaim
   re-reads and compares the record before renaming
   (`src/record_lock.rs:703-724`). Remove the stale `src/transaction.rs:1268`,
   `src/local_data.rs:326-354`, and `:356-370` references.
2. Make `architecture/persistence.md:867` agree with `:929`: both locks'
   reclaims are ownership-verified.
3. Correct the "release on `Drop` without unlinking" wording for the
   owned-record family wherever it appears — the roadmap's §4
   (`plans/subsystems/locking-convergence-roadmap.md:13-16` assigns this to
   M003), `architecture/process_file_lock.md`, `architecture/overview.md`,
   `.skills/transactions-and-auto-sync.md`, and
   `architecture/commands/restore_cmd.md`. The kernel-backed family does
   release without unlinking; the owned-record family performs an
   ownership-verified removal (`src/record_lock.rs:432-447`).
4. Correct the `file:line` references that drifted during M001:
   `architecture/process_file_lock.md:399` cites `src/record_lock.rs:692`
   (the function is at `:695`), and `architecture/persistence.md:345` cites
   `src/record_lock.rs:770-778` (`is_transient_lock_contention` is at
   `:773`).

**Acceptance evidence**

- `grep -rn "not ownership-verified" architecture/ .skills/` reports no hits.
- `grep -rn "without unlinking" architecture/ .skills/ plans/subsystems/locking-convergence-roadmap.md`
  reports only statements scoped to the **kernel-backed** family.
- Every `file:line` reference added or corrected points at a line that
  exists in the tree after the change.

### Work package G — Documentation updates and verification

**Required changes**

1. Update `architecture/diagnostics.md:249-250` — remove the
   `sync.worker_lock.*` codes.
2. Update `architecture/commands/status_cmd.md:115` — remove the
   `execution.worker_lock` row.
3. Update `architecture/auto_sync.md:400-404` — the "Locks: what each one
   guards" table loses the `WorkerLock` row.
4. Update `CHANGELOG.md` — the `status --json` schema bump and the removal
   of the retired tier.
5. Run §11 in full.

**Acceptance evidence**

- `grep -rn "worker_lock" architecture/ docs/ .skills/` reports no hits
  except historical `CHANGELOG.md` entries and any explicitly-marked
  archival note.
- Every command in §11 passes, and `bash scripts/check.sh` is green.

## 8. Failure, cancellation, restart, and contention semantics

### 8.1 The guard does not change runtime behavior

The ordering contract is a **documentation and static-analysis** artifact.
It changes no acquisition site, no wait policy, no timeout, and no error
path. A violation detected by the guard is a *build-time* failure, not a
runtime one.

### 8.2 Retiring the `WorkerLock` tier changes no runtime behavior

Because no production path acquires the tier, removing it changes only
observable output:

- `status --json` loses `execution.worker_lock` and moves to
  `"schema": 2`.
- `snp doctor` loses the three `sync.worker_lock.*` diagnostics.
- `snp sync repair --apply` no longer chmods `auto-sync-worker.lock` to
  `0o600` (`src/commands/sync_cmd.rs:913-914`). That file is never written
  by any current code path, so nothing is lost. If a lock file left over from
  an older build is present, it is simply ignored — exactly as
  `architecture/auto_sync.md:403` already documents.

### 8.3 Contention, cancellation, and restart are unchanged

Each tier keeps its own policy, and none of them changes:

| Tier | Family | Policy | Source |
|------|--------|--------|--------|
| `SyncExecutionLock` | kernel | worker: `try_acquire` (cheap exit); foreground: `wait_acquire`, 30 s default | `src/auto_sync/execution_lock.rs:143`, `:152`, `:24` |
| `PendingTxnGuard` | kernel | `wait_acquire`, 10 s timeout | `src/auto_sync/pending_lock.rs:110-119`, `src/auto_sync/pending.rs:17` |
| `LocalDataLock` | owned-record | wait-for-progress | `src/local_data.rs:131`, `src/record_lock.rs:458` |
| `TransactionLock` | owned-record | fail-fast | `src/transaction.rs:962`, `src/record_lock.rs:458` |

The kernel-backed tiers release on `Drop` without unlinking; the owned-record
tiers perform an ownership-verified removal
(`src/record_lock.rs:432-447`). Stale-lock reclaim stays
ownership-verified for both owned-record tiers
(`src/record_lock.rs:695`, `:703-724`).

### 8.4 The mutation gate keeps its fail-closed behavior

`gate_mutation_on_interrupted_transactions()` remains a precondition, not a
tier: it runs before any tier is acquired
(`src/library/persistence.rs:266-269`), and it is not named in
`LOCK_TIER_ORDER`. Its fail-closed rules — one journal means auto-rollback;
multiple or incomplete journals mean refuse and direct to `snp repair`;
corrupt journals and `RecoveryClass::UnsafeFailed` also refuse — are
unchanged (`architecture/overview.md:446-455`).

## 9. Compatibility and migration

- **The `status --json` schema changes.** `execution.worker_lock` is removed
  and `"schema"` moves from `1` to `2`, as
  `docs/JSON_SCHEMAS.md:31` requires for a breaking change. Consumers that
  read `execution.worker_lock` must stop; consumers that read
  `execution.execution_lock` are unaffected. The change is recorded in
  `CHANGELOG.md`.
- **The human-readable `snp status` output is unchanged** — it never rendered
  the execution summary (§3.3).
- **No lock file format changes.** No tier's on-disk record changes, so locks
  taken by an older build stay recognizable and a lock file left behind by an
  older build does not block a newer one.
- **No CLI flag, subcommand, or exit-code change**
  (`plans/subsystems/locking-convergence-roadmap.md:338`).
- **No wire-protocol change** (`plans/subsystems/locking-convergence-roadmap.md:337`).
- **A leftover `auto-sync-worker.lock` from an older build is ignored**, as it
  already is. Nothing migrates it and nothing deletes it.
- **The ordering statement is a documentation change** with no runtime
  effect.

## 10. Required tests

### 10.1 New tests in `src/lock_order.rs`

- The tier ranks are `1, 2, 3, 4` in the declared order.
- `LOCK_TIER_ORDER` contains each tier exactly once.
- Every entry in `ACQUISITION_SITES` names a file that exists and a line
  that still contains an acquisition call.

### 10.2 New guards in `tests/architecture.rs`

- `lock_tier_order_is_declared_once` — the code constant and the
  architecture document state the same order.
- `acquisition_sites_respect_the_declared_tier_order` — no function body
  acquires a lower tier before a higher one.
- `every_acquisition_site_is_registered` — no acquisition call site is
  outside the allowlist.

Each with a synthetic-fixture test proving the detector can fail (§6.5).

### 10.3 Updated tests

- `src/status_snapshot.rs:1151-1152` — the schema assertion moves to `2`.
- `tests/auto_sync_concurrency.rs:793`
  (`test_symlink_substitution_on_worker_lock_resisted`) — retargeted to the
  execution lock (§7, work package C).
- `tests/recovery_integration.rs:471` — unchanged; it asserts that
  `snp sync repair --dry-run` ignores lock artifacts, which remains true
  for the retired tier.

### 10.4 Existing suites that must stay green

- `tests/mutual_exclusion.rs` — including
  `test_stale_lock_with_dead_pid_is_recovered` (`:143`).
- `tests/process_lock_concurrency.rs` — real subprocess contenders. Serial.
- `tests/auto_sync_concurrency.rs` and `tests/auto_sync_detached_worker.rs` —
  the worker-path ordering is unchanged.
- `tests/local_data_lock_barriers.rs` and `tests/repair_transactions.rs` —
  the owned-record tiers. Serial.
- `tests/architecture.rs` — the existing three M001 guards plus the three
  new ones.
- `tests/snippet_lifecycle.rs` / `tests/doctor_contract.rs` (if present) —
  the `snp doctor` remediation set.

## 11. Required verification commands

Run these in order. **Never run a cargo command while another test run is in
flight** — the integration tests execute `target/debug/snp`, and a rebuild
replaces that binary mid-run and produces false failures (`AGENTS.md`,
"Gotchas that break builds or corrupt state").

```bash
# 0. Formatting and lint (no test is running)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

# 1. Lint the test-support-gated targets that scripts/check.sh does NOT lint
#    (check.sh does not pass --all-features; AGENTS.md).
cargo clippy --workspace --all-targets --all-features -- -D warnings

# 2. Unit tests (parallel-safe; isolated TempDir per test)
cargo test --workspace --lib

# 3. Static guards, including the three new ones
cargo test --test architecture

# 4. Serial lock and barrier suites
cargo test --test process_lock_concurrency --features test-support -- --test-threads=1
cargo test --test local_data_lock_barriers --features test-support -- --test-threads=1
cargo test --test repair_transactions --features test-support -- --test-threads=1
cargo test --test auto_sync_concurrency --features test-support -- --test-threads=1
cargo test --test mutual_exclusion --features test-support -- --test-threads=1

# 5. The authoritative Linux CI gate
bash scripts/check.sh
```

`scripts/check.sh` is the authoritative Linux CI gate (`AGENTS.md`); it
already runs `cargo fmt`, `cargo clippy --workspace --all-targets`,
`cargo test --workspace --lib`, the serial `auto_sync_concurrency` and
`sync_multibatch` targets, and the `snip_sync_lifetime` suite.
`cleanup_crash_failpoints` and `restore_crash_failpoints` are invoked by **no
script** and must be run by hand before merging a restore or cleanup change.
**This milestone changes neither the restore nor the cleanup path**, so they
are not required here — but run them if the change touches
`src/transaction.rs`, `src/local_data.rs`, or `src/record_lock.rs` at all.
The `status --json` schema bump should also be checked by hand:

```bash
snp status --json | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d["schema"], sorted(d["execution"]))'
```

## 12. Documentation updates

| Document | Change |
|----------|--------|
| `architecture/process_file_lock.md` | The merged total order (§6.1), replacing `:481-482`; the corrected `Drop` wording (§3.5); the `src/record_lock.rs:692` → `:695` reference at `:399`. |
| `architecture/overview.md` | §7 (`:476-496`) gains the pointer to the merged order; the owned-record `Drop` wording corrected. |
| `architecture/auto_sync.md` | The `WorkerLock` row at `:403` is removed; the "Acquisition order" section (`:406-425`) gains a pointer and the two missing tiers; the lock table (`:400-404`) is updated. |
| `architecture/persistence.md` | `:867` agrees with `:929` (both locks ownership-verified); the `:345` reference corrected to `src/record_lock.rs:773`. |
| `architecture/commands/restore_cmd.md` | `:284` corrected: the transaction lock's quarantine **is** ownership-verified. |
| `architecture/commands/status_cmd.md` | `:115` drops the `execution.worker_lock` row. |
| `architecture/diagnostics.md` | `:249-250` drop the `sync.worker_lock.*` codes. |
| `.skills/transactions-and-auto-sync.md` | `:54` points at the merged order; the owned-record `Drop` wording corrected; the two-mechanism distinction at `:56`/`:64` preserved. |
| `docs/JSON_SCHEMAS.md` | `:24`, `:170`, `:193` drop `worker_lock`; the schema version moves to `2` under the rule at `:31`. |
| `plans/subsystems/locking-convergence-roadmap.md` | §4's false `persistence.rs` example (`:163-164`) and the stale "renames unconditionally" pointer (`:169-172`) are corrected, and the "without unlinking" wording (`:13-16`) is corrected — the owner of the roadmap is the parent agent. |
| `CHANGELOG.md` | The `status --json` schema bump to `2` and the removal of the retired tier. |

## 13. Acceptance criteria

1. One document states the total order over the four tiers, and every other
   document that mentions an order points at it rather than restating it.
2. The order matches the code: `SyncExecutionLock` → `LocalDataLock` →
   `TransactionLock` → `PendingTxnGuard`.
3. `src/lock_order.rs` exposes the tier table, and `tests/architecture.rs`
   asserts the code constant and the architecture document agree.
4. `tests/architecture.rs` contains a guard that fails when a function body
   acquires a lower tier before a higher one, and a guard that fails when an
   acquisition site is not registered. Each has a synthetic-fixture test
   proving it can fail.
5. `grep -rn "WorkerLock\|worker_inspect\|worker_is_stale\|worker_lock_path\|WORKER_LOCK" src/`
   reports no hits: the tier is gone from the code.
6. `grep -rn "worker_lock" src/ architecture/ docs/ .skills/` reports no hits
   outside historical `CHANGELOG.md` entries.
7. `status --json` emits `"schema": 2`, contains `execution.execution_lock`,
   and contains no `worker_lock` key. The human-readable output is unchanged.
8. `architecture/commands/restore_cmd.md:284` states that the transaction
   lock's quarantine is ownership-verified, and the "release on `Drop`
   without unlinking" wording is scoped to the kernel-backed family only.
9. No acquisition site's nesting changed: the five edges in §3.2 are
   unchanged after the milestone.
10. `src/record_lock.rs`, `src/transaction.rs`, `src/local_data.rs`, and
    `src/process_file_lock.rs` are unchanged in behavior: the owned-record
    protocol and the kernel protocol are M001's and M002's.
11. Every command in §11 passes, and `bash scripts/check.sh` is green.

## 14. Stop conditions

Stop the milestone and escalate rather than proceeding if any of these hold.

1. **The order cannot be stated without changing an acquisition site's
   nesting.** §3.2 shows the code already matches one total order. If
   stating it turns out to require re-nesting a tier — for example, moving
   the local-data acquisition inside the transaction lock — the milestone
   stops: that is a behavior change to lock scope, not a documentation pass.
2. **The guard cannot distinguish a legitimate acquisition site from a
   violating one.** If the textual scan cannot separate call sites from
   definition sites, or produces false positives on the existing sites, the
   guard is useless and the milestone stops until the detection strategy is
   reworked. A guard that cannot fail proves nothing.
3. **The guard's allowlist cannot be kept honest.** If
   `ACQUISITION_SITES` would have to grow on every unrelated commit, the
   guard becomes noise and the milestone stops until the registration
   mechanism is reworked.
4. **The `status --json` schema bump is refused.** If the
   `execution.worker_lock` removal cannot ship with `"schema": 2`
   (`docs/JSON_SCHEMAS.md:31`) — because a documented consumer depends on
   the field — the milestone stops. Keeping the field is not an option: the
   tier must leave the status surface, and a field that reports a tier with
   no acquirer is itself the defect.
5. **The owned-record `Drop` wording cannot be corrected without a code
   change.** §3.5 records that the code already performs an ownership-verified
   removal (`src/record_lock.rs:432-447`). If a document turns out to
   describe a *different* behavior than the code, the milestone stops and
   reports the discrepancy rather than editing the document to match a bug.
6. **The mutation gate cannot stay outside the tier order.** If making the
   gate part of the order is unavoidable, the milestone stops: the gate's
   fail-closed behavior (`architecture/overview.md:446-455`) is an ADR-scale
   change, not an ordering statement.
7. **M002 and M003 would delete the same symbols.** If M002 has landed and
   has already deleted the `worker_*` names that §6.3 lists, work package C
   shrinks to nothing and no symbol is deleted twice. If the two milestones
   are landing in the same change, the milestone stops until they are
   sequenced.

## 15. Closure evidence required

The closure record is written at
`plans/closure/locking-convergence/003-status.md` using the template in
`plans/closure/README.md`. It must contain:

1. The verification command list from §11 with the actual output, including
   `bash scripts/check.sh`.
2. The `grep` output proving the `WorkerLock` tier is gone from `src/`,
   `architecture/`, `docs/`, and `.skills/`.
3. The `tests/architecture.rs` guard names and the synthetic-fixture test
   results, including the proof that the order guard fails on a synthetic
   inversion and that the allowlist guard fails on an unregistered site.
4. A `snp status --json` sample showing `"schema": 2`, the surviving
   `execution.execution_lock`, and the absent `worker_lock`.
5. The merged ordering statement, with the five nesting edges of §3.2 and
   their `file:line` evidence.
6. A statement that no acquisition site's nesting changed.
7. A statement that the owned-record protocol (`src/record_lock.rs`) and the
   kernel protocol (`src/process_file_lock.rs`) were not modified in
   behavior.
8. The list of every architecture document updated, with the `file:line`
   references that changed, including the two corrected stale statements.

## 16. Handoff notes

- **M002 and M003 do not overlap.** M002 collapses the duplicated
  `inspect`/`worker_inspect` and `is_stale`/`worker_is_stale` *bodies* while
  keeping every public name; M003 deletes the `worker_*` names and the tier.
  If M003 lands first, M002's work package E shrinks to the `process_alive`
  split and the error-conversion collapse. The parent should sequence them.
- **The roadmap's evidence section contains two false premises that M003
  must not inherit.** §3.5: the "renames unconditionally" description the
  roadmap names (`architecture/overview.md`,
  `architecture/process_file_lock.md`) was already corrected by M001; the
  surviving stale copy is in `architecture/commands/restore_cmd.md:284`.
  §3.6: the `persistence.rs` "reverse tier order" example is false —
  `src/library/persistence.rs` never acquires the transaction lock. The
  roadmap is owned by the parent agent; this plan states the corrections so
  they can be applied there.
- **The `WorkerLock` retirement is a breaking `status --json` change.** The
  plan bumps `SNAPSHOT_SCHEMA` to `2` and records it in `CHANGELOG.md`.
  That decision is the plan's; if the parent prefers to keep the field for a
  deprecation window, §14 stop condition 4 applies and the parent must
  choose.
- **The guard's allowlist is a maintenance obligation.** Every new
  acquisition site must be registered in `src/lock_order.rs`. That is the
  point — a new site that is not registered fails the build — but it is a
  recurring cost and should be mentioned in `AGENTS.md` if the parent wants
  it surfaced there.
- **Do not run cargo concurrently with another agent's test run.** The
  integration tests execute `target/debug/snp`; a concurrent rebuild replaces
  that binary mid-run and produces false failures.
