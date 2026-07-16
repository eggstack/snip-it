# Release 5E: Transactional Pending State and Lock Correctness

## Purpose

Correct the remaining cross-process races in the detached auto-sync worker without introducing a daemon, IPC service, second executable, or shell wrapper.

The current design uses a durable pending marker and a detached one-shot worker, but pending generation updates and conditional clears are not serialized. The worker lock also has unsafe stale-takeover and ownership-release behavior. These defects can lose wakeups, delete newer pending generations, or permit overlapping workers.

This phase establishes the filesystem transaction and ownership primitives that every later Release 5 corrective phase depends on.

## Required Outcomes

1. Every committed logical mutation increments the pending generation exactly once, including under concurrent CLI processes.
2. No concurrent writer can overwrite another writer's pending generation or snapshot.
3. Conditional clear is atomic with respect to concurrent mutation recording.
4. A stale worker cannot clear a newer pending generation.
5. A lock owner removes only the lock record it owns.
6. A live worker lock is never stolen solely because of elapsed wall-clock time.
7. Temporary files are unique per transaction and cannot be shared or truncated by competing processes.
8. Pending-state integrity covers every control field used for generation, debounce, and recovery decisions.
9. All state and lock artifacts retain restrictive permissions and bounded size.
10. The changes remain portable across Linux, macOS, and Windows.

## Current Defects to Remove

### Unlocked pending read-modify-write

`record_pending_mutation` currently reads generation N, computes N+1, then writes the marker without a transaction lock. Two processes can both publish N+1 or interfere through the shared temporary path.

### Non-atomic conditional clear

`clear_if_generation_matches` reads the marker and removes the path later. A newer generation can replace the file between those operations and then be deleted by the older worker.

### Shared temporary path

The current `auto-sync-pending.toml.tmp` path is reused by every writer. Concurrent opens, writes, fsyncs, and renames can interfere.

### Unsafe stale-lock takeover

The worker lock treats an old-but-live process as stale. The maximum lock age and maximum worker lifetime are both five minutes, allowing a live worker to be displaced.

### Unconditional lock removal

`WorkerLock::drop` removes the lock path without verifying that PID and nonce still match the current file. An older owner can delete a newer owner's lock after takeover.

## Architecture Decision

Introduce two narrowly scoped lock concepts:

- `PendingTxnLock`: protects short pending-marker transactions only.
- `SyncExecutionLock`: protects the full sync execution lifecycle and is addressed in Release 5F.

Do not reuse the long-lived worker execution lock for marker transactions. Parent mutation commands must remain fast and should hold `PendingTxnLock` only for the minimum read/modify/write or read/conditional-delete critical section.

## Workstream A: Pending Transaction Lock

Create `src/auto_sync/pending_lock.rs` or an equivalent focused module.

Requirements:

- acquire atomically with `create_new(true)`;
- include PID, nonce, creation timestamp, and schema version;
- use restrictive permissions;
- keep the critical section short;
- support bounded retry with jitter for brief contention;
- return a typed timeout/busy error rather than waiting indefinitely;
- reclaim only demonstrably dead owners;
- never classify a live owner as stale solely due to age;
- ownership-checked removal on drop.

Recommended API:

```rust
pub struct PendingTxnGuard { ... }

pub fn acquire_pending_txn(
    state_dir: &Path,
    timeout: Duration,
) -> Result<PendingTxnGuard, PendingTxnLockError>;
```

The lock file should be distinct from the sync execution lock, for example:

```text
auto-sync-pending.lock
```

## Workstream B: Transactional Generation Increment

Refactor `record_pending_mutation` so the entire operation occurs under `PendingTxnGuard`:

1. Acquire pending transaction lock.
2. Read and validate the current marker.
3. Calculate the next generation.
4. Build the complete next on-disk state.
5. Write to a unique temporary file in the same directory.
6. Flush file contents.
7. Atomically replace the marker.
8. Flush the parent directory where supported.
9. Release the transaction lock.

Use a unique temporary name containing PID and nonce or reuse the repository's hardened private atomic-write utility if it guarantees unique temporary files and same-directory replacement.

Never silently reset generation to 1 on malformed existing state. Return a typed corruption error and preserve the damaged file for diagnostics.

## Workstream C: Atomic Conditional Clear

Refactor `clear_if_generation_matches`:

1. Acquire `PendingTxnGuard`.
2. Read and validate the current marker while holding the guard.
3. Compare the current generation to the observed generation.
4. Remove or replace the marker while the guard is still held.
5. Release the guard.

The function must return a precise result:

```rust
pub enum ConditionalClearResult {
    Cleared,
    Missing,
    GenerationChanged { current: u64 },
}
```

Do not collapse generation mismatch into a generic boolean in internal code.

Manual sync, worker success, and any recovery cleanup must use this same primitive.

## Workstream D: Full-State Integrity

The current CRC covers only the snapshot. Replace it with integrity over the canonical serialized form of all behavior-driving fields:

- schema;
- generation;
- created/requested timestamp;
- snapshot;
- any future status or retry fields included in the marker.

Recommended process:

1. Serialize a structure excluding the integrity field.
2. Compute CRC32 over those bytes.
3. Store `integrity = "crc32:..."`.
4. Recompute over the same canonical structure during load.

CRC32 remains an accidental-corruption check, not an authentication mechanism. Document that distinction.

## Workstream E: Worker Lock Ownership Safety

Refactor `WorkerLock` or replace it with the shared execution lock introduced in Release 5F.

Immediate requirements:

- `Drop` must read the current lock record and remove it only when PID and nonce match the guard;
- an old guard must never remove a replacement owner's lock;
- malformed lock files must be handled conservatively;
- live PID means owned, regardless of age;
- dead PID may be reclaimed;
- PID reuse must be mitigated by nonce and, where practical, process start identity;
- Windows must not treat every PID as alive forever.

If reliable cross-platform PID liveness is not available, prefer conservative non-stealing plus a documented manual recovery command over unsafe takeover.

## Workstream F: Stale and Abandoned Artifact Recovery

Define explicit rules:

- dead pending transaction locks may be reclaimed;
- live pending transaction locks are never stolen;
- unique abandoned temporary files may be cleaned by age only after confirming they are not the current transaction's file;
- pending markers are never deleted merely because they are old;
- lock cleanup is ownership checked;
- doctor output reports malformed or abandoned artifacts without mutating them unless an explicit repair option is requested.

## Workstream G: API Cleanup

Remove or restrict legacy APIs that bypass transaction safety:

- make `mark_pending_internal` private;
- remove the public `mark_pending` alias from production-visible surfaces, or gate it to tests;
- ensure every generation writer calls `record_pending_mutation`;
- ensure every clear path calls the transactional conditional-clear primitive;
- add compile-time/module-boundary pressure so future callers cannot write the marker directly.

## Tests

### Unit tests

- unique temporary file generation;
- lock acquisition and ownership-checked drop;
- dead-owner reclaim;
- live-owner non-reclaim regardless of age;
- malformed lock handling;
- full-state integrity detects generation corruption;
- full-state integrity detects timestamp corruption;
- conditional-clear result variants;
- directory fsync helper behavior where supported.

### Multi-process tests

Use real `snp` subprocesses or a dedicated test helper binary.

1. Launch 20 concurrent mutation writers from the same initial generation.
2. Assert final generation increases by exactly 20.
3. Assert no writer reports a shared-temp rename failure.
4. Assert the final marker parses and passes integrity validation.
5. Race conditional clear of generation N against a writer publishing N+1; assert N+1 survives.
6. Repeatedly race record/clear for thousands of iterations under a bounded stress test.
7. Simulate old owner A, replacement owner B, then drop A; assert B's lock remains.
8. Hold a live lock beyond the stale-age threshold; assert another process cannot steal it.

### Security tests

- marker and locks contain no command body, output, tags, API key, token, or encryption material;
- symlink and directory substitution attempts fail closed;
- permissions are 0600 on Unix;
- temporary artifacts are in the state directory and not world-readable.

## Documentation

Update:

- `architecture/auto_sync.md`;
- `architecture/sync.md`;
- `docs/ARCHITECTURE_INVENTORY.md`;
- `AGENTS.md`.

Document the distinction between the short pending transaction lock and the long sync execution lock.

## Recommended Commit Sequence

1. Add pending transaction lock and ownership-safe lock release.
2. Convert generation increment to transactional unique-temp writes.
3. Convert conditional clear to the same transaction boundary.
4. Expand integrity coverage and migration tests.
5. Add deterministic concurrent subprocess tests.
6. Reconcile documentation and remove unsafe legacy APIs.

## Exit Criteria

This phase is complete only when:

- exact concurrent generation increments are proven;
- a newer generation cannot be deleted by an older clear;
- no shared temporary path remains;
- live locks cannot be stolen by age;
- old owners cannot delete replacement locks;
- all marker control fields are integrity checked;
- formatting and Clippy are clean;
- Linux, macOS, and Windows CI pass the relevant portable tests.
