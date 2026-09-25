# Process File Lock (kernel-backed primitive)

[← Back to Overview](overview.md)

## Overview

`src/process_file_lock.rs` (770 lines) is the single authoritative
cross-process mutual-exclusion primitive. Authority is the OS kernel —
Unix `flock(LOCK_EX | LOCK_NB)`, Windows `LockFileEx` over one fixed byte —
never PID files, timestamps, or metadata comparison. File presence means
nothing; only kernel lock state decides ownership, so acquire-after-crash is
immediate with no stale-PID classification. A parallel same-protocol
implementation guards the server singleton (`snip-sync/src/server_lock.rs`).

## Key types

| Type | Location | Notes |
|------|----------|-------|
| `LockIdentity` | `process_file_lock.rs:64` | Diagnostic-only record: `schema_version` (`LOCK_IDENTITY_SCHEMA_VERSION = 1`, `:58`), `purpose` label, `pid`, `start_token`, `nonce` (UUID v4), `acquired_at_unix_ms` |
| `ProcessFileLockError` | `:82` | `Busy { owner }` / `Timeout { owner }` (best-effort owner metadata) / `Io` / `UnsupportedPlatform` (refuses to weaken exclusion, `:96`) |
| `ProcessFileLock` | `:144` | RAII guard: `path()` (`:152`), `identity()` (`:157`), `nonce()` (`:162`), `read_identity_via_handle()` (`:170`, avoids a second handle that would conflict with the Windows byte range) |
| `try_acquire` / `wait_acquire` | `:241`, `:251` | Nonblocking attempt; poll-every-100 ms until deadline, then `Timeout` preserving the last owner |
| `read_owner` | `:276` | Best-effort metadata read; `None` on missing/empty/unreadable — diagnostic only, never an availability signal |

## Acquisition order (`inner_acquire`, `:284`)

1. `create_dir_all(parent)` (`:285`).
2. Open read/write/create, no truncate (`:291`) — the descriptor lives as long as the guard.
3. Nonblocking kernel lock: `flock(fd, LOCK_EX|LOCK_NB)` (`:302`); `EWOULDBLOCK`/`EAGAIN`/`EACCES` → `Busy` with `read_owner` attached (`:303`, busy predicate `:405`). Windows: `LockFileEx(EXCLUSIVE|FAIL_IMMEDIATELY)` on **one byte at offset `u32::MAX`** (`LOCKED_BYTE_COUNT/OFFSET`, `:229`) — far beyond real content so file I/O stays shareable; `ERROR_LOCK_VIOLATION (33)` / `ERROR_SHARING_VIOLATION (32)` → `Busy` (`:338`). Other platforms → `UnsupportedPlatform` (`:346`).
4. Build `LockIdentity`: pid, `current_start_token()` (`:431`), fresh nonce, unix-ms timestamp (`:353`).
5. Unix: tighten the file to `0o600` (warn-only on failure, `:362`).
6. `publish_identity` (`:410`): `set_len(0)`, seek, pretty-TOML write, `sync_all`. **Any failure releases the kernel lock and errors** (`:390`) — nobody holds ownership with partial metadata.
7. Return the guard (`:398`).

## Release and file lifecycle

- `Drop` (`:190`) calls `release_unix_lock` (`LOCK_UN`, `:202`) or `release_windows_lock` (matching `UnlockFileEx` range, `:212`), then drops the `File`. **It never unlinks or renames** — the canonical lock file persists with stale metadata, which the next acquirer overwrites without investigation.
- Cancellation-safe (`:41`): Drop touches no filesystem state; the kernel alone arbitrates the next acquirer.
- Exactly one file ever exists per lock: 100 acquire/drop cycles leave one entry (test, `:577`); release preserves the canonical file (`:564`); malformed (`:597`) or empty (`:608`) content never blocks a free lock.
- `remove_owned_lock_file` (`src/utils/process.rs:60`) — unlink only when nonce+pid+start-token read through the open handle still match, with a Unix `(dev, ino)` TOCTOU check (`:97`) — is used by the legacy lock-file paths, never by this guard.

## Liveness semantics (diagnostic only)

- Unix `is_process_alive` (`src/utils/process.rs:14`): `kill(pid, 0)`; PID 0 is always dead; otherwise alive unless `errno == ESRCH` (`classify_kill_zero_error`, `:22`). **Only `ESRCH` proves absence — `EPERM` or unknown means live.** Windows checks `STILL_ACTIVE` exit codes (`:32`).
- Linux start token (`process_file_lock.rs:499`): parse `/proc/<pid>/stat` by cutting after the **final `)`** (comm may contain spaces/parens, `:501`), then take `fields[19]` = field 22 (pinned by test, `:511`). macOS uses `proc_pidinfo(PROC_PIDTBSDINFO)` (`:439`); Windows uses process creation time (`:461`); elsewhere `None`.
- `is_stale` helpers on the wrappers consult liveness for display/status only — a "stale" reading never authorizes stealing; contenders that see a busy kernel lock with malformed/empty/legacy metadata treat it as a live owner (module docs, `:17`).

## Wrappers

| Wrapper | Location | Lock file / purpose |
|---------|----------|---------------------|
| `SyncExecutionLock` | `src/auto_sync/execution_lock.rs:104` | `execution_lock_path` (`:134`); shared by manual sync and the detached worker; `try_acquire` (`:143`), `wait_acquire` (`:152`), `inspect` (`:166`, diagnostic-only), `is_stale`/`process_alive` (`:171`, delegate to `utils::process`, `:180`) |
| `WorkerLock` | `execution_lock.rs:246` | `auto-sync-worker.lock` / `"auto-sync-worker"` (`:195`); `try_acquire_worker` (`:277`), `wait_acquire_worker` (`:291`); parent never holds it, scheduler never probes the execution lock |
| `PendingTxnGuard` | `src/auto_sync/pending_lock.rs:54` | Transaction-scoped pending-marker lock; `acquire_pending_txn` (`:91`), `read_owner_via_handle` (`:66`) |
| `ServerLock` | `snip-sync/src/server_lock.rs:90` | **Sibling implementation, not a wrapper**: own `ServerLockIdentity` (`:29`), own release fns and `u32::MAX` range (`:123`, `:149`), `try_acquire(state_dir)` (`:165`), `read_owner` (`:273`); same lifecycle rules (nonblocking, no unlink, no PID file) |

## Error display

`Display` (`:99`) keeps contention errors greppable and safe:

- `Busy` with owner → `process lock busy (pid=…, purpose=…, nonce=…)` (`:103`).
- `Busy` without readable metadata → bare `process lock busy` (`:108`) — still a live owner, never a license to proceed.
- `Timeout` mirrors both forms with `timed out waiting for …` (`:110`).
- `UnsupportedPlatform` states the refusal explicitly (`:119`).

Wrapper errors (`ExecutionLockError`, `WorkerLockError`, `ServerLockError`)
preserve the pid/nonce projection when converting from these variants
(`execution_lock.rs:91`, `:223`; `server_lock.rs:60`).

## Windows byte range

```text
offset 0                          u32::MAX
│ file content (shared R/W)         │1-byte exclusive range│
├───────────────────────────────────┼─────────────────────┤
│ lock metadata TOML                │ LOCKED_BYTE_OFFSET  │
│ readable/writable by anyone       │ contended by lockers│
```

Lock and unlock ranges must match (`UnlockFileEx` with the same offset,
`:218`); only a second `LockFileEx` on that byte fails, so concurrent
`read_owner` calls never block acquisition.

## Tests (selected)

| Test | Location | Pins |
|------|----------|------|
| first acquisition publishes identity | `:521` | purpose/pid/nonce present |
| same-process second acquire → `Busy` with owner nonce | `:540` | kernel, not metadata, decides |
| drop allows re-acquire with new nonce | `:554` | release works |
| canonical file persists after release | `:564` | no unlink |
| 100 cycles → exactly 1 dir entry | `:577` | no quarantine/sidecar files |
| malformed / empty content, lock free → acquire | `:597`, `:608` | metadata never gates |
| `wait_acquire` success after release / timeout at deadline | `:618`, `:633` | polling + `Timeout` |
| no secret-adjacent values in lock file | `:644` | pid/nonce/timestamps only |
| Unix `0o600` on the lock file | `:672` | private metadata |
| Linux field-22 parser with `)`-in-comm | `:511` | `rfind(')')` + `fields[19]` |
| identity TOML round-trip, nonce uniqueness | `:698`, `:705` | format stability |

## Invariants

- Kernel state is the only ownership signal; metadata is diagnostic-only and never authorizes stealing.
- No `.quarantine`, `.bak`, or counter files: one canonical path per lock, overwritten in place.
- Publish-or-release: kernel acquisition without durable identity is always rolled back.
- Unsupported platforms error instead of degrading to non-exclusion.
- Lock records carry no secrets (pid/nonce/timestamps only; value-scan test, `:644`) and lock files are `0o600` on Unix (test, `:672`).

## Key files

- `src/process_file_lock.rs` — primitive, identity, acquire/release, start tokens
- `src/utils/process.rs:14` (liveness), `:60` (owned-remove for legacy paths)
- `src/auto_sync/execution_lock.rs:104`, `:246` — execution + worker guards
- `src/auto_sync/pending_lock.rs:54` — pending-transaction guard
- `snip-sync/src/server_lock.rs:90` — server singleton sibling
