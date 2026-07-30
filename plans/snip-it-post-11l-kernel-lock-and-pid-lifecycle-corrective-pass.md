# Post-Phase-11L — Kernel Lock, PID Lifecycle, and Editor Mutation Corrective Pass

Status: READY FOR IMPLEMENTATION

Corrective baseline: `48a20a7f701bf49924aa77c39ff4a5af6b40f7ba`

Phase 11L remains complete. This is a separate post-closure corrective pass for regressions introduced by the later `bugs.md` audit implementation.

---

## 1. Purpose

The `48a20a7` audit commit attempted to harden auto-sync locks, stale-lock recovery, `snip-sync` PID ownership, select output writes, editor handling, and rate-limiter persistence.

Several parts are useful and should be preserved, especially:

- atomic select output installation;
- exact selector output-file preservation on cancellation;
- nonzero editor status being visible to the caller;
- process start-token support;
- rate-limiter batch persistence;
- the test-target `required-features = ["test-support"]` correction.

However, the new lock and PID lifecycle code contains correctness defects:

1. stale-lock reclamation still has an inspect-then-rename race that can rename a newly acquired live lock;
2. lock and PID “removal” renames files to unique quarantine names but never deletes those quarantine files;
3. a process crash after `create_new` and before record publication can leave an empty canonical lock that permanently wedges future acquisition;
4. legacy numeric `snip-sync.pid` files are no longer readable, breaking upgrades and stop/start behavior;
5. Windows process liveness is hard-coded false in `snip-sync`, so a live server record can be treated as stale;
6. `snp edit` decides whether to notify auto-sync from editor exit status rather than actual file mutation.

This plan replaces the fragile stale-reclaim design with a simpler kernel-owned locking model. PID and start-token records remain useful for diagnostics and process signaling, but they must not authorize lock stealing or deletion.

---

## 2. Scope and non-goals

### In scope

- one shared cross-platform kernel-backed file-lock primitive;
- migration of worker, execution, and pending locks to that primitive;
- crash-safe owner metadata publication after lock acquisition;
- elimination of stale-lock quarantine and canonical-lock deletion races;
- bounded wait and nonblocking acquisition semantics;
- `snip-sync` singleton server lock;
- backward-compatible PID record parsing;
- atomic PID record publication and safe cleanup;
- Windows process liveness and start-token verification;
- exact file-change detection for `snp edit`;
- deterministic subprocess-level lock and crash tests;
- removal of leaked quarantine artifacts;
- focused documentation updates;
- full local verification and existing three-instance CI.

### Out of scope

- reopening Phase 11L;
- changing the transaction journal protocol;
- changing sync wire protocols;
- adding a daemon or task queue;
- adding a database for lock ownership;
- distributed locks;
- network leases;
- automated crates.io publishing;
- additional CI matrices;
- a new evidence registry;
- refactoring unrelated commands;
- reworking the select output implementation unless a focused regression is found.

---

## 3. Required architectural decision

### 3.1 Kernel lock is authoritative

Mutual exclusion must be owned by the operating system, not by PID-file inspection.

Use one shared abstraction that opens a persistent lock file and acquires an exclusive advisory lock:

- Unix: `flock(fd, LOCK_EX | LOCK_NB)` and `flock(fd, LOCK_UN)`;
- Windows: `LockFileEx` with `LOCKFILE_EXCLUSIVE_LOCK` and `LOCKFILE_FAIL_IMMEDIATELY`, then `UnlockFileEx`;
- unsupported platforms: return a clear unsupported-platform error rather than silently weakening exclusion.

The file may remain on disk permanently. The kernel lock, not file presence, indicates ownership.

### 3.2 PID metadata is diagnostic only

PID, nonce, start token, and acquisition time may be written to the lock file after the kernel lock is acquired.

They may be used for:

- error messages;
- status output;
- process signaling decisions;
- debugging.

They must not be used to:

- decide whether a lock can be stolen;
- unlink a canonical lock file;
- rename another process's lock aside;
- grant simultaneous ownership.

### 3.3 No quarantine lifecycle for normal locks

After this pass:

- normal lock release unlocks the OS lock and closes the file;
- canonical lock files remain in place;
- no `.quarantine.*` file is created on acquire, release, timeout, malformed metadata, or crash recovery;
- stale metadata is overwritten only after the new process has acquired the kernel lock.

This is intentionally simpler than compare-and-rename ownership protocols.

---

## 4. Small-model execution rules

1. Complete workstreams in order.
2. Do not combine server PID changes with auto-sync lock migration in one commit.
3. Keep existing public wrapper types where practical so callers do not require broad edits.
4. Add a focused failing test before each behavioral correction.
5. Do not weaken exact assertions to make platform tests pass.
6. Do not retain a fallback stale-reclaim path beside the kernel-lock path.
7. Do not use PID liveness as a substitute for kernel exclusion.
8. Do not delete or rename a canonical lock file during normal ownership transitions.
9. Do not use timing-only sleeps as the sole concurrency proof; use pipes, barriers, or explicit child-process signaling.
10. Keep Phase 11 closure status unchanged.
11. Keep manual release and the current lightweight CI topology unchanged.
12. Record the exact final implementation SHA only after source review and all verification gates pass.

---

# Workstream A — Add a shared kernel-backed process file lock

## Goal

Create one reusable primitive that provides authoritative cross-process mutual exclusion and crash recovery without stale-file deletion.

## Recommended location

Use one shared module, for example:

```text
src/process_file_lock.rs
```

If `snip-sync` cannot depend on the root crate module cleanly, place the implementation in the lowest existing shared crate or add a small equivalent module in `snip-sync` only after the root implementation is complete. Do not copy three separate versions into the auto-sync modules.

## Required API shape

Names may differ, but behavior must match:

```rust
pub struct ProcessFileLock {
    file: std::fs::File,
    path: PathBuf,
    identity: LockIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockIdentity {
    pub schema_version: u32,
    pub purpose: String,
    pub pid: u32,
    pub start_token: Option<String>,
    pub nonce: String,
    pub acquired_at_unix_ms: u64,
}

pub enum ProcessFileLockError {
    Busy { owner: Option<LockIdentity> },
    Timeout { owner: Option<LockIdentity> },
    Io(std::io::Error),
    UnsupportedPlatform,
}

pub fn try_acquire(path: &Path, purpose: &str)
    -> Result<ProcessFileLock, ProcessFileLockError>;

pub fn wait_acquire(path: &Path, purpose: &str, timeout: Duration)
    -> Result<ProcessFileLock, ProcessFileLockError>;
```

## Acquisition order

The exact sequence must be:

1. create the parent directory;
2. open the persistent lock file with read/write/create;
3. attempt the kernel lock;
4. if busy, read owner metadata best-effort for diagnostics and return `Busy`;
5. after the kernel lock succeeds, truncate the file;
6. write the new identity record;
7. flush and `sync_all` or `sync_data`;
8. set private permissions where supported;
9. return the guard.

If metadata publication fails after the kernel lock succeeds:

- release the kernel lock by dropping/unlocking the guard;
- return `Err`;
- leave no process believing it owns the lock;
- the next process must still be able to acquire and overwrite the empty or partial file.

## Drop behavior

`Drop` must:

- release the kernel lock;
- close the file naturally;
- not unlink the canonical lock path;
- not rename it;
- not create a quarantine file.

## Busy metadata behavior

Malformed or empty owner metadata must not be treated as stale ownership.

When the kernel lock is busy:

- return `Busy` even if metadata is empty or malformed;
- set `owner=None` when metadata cannot be parsed;
- never reclaim based on metadata.

When the kernel lock is free:

- acquisition succeeds regardless of old, empty, malformed, or legacy contents;
- new metadata replaces old contents after the lock is held.

## Unix implementation

Use the already available `libc` dependency.

Required behavior:

```rust
let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
```

Map `EWOULDBLOCK`/`EAGAIN` to `Busy`. Propagate other errors.

Unlock explicitly or rely on file close after best-effort explicit unlock:

```rust
unsafe { libc::flock(fd, libc::LOCK_UN) };
```

## Windows implementation

Use the existing `windows-sys` dependency.

Required behavior:

- obtain the file handle through `AsRawHandle`;
- call `LockFileEx` over a fixed byte range;
- use exclusive and fail-immediately flags for `try_acquire`;
- map lock violation/sharing violation to `Busy`;
- call `UnlockFileEx` in `Drop`;
- keep the file handle open for the guard lifetime.

Do not use `create_new` as the ownership primitive on Windows.

## Required unit tests

1. first acquisition succeeds;
2. second acquisition in the same process returns `Busy`;
3. dropping the first guard allows a later acquisition;
4. canonical file remains after release;
5. repeated 100 acquire/drop cycles create no additional files;
6. old malformed contents are overwritten after a successful acquisition;
7. empty contents do not block acquisition when no process holds the kernel lock;
8. metadata write failure releases ownership;
9. wait acquisition times out at the configured deadline;
10. wait acquisition succeeds after the first owner releases;
11. owner metadata contains no snippet content, API key, password, or command data;
12. permissions are private on Unix.

## Acceptance criteria

- one shared authoritative lock implementation exists;
- kernel ownership is the only mutual-exclusion authority;
- no stale PID classification is required to acquire;
- crash-released kernel locks are immediately reusable;
- malformed or empty disk contents cannot permanently wedge acquisition;
- canonical files are persistent and harmless;
- no quarantine files are created.

---

# Workstream B — Migrate worker, execution, and pending locks

## Goal

Replace all custom create/read/classify/quarantine loops with thin wrappers over the shared lock primitive.

## Files

Primary files:

- `src/auto_sync/lock.rs`;
- `src/auto_sync/execution_lock.rs`;
- `src/auto_sync/pending_lock.rs`.

## Wrapper requirements

Preserve caller-facing types where practical:

```rust
pub struct WorkerLock {
    inner: ProcessFileLock,
}

pub struct SyncExecutionLock {
    inner: ProcessFileLock,
}

pub struct PendingTxnGuard {
    inner: ProcessFileLock,
}
```

Keep existing path helpers and timeout semantics.

### Worker lock

- `try_acquire` remains nonblocking;
- busy maps to existing `AlreadyHeld` or a revised explicit `Busy` variant;
- owner diagnostics may include PID/nonce when parseable;
- no age-based or PID-based reclamation remains.

### Execution lock

- `try_acquire` remains nonblocking;
- `wait_acquire` polls the kernel lock until timeout;
- timeout reports best-effort owner metadata;
- no canonical file deletion occurs.

### Pending transaction lock

- preserve the short bounded wait behavior;
- use the same kernel primitive;
- remove corrupted-record and stale-owner reclamation branches;
- an unreadable record is diagnostic only.

## Code that must be removed

Remove or make unreachable:

- `OwnerClass` stale/PID-reuse classification for lock acquisition;
- `quarantine_lock`;
- `remove_owned_lock` rename-aside behavior;
- canonical lock `create_new` publication;
- empty-file retry budgets used as ownership recovery;
- lock-file deletion in `Drop`;
- tests that assert the lock file disappears after release.

`ProcessIdentity` may remain for diagnostics and PID records, but lock correctness must not depend on it.

## Required deterministic tests

### Exact simultaneous acquisition

Use a subprocess-level test helper gated by `test-support`.

Acceptable implementation:

```text
tests/bin/process_lock_helper.rs
```

with a Cargo target that requires `test-support`, or a self-spawning integration-test helper.

The helper must:

- wait on a barrier;
- attempt one named lock;
- print `ACQUIRED` only after ownership is held;
- hold until stdin closes or a release signal is received;
- print `BUSY` on contention.

Test:

1. create one lock path;
2. spawn at least eight contenders;
3. release all from one barrier;
4. assert exactly one prints `ACQUIRED`;
5. assert every other contender reports `BUSY` or times out without ownership;
6. assert a second owner can acquire only after the first releases.

### Crash recovery

1. child acquires lock;
2. child signals `ACQUIRED`;
3. parent kills child without graceful drop;
4. a new child acquires the same lock;
5. no canonical deletion or quarantine cleanup is required.

### Publication interruption

Add a test-support-only failure hook after kernel acquisition but before metadata publication.

1. child acquires kernel lock;
2. failpoint exits before or during record write;
3. next child acquires successfully;
4. next child publishes valid metadata;
5. no empty-file wedge remains.

### Artifact boundedness

After 500 acquisition/release cycles across all three wrappers:

- only the three canonical lock files may remain;
- zero filenames contain `.quarantine.`;
- directory entry count remains bounded.

## Acceptance criteria

- all three locks use the shared kernel primitive;
- no inspect-then-rename race remains;
- exactly one process owns each lock at a time;
- killed owners release through kernel process teardown;
- empty and malformed records cannot wedge future owners;
- no quarantine artifacts accumulate;
- existing caller behavior and timeout policies remain stable.

---

# Workstream C — Add an authoritative `snip-sync` server singleton lock

## Goal

Make server singleton ownership independent of PID-file deletion and PID reuse.

## Required server lock

Add a persistent server lock path, for example:

```text
<runtime-dir>/snip-sync.server.lock
```

Use the same kernel-lock abstraction or a small adapter around it.

`serve()` must acquire the server lock before interpreting or writing the PID file and hold it for the complete server lifetime.

Required sequence:

1. ensure layout;
2. acquire server singleton lock nonblocking;
3. if busy, report already running using best-effort owner metadata;
4. while holding the lock, read the PID record;
5. migrate or replace stale/legacy metadata as specified below;
6. atomically publish the current structured PID record;
7. start listeners and runtime;
8. hold the server lock until shutdown;
9. on graceful shutdown, remove the PID record only while still holding the server lock.

A process crash automatically releases the kernel lock. A stale PID record may remain, but it cannot block the next startup.

## Acceptance criteria

- two concurrent `serve` processes cannot both pass singleton acquisition;
- a crash leaves at most stale metadata, never a held lock;
- a new server can start after a crashed owner exits;
- PID metadata is no longer the singleton authority.

---

# Workstream D — Make PID records backward-compatible and atomically published

## Goal

Support existing numeric PID files, structured records, partial files, and platform identity without unsafe deletion.

## Parser design

Use a typed parser:

```rust
pub enum ParsedPidFile {
    Structured(PidRecord),
    LegacyPid(u32),
    Empty,
    Malformed(String),
}
```

Required parsing order:

1. trim contents;
2. empty → `Empty`;
3. all-decimal numeric value → `LegacyPid`;
4. structured TOML → `Structured`;
5. otherwise → `Malformed`.

Do not silently convert present malformed data into `None`.

## Startup behavior while server lock is held

- structured record for the current owner: replace atomically;
- stale structured record: replace atomically;
- legacy PID that is dead: replace atomically;
- legacy PID that is alive and process name matches `snip-sync`: refuse startup, because it may be an older live server that does not hold the new lock;
- legacy PID alive but process name cannot be verified: refuse by default with a diagnostic rather than risking two servers;
- empty or malformed record while server lock is free: log a warning and replace atomically.

## Atomic PID publication

Do not write the canonical PID file in place.

Required sequence:

1. serialize record completely in memory;
2. create a private unique temp file in the same directory;
3. write all bytes;
4. `sync_all`;
5. atomically replace the canonical PID file;
6. fsync the parent directory where supported;
7. remove the temp file on failure.

A crash before rename leaves only a temp file. A crash after rename leaves a complete structured record.

## PID cleanup

Do not rename the PID file to a permanent quarantine name.

Graceful server shutdown:

- server still holds singleton lock;
- re-read canonical record;
- remove it only if PID/start-token/nonce match the current server record;
- propagate or log cleanup errors accurately;
- fsync parent after deletion where supported.

`stop` cleanup:

1. read and retain the exact expected record;
2. signal the process;
3. wait for exit;
4. acquire the server singleton lock;
5. re-read the canonical PID record;
6. remove only if it still identifies the stopped process;
7. if lock acquisition is busy, a new server has started; do not remove its PID record.

## Legacy stop behavior

For `LegacyPid(pid)`:

- verify liveness;
- verify process name unless `--force` was supplied;
- signal as currently supported;
- after exit, acquire server lock and remove the numeric record only if it is still the same PID text.

## Windows liveness

Replace `#[cfg(not(unix))] is_running => false`.

On Windows use:

- `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, ...)`;
- `GetExitCodeProcess`;
- `STILL_ACTIVE`;
- `GetProcessTimes` for the start token;
- `CloseHandle` on every path.

`record_still_matches` must:

- return false when process is positively dead;
- return false when both start tokens are available and differ;
- return true when process is alive and tokens match;
- use a documented conservative policy when the platform cannot retrieve a token.

## Required PID tests

1. structured round-trip;
2. legacy numeric parsing;
3. empty parsing;
4. malformed parsing;
5. atomic write never exposes partial canonical TOML;
6. existing live legacy record prevents startup;
7. dead legacy record is replaced;
8. crash after temp write but before rename leaves canonical record unchanged;
9. graceful cleanup removes matching record;
10. cleanup preserves a replacement record;
11. repeated start/stop leaves zero `.quarantine.*` PID files;
12. Windows liveness compiles and is exercised in Windows smoke tests;
13. PID reuse mismatch is rejected when start token differs;
14. `stop` cannot delete a new server's replacement record.

## Acceptance criteria

- upgrades from numeric PID files remain operable;
- empty and malformed records no longer masquerade as absence;
- server singleton is kernel-owned;
- PID publication is atomic;
- PID cleanup cannot remove a replacement server's record;
- Windows no longer classifies every process as dead;
- no PID quarantine files accumulate.

---

# Workstream E — Base `snp edit` sync notification on actual file change

## Goal

Ensure auto-sync notification reflects whether the library changed, independent of editor exit status.

## Required behavior

After resolving/creating the target file, snapshot the exact pre-editor bytes:

```rust
let before = fs::read(&path)?;
let status = Command::new(...).status()?;
let after = fs::read(&path)?;
let changed = before != after;
```

Use exact bytes, not modification time alone.

Required outcome matrix:

| Editor status | File changed | Command result | Auto-sync notification |
|---|---:|---|---|
| success | no | success | none |
| success | yes | success | exactly one mutation notification |
| failure | no | error | none |
| failure | yes | error describing saved changes plus editor failure | exactly one mutation notification |

The nonzero error message must not claim the library was unmodified when bytes changed.

Do not trigger notification merely because the editor opened.

## Required tests

Use small test editor scripts or helper executables:

1. exits 0 without writing;
2. writes then exits 0;
3. exits nonzero without writing;
4. writes then exits nonzero;
5. truncates file then exits nonzero;
6. exact one-notification assertion for changed cases;
7. zero-notification assertion for unchanged cases;
8. error text distinguishes changed/nonchanged failure.

Do not make tests depend on a user's real `$EDITOR`.

## Acceptance criteria

- actual bytes determine mutation notification;
- nonzero status remains visible;
- saved changes are not silently left unsynchronized;
- unchanged editor sessions do not create pending sync intent.

---

# Workstream F — Remove obsolete code and update documentation

## Required cleanup

After migration, search the repository for:

```text
quarantine_lock
remove_owned_lock
.quarantine.
OwnerClass
empty beyond retry budget
create_new(true)
read_pid().is_some() || read_pid().is_none()
```

Expected results:

- no quarantine helper remains in auto-sync locks or PID cleanup;
- no canonical lock ownership uses `create_new`;
- `create_new` remains allowed for unique temporary files and unrelated exclusive-creation contracts;
- no tautological PID test remains;
- no documentation claims rename-aside itself proves replacement safety.

## Documentation updates

Update only relevant sections in:

- `AGENTS.md`;
- `architecture/persistence.md` or the existing lock/lifecycle architecture document;
- the new plan status section.

Document:

- persistent lock files are expected;
- OS lock state, not file existence, indicates ownership;
- owner metadata may be stale when no lock is held;
- PID records support legacy numeric parsing;
- `snp edit` notification follows actual byte changes.

Do not reopen or rewrite Phase 11L closure history.

---

# Workstream G — Verification and closure

## Focused tests

Run exact focused targets for the final implementation commit:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

cargo test --lib process_file_lock --all-features -- --test-threads=1
cargo test --lib auto_sync::lock --all-features -- --test-threads=1
cargo test --lib auto_sync::execution_lock --all-features -- --test-threads=1
cargo test --lib auto_sync::pending_lock --all-features -- --test-threads=1

cargo test --test process_lock_concurrency --features test-support -- --test-threads=1
cargo test --test process_lifecycle --features test-support -- --test-threads=1
cargo test --test pty_integration --features test-support -- --test-threads=1

cargo test -p snip-sync process --all-features -- --test-threads=1
```

Use actual final test names if different. Preserve one exact target for:

- eight-contender mutual exclusion;
- killed-owner recovery;
- interrupted metadata publication;
- PID legacy migration;
- replacement PID preservation;
- editor changed/nonchanged status matrix.

## Existing regression suites

```bash
cargo test --test mutual_exclusion --features test-support -- --test-threads=1
cargo test --test deterministic_e2e --features test-support -- --test-threads=1
cargo test --test repair_transactions --features test-support -- --test-threads=1
cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1
```

## Normal local verification

```bash
bash scripts/check.sh
bash scripts/release-check.sh verify
```

## Publish dry-runs

Run for changed crates in dependency order:

```bash
bash scripts/release-check.sh dry-run snip-proto
bash scripts/release-check.sh dry-run snip-sync
bash scripts/release-check.sh dry-run snip-it
```

`snip-proto` may be omitted only if it is unchanged and the release script explicitly supports changed-crate selection. Do not publish automatically.

## CI

The exact final implementation SHA must pass the existing instances only:

- Linux correctness;
- macOS platform smoke;
- Windows platform smoke.

Do not add another matrix or release job.

## Filesystem artifact check

Run a focused lifecycle loop and verify:

```bash
find <test-state-dir> -type f -name '*quarantine*' -print
```

Expected output: empty.

After repeated lock cycles, only canonical persistent lock files and active PID metadata may remain.

---

## 5. Required implementation sequence

### Commit 1 — Shared kernel process lock

Files:

- shared lock module;
- platform-specific implementation;
- unit tests.

Closure for this commit:

- first/second acquisition behavior exact;
- drop permits reacquisition;
- malformed and empty metadata do not wedge;
- no quarantine files.

### Commit 2 — Auto-sync lock migration

Files:

- worker lock;
- execution lock;
- pending lock;
- wrapper tests.

Closure for this commit:

- all wrappers use shared primitive;
- old reclaim/delete logic removed;
- timeout semantics retained.

### Commit 3 — Process-level concurrency and crash proof

Files:

- gated helper binary or self-spawning integration helper;
- deterministic concurrency integration tests.

Closure for this commit:

- exactly one of eight contenders acquires;
- killed owner releases automatically;
- interrupted metadata publication cannot wedge;
- repeated cycles leave zero quarantine artifacts.

### Commit 4 — `snip-sync` singleton and PID compatibility

Files:

- `snip-sync/src/process.rs`;
- `snip-sync/src/main.rs`;
- path helpers;
- PID/server tests.

Closure for this commit:

- server lock held for runtime lifetime;
- legacy numeric PID supported;
- atomic PID publication;
- replacement record protected;
- Windows liveness implemented.

### Commit 5 — Editor actual-change detection

Files:

- `src/commands/edit_cmd.rs`;
- focused tests/helper scripts.

Closure for this commit:

- four outcome matrix rows proven exactly;
- changed/nonzero case notifies and errors;
- unchanged cases do not notify.

### Commit 6 — Cleanup and documentation

Files:

- AGENTS/architecture docs;
- obsolete code removal;
- test target declarations.

Closure for this commit:

- no obsolete quarantine/reclaim code;
- no tautological tests;
- documentation matches persistent kernel lock model.

### Commit 7 — Verification and plan closure

Run all Workstream G commands on one exact implementation SHA.

Update this plan header only after verification:

```text
Status: COMPLETE
Final implementation commit: <full SHA>
Linux correctness: passed for <same SHA>
macOS smoke: passed for <same SHA>
Windows smoke: passed for <same SHA>
Local release verification: passed
Publish dry-runs: passed
```

If any result is unavailable or failing, keep `Status: READY FOR IMPLEMENTATION` or change to `IN PROGRESS`; do not claim completion.

---

## 6. Explicit anti-patterns

The implementation is not complete if any of these remain in the lock/PID ownership path:

```rust
if process_is_dead(record.pid) {
    fs::rename(canonical_lock, quarantine)?;
}
```

```rust
let snapshot = read_lock(path)?;
// another process can replace path here
fs::rename(path, quarantine)?;
```

```rust
OpenOptions::new().create_new(true).open(canonical_lock)
```

```rust
fn remove_owned_lock(path: &Path) {
    fs::rename(path, unique_quarantine_name())
}
```

```rust
if content.trim().is_empty() {
    retry_for_two_seconds_then_fail_forever();
}
```

```rust
#[cfg(not(unix))]
fn is_running(_pid: u32) -> bool { false }
```

```rust
assert!(read_pid().is_some() || read_pid().is_none());
```

```rust
if !editor_status.success() {
    return Err("library was not modified");
}
notify_mutation();
```

Required replacements are kernel ownership, typed PID parsing, atomic publication, exact subprocess tests, and byte-based editor mutation detection.

---

## 7. Final binary closure criteria

This corrective pass is complete only when every statement is true:

1. one shared kernel-backed file-lock primitive exists;
2. Unix uses authoritative `flock` ownership;
3. Windows uses authoritative `LockFileEx` ownership;
4. unsupported platforms fail explicitly rather than weaken exclusion;
5. worker lock uses the shared primitive;
6. execution lock uses the shared primitive;
7. pending transaction lock uses the shared primitive;
8. server singleton uses the shared primitive or an exact adapter;
9. PID liveness is not used to grant a lock;
10. no auto-sync lock uses inspect-then-rename stale reclamation;
11. no lock `Drop` deletes or renames the canonical lock file;
12. no normal lifecycle creates `.quarantine.*` files;
13. empty canonical metadata cannot wedge acquisition;
14. malformed canonical metadata cannot wedge acquisition;
15. a killed owner releases ownership through kernel teardown;
16. exactly one of at least eight simultaneous contenders acquires;
17. a replacement owner cannot be renamed or deleted by an earlier contender;
18. wait acquisition times out deterministically;
19. wait acquisition succeeds after release;
20. repeated lock cycles leave a bounded directory entry count;
21. `snip-sync` holds a singleton lock for the full runtime;
22. numeric legacy PID files parse and are handled safely;
23. structured PID records parse and round-trip;
24. empty and malformed PID records are explicit states, not silent absence;
25. PID records are atomically published through temp-file replacement;
26. graceful shutdown removes only the matching PID record;
27. `stop` cannot delete a replacement server's PID record;
28. Windows process liveness is implemented;
29. start-token mismatch rejects PID reuse;
30. repeated server lifecycle tests leave zero PID quarantine files;
31. `snp edit` compares exact bytes before and after the editor;
32. changed files trigger exactly one mutation notification;
33. unchanged files trigger zero mutation notifications;
34. changed plus nonzero editor exit both notifies and returns an accurate error;
35. unchanged plus nonzero editor exit returns an error without notification;
36. Phase 11L code and closure history remain unchanged;
37. transaction and restore regression tests still pass;
38. `scripts/check.sh` passes on the final implementation SHA;
39. `scripts/release-check.sh verify` passes on the same SHA;
40. changed-crate publish dry-runs pass;
41. Linux correctness passes for the exact SHA;
42. macOS smoke passes for the exact SHA;
43. Windows smoke passes for the exact SHA;
44. CI topology remains lightweight;
45. crates.io publishing remains manual;
46. no automated release workflow is added;
47. no daemon, database, queue, distributed lock, or evidence registry is added;
48. this plan records the exact verified implementation SHA before status-only follow-up commits.

Until every criterion is satisfied, the repository should not be described as release-ready after the `48a20a7` audit pass.
