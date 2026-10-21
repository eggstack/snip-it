# Process File Lock

[← Back to Overview](overview.md)

`src/process_file_lock.rs` implements the repository's single authoritative
cross-process mutual-exclusion primitive: an **advisory kernel lock** taken on
a persistent lock file. On Unix the authority is `flock(2)`; on Windows it is
`LockFileEx`. Unsupported platforms return an explicit error rather than
silently degrading to a no-op lock.

This module covers **only the kernel-backed lock family**. It does *not* cover
the owned-record lock family (`transaction.lock`, `local-data.lock`), which is
a different protocol layered on `create_new` + PID reclaim. See
[Lock Family Boundary](#lock-family-boundary) for the split and the one
asymmetry between the two families, and [persistence.md](persistence.md) for
the owned-record side.

---

## Module Inventory

| Item | Kind | Line | Role |
|------|------|------|------|
| `LOCK_IDENTITY_SCHEMA_VERSION` | const | `src/process_file_lock.rs:58` | On-disk schema version for `LockIdentity` |
| `LockIdentity` | struct | `src/process_file_lock.rs:64` | Diagnostic owner record published into the lock file |
| `ProcessFileLockError` | enum | `src/process_file_lock.rs:82` | Error type for `try_acquire` / `wait_acquire` |
| `ProcessFileLock` | struct | `src/process_file_lock.rs:144` | RAII guard holding the open file + the kernel lock |
| `ProcessFileLock::read_identity_via_handle` | method | `src/process_file_lock.rs:170` | Read own lock file through the already-open handle |
| `try_acquire` | fn | `src/process_file_lock.rs:241` | Nonblocking acquire |
| `wait_acquire` | fn | `src/process_file_lock.rs:251` | Polling acquire with a deadline |
| `read_owner` | fn | `src/process_file_lock.rs:276` | Best-effort diagnostic read of the on-disk record |
| `inner_acquire` | fn (private) | `src/process_file_lock.rs:284` | The real acquire sequence |
| `is_lock_busy_error` | fn (unix) | `src/process_file_lock.rs:406` | Classifies `EWOULDBLOCK` / `EAGAIN` / `EACCES` as contention |
| `publish_identity` | fn (private) | `src/process_file_lock.rs:410` | Truncate + write + `sync_all` the identity record |
| `current_start_token` | fn (private) | `src/process_file_lock.rs:431` | Platform start-time token for the *current* process |
| `parse_linux_proc_start_token` | fn (linux) | `src/process_file_lock.rs:500` | Extracts `/proc/<pid>/stat` field 22 |

---

## Identity Metadata

`LockIdentity` is published **after** the kernel lock is acquired
(`src/process_file_lock.rs:353-360`) and is written into the lock file as
pretty TOML (`src/process_file_lock.rs:413`).

```rust
pub struct LockIdentity {
    pub schema_version: u32,
    pub purpose: String,
    pub pid: u32,
    pub start_token: Option<String>,
    pub nonce: String,
    pub acquired_at_unix_ms: u64,
}
```

| Field | Meaning | Source |
|-------|---------|--------|
| `schema_version` | On-disk format version (`LOCK_IDENTITY_SCHEMA_VERSION`) | `src/process_file_lock.rs:354` |
| `purpose` | Free-form diagnostic label (e.g. `"auto-sync-execution"`) | `src/process_file_lock.rs:355` |
| `pid` | Owner process ID (`std::process::id()`) | `src/process_file_lock.rs:356` |
| `start_token` | Platform start-time token; `None` where the platform cannot supply one | `src/process_file_lock.rs:357` |
| `nonce` | UUID v4 random nonce — distinguishes two acquisitions by the same PID | `src/process_file_lock.rs:358` |
| `acquired_at_unix_ms` | Wall-clock milliseconds at acquisition | `src/process_file_lock.rs:359` |

The nonce is `uuid::Uuid::new_v4().to_string()` (`src/process_file_lock.rs:420-422`).

**The record is diagnostic only.** It may be used in error messages, status
output, and signaling decisions, but it must never authorize lock stealing. A
contender that observes a busy kernel lock with malformed, empty, or legacy
metadata must treat it as a live owner
(`src/process_file_lock.rs:17-24`). A test asserts the record never carries
credential-ish values (`src/process_file_lock.rs:645-670`).

---

## Acquisition Sequence

All of the work happens in `inner_acquire` (`src/process_file_lock.rs:284`):

1. **Create the parent directory** — `create_dir_all` on the parent path
   (`src/process_file_lock.rs:285-287`).
2. **Open the persistent file** with read/write/create and
   **`truncate(false)`** (`src/process_file_lock.rs:291-296`):

   ```rust
   let mut file = OpenOptions::new()
       .read(true)
       .write(true)
       .create(true)
       .truncate(false)
       .open(path)?;
   ```

   `truncate(false)` matters: the open itself does not clear a previous
   owner's bytes. Truncation happens later, inside `publish_identity`
   (`src/process_file_lock.rs:411`). The descriptor stays open for the whole
   lifetime of the guard.
3. **Take the kernel lock** — nonblocking in both platforms (see
   [Unix](#unix-flock) and [Windows](#windows-lockfileex)).
4. **Build the identity record** (`src/process_file_lock.rs:353-360`).
5. **Tighten permissions** — on Unix the lock file is set to `0o600`
   (`src/process_file_lock.rs:363-385`). Failures are logged as warnings, not
   errors: the lock itself is already held and must not be abandoned over a
   permission problem.
6. **Publish the identity** — `set_len(0)`, `seek(0)`, TOML write, `sync_all`
   (`src/process_file_lock.rs:410-418`).
7. **Return the guard** (`src/process_file_lock.rs:398-402`).

### Unix: `flock`

```rust
let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
if rc != 0 {
    let err = std::io::Error::last_os_error();
    if is_lock_busy_error(err.raw_os_error()) {
        let owner = read_owner(path);
        return Err(ProcessFileLockError::Busy { owner });
    }
    return Err(ProcessFileLockError::Io(err));
}
```
— `src/process_file_lock.rs:299-311`

`is_lock_busy_error` treats `EWOULDBLOCK`, `EAGAIN`, and `EACCES` as
contention; anything else (including `EINTR`) is a hard I/O error
(`src/process_file_lock.rs:405-408`, test at `src/process_file_lock.rs:530-537`).

### Windows: `LockFileEx`

```rust
let ok = unsafe {
    LockFileEx(
        handle,
        LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
        0,
        LOCKED_BYTE_COUNT,
        0,
        &mut overlapped,
    )
};
```
— `src/process_file_lock.rs:324-333`

The locked range is a **single byte at offset `u32::MAX`**
(`src/process_file_lock.rs:229-236`). That offset is far beyond any real file
content, so Windows does not deny ordinary read/write access to the lock
file's actual bytes — only another attempt to lock the same range returns
`ERROR_LOCK_VIOLATION`. Failure with raw OS error `33`
(`ERROR_LOCK_VIOLATION`) or `32` (`ERROR_SHARING_VIOLATION`) maps to
`Busy { owner }`; everything else maps to `Io`
(`src/process_file_lock.rs:334-343`).

> **Windows MSVC link note.** `.cargo/config.toml:3-7` sets
> `-C link-arg=/STACK:8388608` for both `x86_64-pc-windows-msvc` and
> `aarch64-pc-windows-msvc`, because the large `Commands` enum in
> `src/main.rs` overflows the 1 MB default Windows stack. This is a
> build-level requirement, not part of this module, but a reviewer changing
> the Windows path must not remove the flag.

### Unsupported platforms

Neither `unix` nor `windows` → `ProcessFileLockError::UnsupportedPlatform`
(`src/process_file_lock.rs:346-350`). Exclusion is never silently weakened.

### Metadata publication failure

If `publish_identity` fails *after* the kernel lock was taken, the lock is
released and an error is returned (`src/process_file_lock.rs:390-396`). No
caller may believe it owns a lock whose on-disk metadata is partial or absent.

---

## Release Semantics

```rust
impl Drop for ProcessFileLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        release_unix_lock(self.file.as_ref());
        #[cfg(windows)]
        release_windows_lock(self.file.as_ref());
        // Dropping the File releases the underlying handle and closes
        // the descriptor. The lock file remains on disk.
        self.file = None;
    }
}
```
— `src/process_file_lock.rs:190-200`

| Platform | Release call | Source |
|----------|--------------|--------|
| Unix | `libc::flock(fd, libc::LOCK_UN)` | `src/process_file_lock.rs:202-210` |
| Windows | `UnlockFileEx(handle, 0, LOCKED_BYTE_COUNT, 0, &mut overlapped)` on the **same byte range** | `src/process_file_lock.rs:212-227` |

**`Drop` releases the kernel lock and closes the descriptor. It does not
unlink, rename, or truncate the lock file.** The file stays on disk with
whatever metadata the last owner wrote.

### Why the file persisting matters

- **File presence is not ownership.** Only the kernel lock state is
  authoritative (`src/process_file_lock.rs:11-15`).
- **Stale metadata is expected.** A crashed owner leaves behind a file whose
  `pid` / `nonce` / `start_token` / `acquired_at_unix_ms` describe a process
  that no longer exists. The next acquirer truncates and overwrites it
  (`src/process_file_lock.rs:411`); no investigation is required.
- **Acquire-after-crash is immediate.** The kernel released the lock when the
  dead process's descriptor was closed, so a new acquirer succeeds on the
  first try — there is no stale-PID classification step in this family
  (`src/process_file_lock.rs:11-15`).
- **The file never accumulates.** A test asserts that 100 acquire/drop cycles
  leave exactly one directory entry (`src/process_file_lock.rs:577-595`); the
  canonical file persists after release
  (`src/process_file_lock.rs:564-575`).
- **`read_owner()` on a free lock returns the previous owner's record.**
  It must never be used to decide that a lock is free — use `try_acquire`.

---

## `try_acquire` vs `wait_acquire`

| Aspect | `try_acquire` | `wait_acquire` |
|--------|---------------|----------------|
| Signature | `try_acquire(path, purpose)` (`:241`) | `wait_acquire(path, purpose, timeout)` (`:251`) |
| Blocking | none | polls the kernel lock every **100 ms** (`:257`) |
| Deadline | n/a | `timeout` measured from entry (`:256`) |
| Contention result | `Busy { owner }` immediately | retries; `Timeout { owner }` at the deadline (`:262-264`) |
| Other errors | `Io` / `UnsupportedPlatform` returned at once | same, returned at once (`:268`) |
| Final sleep | — | `poll_interval.min(remaining)` so the loop cannot overshoot the deadline (`:266`) |
| Acquires the lock itself | calls `inner_acquire` (`:242`) | calls `inner_acquire` in a loop (`:259`) |

The 100 ms poll interval is hard-coded and not configurable; the only knob is
the timeout.

---

## Error Type

```rust
pub enum ProcessFileLockError {
    Busy { owner: Option<LockIdentity> },
    Timeout { owner: Option<LockIdentity> },
    Io(std::io::Error),
    UnsupportedPlatform,
}
```
— `src/process_file_lock.rs:82-97`

| Variant | Payload | Raised when |
|---------|---------|-------------|
| `Busy` | best-effort parsed owner metadata | the kernel lock is held by another process (`src/process_file_lock.rs:307`, `:340`) |
| `Timeout` | best-effort parsed owner metadata | `wait_acquire` reached its deadline while the lock was held (`src/process_file_lock.rs:263`) |
| `Io` | the underlying `std::io::Error` | open / write / sync failure, or a non-contention errno (`src/process_file_lock.rs:309`, `:342`, `:395`) |
| `UnsupportedPlatform` | — | the platform is neither Unix nor Windows (`src/process_file_lock.rs:349`) |

`Display` is implemented at `src/process_file_lock.rs:99-125` and includes
`pid`, `purpose`, and `nonce` when the owner metadata parses
(`src/process_file_lock.rs:103-117`). `From<std::io::Error>` maps to `Io`
(`src/process_file_lock.rs:129-133`).

`owner` is `None` when the lock file is missing, empty, or unparseable — the
lock may still be held (`src/process_file_lock.rs:273-282`).

---

## Process Identity and Start Tokens

`current_start_token()` (`src/process_file_lock.rs:431`) returns the start-time
token of the **current** process. It is captured once, at acquisition time
(`src/process_file_lock.rs:357`).

| Platform | Mechanism | Source |
|----------|-----------|--------|
| Linux | `/proc/<pid>/stat` field 22 (`starttime`, clock ticks since boot) | `src/process_file_lock.rs:432-438` |
| macOS | `proc_pidinfo(PROC_PIDTBSDINFO)` → `pbi_start_tvsec.pbi_start_tvusec` | `src/process_file_lock.rs:439-460` |
| Windows | `GetProcessTimes` → process creation `FILETIME` as a decimal string | `src/process_file_lock.rs:461-491` |
| Other | `None` | `src/process_file_lock.rs:492-496` |

### Linux field 22

```rust
fn parse_linux_proc_start_token(stat: &str) -> Option<String> {
    let after_comm = stat.rfind(')')?;
    let fields: Vec<&str> = stat.get(after_comm + 2..)?.split_whitespace().collect();
    fields.get(19).map(|value| (*value).to_owned())
}
```
— `src/process_file_lock.rs:499-504`

Field 2 of `/proc/<pid>/stat` is `comm`, which may itself contain spaces and
parentheses, so the parser anchors on the **last** `)` and then splits the
remainder on whitespace. Starting from field 3, index 19 is field 22
(1-indexed). The unit test at `src/process_file_lock.rs:511-518` asserts the
parser reads `START22` and *not* `FIELD21`.

> **Duplicated parser.** The same field-22 parser exists independently at
> `src/transaction.rs:98-102` for the owned-record lock family. The logic is
> currently identical; a change to one must be mirrored in the other or the
> two families will disagree about PID identity.

### Liveness: `kill(pid, 0)`

`is_process_alive` is **not** in this module — it lives in
[`src/utils/process.rs`](utils/process.md) so liveness semantics cannot
diverge between lock implementations
(`src/utils/process.rs:1-4`).

The rule that matters: **only `ESRCH` proves absence.** `EPERM` and unknown
errnos mean the process is **live** (`src/utils/process.rs:28-30`).

The kernel-backed family does **not** use liveness to authorize stealing.
`is_stale()` on the execution-lock wrapper
(`src/auto_sync/execution_lock.rs:171-173`, `:287-289`) consults liveness for
display and status output only.

---

## Call Sites

### Wrappers in this repository

| Consumer | Lock file | Purpose label | Acquire API | Source |
|----------|-----------|---------------|-------------|--------|
| Auto-sync execution lock | `<state_dir>/auto-sync-execution.lock` | `auto-sync-execution` | `try_acquire` / `wait_acquire` | `src/auto_sync/execution_lock.rs:22-23`, `:143`, `:152` |
| Auto-sync worker lock | `<state_dir>/auto-sync-worker.lock` | `auto-sync-worker` | `try_acquire_worker` / `wait_acquire_worker` | `src/auto_sync/execution_lock.rs:195-196`, `:277`, `:291` |
| Auto-sync pending-marker lock | `<state_dir>/auto-sync-pending.lock` | `auto-sync-pending` | `acquire_pending_txn` → `wait_acquire` | `src/auto_sync/pending_lock.rs:17-18`, `:91-98` |

`src/auto_sync/lock.rs:3-8` re-exports the worker-lock functions under the old
names (`try_acquire`, `wait_acquire`, `inspect`, `is_stale`, `lock_path`) for
backward compatibility.

### Production acquisition sites

| Site | Mode | Timeout | On contention | Source |
|------|------|---------|---------------|--------|
| Detached auto-sync worker | `try_acquire` | — | `AlreadyHeld` → `WorkerOutcome::NothingToDo`; other errors → `WorkerOutcome::Failed` | `src/auto_sync/worker.rs:64-71` |
| Status deferral release | `try_acquire` | — | logged at debug and skipped | `src/auto_sync/status.rs:339-346` |
| `run_explicit_sync` (TUI `--sync`) | `wait_acquire` | 30 s | propagated | `src/commands/mod.rs:346-349` |
| Manual `snp sync` | `wait_acquire` | 30 s | `Timeout` → "sync already in progress", reports the owner PID | `src/commands/sync_cmd.rs:190-201` |
| Manual sync (second path) | `wait_acquire` | 30 s | same mapping | `src/commands/sync_cmd.rs:528-537` |
| Pending-marker read-modify-write | `acquire_pending_txn` → `wait_acquire` | 10 s (`PENDING_TXN_LOCK_TIMEOUT`) | `PendingTxnLockError::Busy` | `src/auto_sync/pending.rs:17`, `:77`, `:132`, `:241`, `:262`, `:284` |

The execution lock is the **same lock** for the detached worker and for manual
sync — the worker is spawned with `--state-dir <derived state dir>`
(`src/auto_sync/execution_lock.rs:614-619`), so a manual `snp sync` waits
behind a running worker and vice versa.

### Server singleton (`snip-sync` crate)

The `snip-sync` server is a separate crate and cannot call into the binary
crate's module, so it carries a **mirrored implementation** of the same
protocol in `snip-sync/src/server_lock.rs`:

- `ServerLockIdentity` (`snip-sync/src/server_lock.rs:34-47`) — same fields
  minus `purpose`.
- `ServerLock::try_acquire` (`snip-sync/src/server_lock.rs:225`) — same
  `read(true).write(true).create(true).truncate(false)` open, same `flock` /
  `LockFileEx`, same `u32::MAX` byte range
  (`snip-sync/src/server_lock.rs:158-164`).
- `Drop` releases the kernel lock and leaves the file on disk
  (`snip-sync/src/server_lock.rs:121-129`).
- Lock file name: `snip-sync.server.lock`
  (`snip-sync/src/server_lock.rs:48`).

Its module doc states the contract explicitly: the kernel owns mutual
exclusion, the persistent file is diagnostic metadata only, and a new server
can start after a crashed owner exits because the kernel releases the lock at
process teardown (`snip-sync/src/server_lock.rs:1-22`).

---

## Lock Family Boundary

Two distinct lock protocols exist in this repository. They must not be
conflated.

### Kernel-backed family — documented here

| Property | Value |
|----------|-------|
| Authority | OS advisory lock (`flock` / `LockFileEx`) |
| Lock file | persistent, opened `create(true).truncate(false)` |
| Ownership proof | the kernel lock state only |
| On `Drop` | release the kernel lock, **do not unlink** (`src/process_file_lock.rs:190-200`) |
| Stale reclaim | none — not needed; the kernel frees the lock when the owner dies |
| Metadata purpose | diagnostic only |
| Users | auto-sync execution / worker / pending locks; `snip-sync` server singleton |

### Owned-record family — documented in [persistence.md](persistence.md)

| Property | Value |
|----------|-------|
| Authority | `create_new(true)` plus an ownership record written into the file |
| Lock files | `<state_dir>/transaction.lock`, `<state_dir>/local-data.lock` |
| Record | `pid`, `nonce`, `start_token`, `created_at_unix_ms`, `schema_version` (plus `operation` in the transaction record, `src/transaction.rs:988`) |
| On `Drop` | unlink, but only after ownership verification via `remove_owned_lock_file` (`src/transaction.rs:1014`, `src/local_data.rs:93`) |
| Stale reclaim | rename to a quarantine file |
| Users | transaction lock, local-data lock |

The two families coexist in one directory tree but never contend with each
other: they lock different files.

### Known asymmetry between the two owned-record locks

The **local-data** lock's stale reclaim is **ownership-verified**:
`quarantine_local_data_lock(lock_path, observed)`
(`src/local_data.rs:322-354`) re-reads the record and compares it against the
record the caller observed before renaming. Both reclaim call sites pass the
observed record — dead owner (`src/local_data.rs:268`) and PID reuse via
start-token mismatch (`src/local_data.rs:297`). If the record changed, the
rename is skipped and the current owner is left alone.

The **transaction** lock's `quarantine_stale_lock`
(`src/transaction.rs:1268`) is **not** ownership-verified — it takes only a
path and renames unconditionally. Its four call sites are
`src/transaction.rs:1175` (empty record), `:1195` (malformed record), `:1213`
(dead owner), and `:1245` (PID reuse). Because "read the record" and "check
the owner is dead" are not atomic, a writer that decided to reclaim can rename
away a *live* lock that another writer had already reclaimed and re-acquired
in the meantime — putting two processes inside the critical section. The
local-data lock closed exactly this hole; the transaction lock did not.

> **Scope note.** This document covers the kernel-backed family. The
> transaction/local-data locks, their bounded retry ladders, and their
> quarantine semantics are owned by [persistence.md](persistence.md). The
> asymmetry above is reported here because it is a difference between the two
> owned-record locks, and it is not currently described in any architecture
> document.

### Related owned-record lock behavior (for contrast)

- The transaction lock's retry ladder is **platform-gated**:
  `is_transient_lock_contention` is **Windows-only**
  (`src/transaction.rs:1038-1047`). On Unix a `PermissionDenied` on read is
  a stable misconfiguration, so retrying it would spin forever without an
  error or a timeout.
- It also carries a **30 s deadline** as defense in depth
  (`src/transaction.rs:1027`, checked at `:1131-1140`), and a failed
  `write_all` no longer leaves a 0-byte lock file behind
  (`src/transaction.rs:1097-1104`).
- That deadline is what keeps the user-visible commands from hanging: the
  transaction lock is taken by the recovery path
  (`src/transaction.rs:782`), by `snp repair`'s finalize step
  (`src/transaction.rs:886`), by `snp restore`
  (`src/commands/restore_cmd.rs:1147`), and — indirectly — by the mutation
  gate (`src/transaction.rs:2510`), which may acquire it through
  `recover_transaction_by_id`.
- The local-data lock's empty arm uses the same ladder, with backoff
  (`src/local_data.rs:288-289`), a 30 s deadline (`src/local_data.rs:128`), and
  quarantine (`src/local_data.rs:356-370`); a failed `write_all` deletes the
  file it created (`src/local_data.rs:152-163`).
- The lock hierarchy is `LocalDataLock → TransactionLock → writes`
  (`src/commands/restore_cmd.rs:1133`).
- `save_library` derives both its mutation-gate path and its local-data-lock
  path from the **config directory**, not from the target library path
  (`src/library/persistence.rs:264-265`).

---

## Key Files

| Path | Role |
|------|------|
| `src/process_file_lock.rs` | The kernel-backed lock primitive: `try_acquire`, `wait_acquire`, `read_owner`, `LockIdentity`, `ProcessFileLockError` |
| `src/auto_sync/execution_lock.rs` | Execution-lock and worker-lock wrappers over the primitive, plus their error types |
| `src/auto_sync/pending_lock.rs` | Pending-marker lock wrapper (`PendingTxnGuard`) |
| `src/utils/process.rs` | `is_process_alive` and `remove_owned_lock_file` — shared liveness and ownership semantics |
| `src/auto_sync/worker.rs` | Detached worker entry point that takes the execution lock |
| `src/auto_sync/status.rs` | Status/deferral paths that take the execution lock |
| `src/commands/sync_cmd.rs` | Manual sync paths that wait on the execution lock |
| `snip-sync/src/server_lock.rs` | Mirrored kernel-backed singleton lock for the `snip-sync` server |
| `.cargo/config.toml` | Windows MSVC `/STACK:8388608` link flag |

---

## Invariants

1. **The kernel is the authority.** A lock file existing on disk proves
   nothing; only the kernel lock state does
   (`src/process_file_lock.rs:11-15`).
2. **The lock file is never unlinked by `Drop`.** Release is
   `LOCK_UN` / `UnlockFileEx` plus descriptor close
   (`src/process_file_lock.rs:190-200`).
3. **Stale metadata is overwritten, not investigated.** The next acquirer
   truncates and rewrites the record (`src/process_file_lock.rs:411`).
4. **Identity metadata never authorizes stealing.** A busy kernel lock with
   unparseable metadata is still a busy lock
   (`src/process_file_lock.rs:17-24`).
5. **Publication failure releases the lock.** A caller never believes it owns
   a lock whose metadata failed to publish
   (`src/process_file_lock.rs:390-396`).
6. **Unsupported platforms fail loudly** rather than weakening exclusion
   (`src/process_file_lock.rs:346-350`).
7. **Only `ESRCH` proves a process is absent**; `EPERM` and unknown errnos
   mean live (`src/utils/process.rs:28-30`).
8. **The Windows locked range is a single byte at `u32::MAX`**, far beyond
   real file content (`src/process_file_lock.rs:229-236`).

---

## Gotchas

- **Do not treat the lock file as a PID file.** It is never removed on
  release, so after a crash it describes a dead process. Use `try_acquire`
  to learn whether the lock is free, not `read_owner`.
- **Do not add a `create_new`-style "already exists" check.** The file is
  intentionally opened with `create(true).truncate(false)`
  (`src/process_file_lock.rs:291-296`); existence is not contention.
- **Do not add PID-based reclaim here.** The kernel-backed family has no
  reclaim path by design; reclaim lives in the owned-record family
  (see [persistence.md](persistence.md)).
- **Do not reuse `read_owner()` to decide ownership.** It opens a second
  handle and reads whatever bytes are on disk, which may belong to a previous
  owner.
- **Do not open a second handle while holding the lock on Windows** — use
  `read_identity_via_handle()` (`src/process_file_lock.rs:170`), which reads
  through the already-open handle.
- **Do not move or delete the lock file while the guard is alive.** The
  descriptor still points at the original inode; the kernel lock would
  remain held on an orphaned inode.
- **Do not remove the `/STACK:8388608` link flag** from
  `.cargo/config.toml:3-7` while touching the Windows path.
- **`wait_acquire` polls every 100 ms**, so a timeout shorter than 100 ms
  still sleeps up to one interval before returning.
