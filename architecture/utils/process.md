# process.rs — Process Liveness & Owned-Lock-File Removal

[← Back to Overview](../overview.md)

## Overview

Shared process-liveness and owned-lock-file helpers used by the transaction, local-data, and execution locks so liveness semantics cannot diverge between implementations.

**File**: `src/utils/process.rs` (166 lines, including unit tests)

## API

```rust
#[cfg(unix)]
pub(crate) fn is_process_alive(pid: u32) -> bool

#[cfg(unix)]
pub(crate) fn classify_kill_zero_error(errno: Option<i32>) -> bool

#[cfg(windows)]
pub(crate) fn is_process_alive(pid: u32) -> bool

pub(crate) fn remove_owned_lock_file(
    path: &Path,
    owner_nonce: &str,
    owner_pid: u32,
    owner_start_token: Option<&str>,
) -> bool
```

All items are `pub(crate)` — shared infrastructure, not public API.

## is_process_alive

Unix implementation calls `libc::kill(pid, 0)` (signal 0 = no delivery, existence probe only):

- `rc == 0` → alive (process exists and we may signal it).
- `rc != 0` → consult `classify_kill_zero_error`: only `ESRCH` proves absence. `EPERM` (exists, different user) and unknown errnos conservatively count as **alive**.

```rust
pub(crate) fn classify_kill_zero_error(errno: Option<i32>) -> bool {
    !matches!(errno, Some(libc::ESRCH))
}
```

Two hard rules:

1. **PID 0 is never a valid lock owner.** `kill(0, 0)` targets the caller's process group and would always succeed, so PID 0 returns `false` immediately.
2. **Conservative liveness**: anything but proven-absent means alive. A contender must never reclaim a lock whose owner might still run.

Windows implementation opens the PID with `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` and checks the exit code against `STILL_ACTIVE`. Null handle (no such process / no access) → dead... in practice treated the same conservative way by callers.

## remove_owned_lock_file

Removes a lock file only if it still holds our ownership record. Returns `true` only when this call removed the file. Verification order:

1. Open a handle and read the record through it (TOML `Table` with `nonce`, `pid`, `start_token`).
2. Require nonce **and** PID **and** start-token to all match (including `None == None`).
3. On Unix, compare the handle's `(dev, ino)` against a fresh `stat` of `path` — closes the TOCTOU window where a concurrent quarantine + re-acquire could swap a different owner's lock into place between verification and unlink.
4. `remove_file(path)`; result is the return value.

Malformed/unreadable records and identity mismatches return `false` without touching the file.

## Consumers

| Consumer | Use |
|----------|-----|
| `src/transaction.rs` | `is_process_alive` for dead-owner reclaim of `transaction.lock`; `remove_owned_lock_file` for nonce + start-token-verified `Drop` |
| `src/local_data.rs` | `remove_owned_lock_file` for `LocalDataLock` release |
| `src/auto_sync/execution_lock.rs` | `is_process_alive` for stale sync-execution/worker lock diagnostics |

## Invariants

- Liveness is diagnostic/reclaim input only; the kernel lock (`flock`/`LockFileEx`) is the sole mutual-exclusion authority (see [persistence.md](../persistence.md)).
- Ownership comparison uses the *observed* start token of the recorded PID, never the contender's own token — a live owner is never misclassified as PID reuse.
- PID 0 is dead by definition on every platform.

## Tests

In-module tests cover matching-record removal plus nonce, PID, and start-token mismatch refusal. `transaction.rs` and `execution_lock.rs` unit-test the ESRCH/EPERM/EINVAL classification matrix.
