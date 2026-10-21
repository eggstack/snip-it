# Process Utilities

[← Back to Overview](../overview.md)

`src/utils/process.rs` holds the two process-level primitives that every lock
implementation in the repository shares: a **liveness probe**
(`is_process_alive`) and an **ownership-checked lock-file removal**
(`remove_owned_lock_file`). The module exists so that liveness and ownership
semantics cannot diverge between the kernel-backed lock family
([process_file_lock.md](../process_file_lock.md)) and the owned-record family
(`transaction.lock`, `local-data.lock`, see
[persistence.md](../persistence.md)).

This module does **not** implement any lock, does not own the
`/proc/<pid>/stat` start-token parser used by the lock records (that lives in
`src/process_file_lock.rs:499` and `src/transaction.rs:98`), and does not
decide when it is safe to reclaim a stale lock — it only answers the two
narrow questions those callers need.

---

## Module Inventory

| Item | Kind | Line | Visibility | Role |
|------|------|------|------------|------|
| `is_process_alive` | fn (unix) | `src/utils/process.rs:14` | `pub(crate)` | `kill(pid, 0)` liveness probe |
| `classify_kill_zero_error` | fn (unix) | `src/utils/process.rs:23` | `pub(crate)` | Maps an errno to "still alive" |
| `is_process_alive` | fn (windows) | `src/utils/process.rs:32` | `pub(crate)` | `OpenProcess` + `GetExitCodeProcess` liveness probe |
| `remove_owned_lock_file` | fn | `src/utils/process.rs:60` | `pub(crate)` | Unlink a lock file only if it still holds our ownership record |

---

## `is_process_alive`

```rust
#[cfg(unix)]
pub(crate) fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let rc = unsafe { libc::kill(pid as i32, 0) };
    rc == 0 || classify_kill_zero_error(std::io::Error::last_os_error().raw_os_error())
}
```
— `src/utils/process.rs:13-20`

### Unix semantics — `kill(pid, 0)`

`kill(pid, 0)` performs the permission and existence checks of `kill(pid, sig)`
without delivering a signal. The result is interpreted as follows:

| Result | Meaning | Source |
|--------|---------|--------|
| `rc == 0` | the process exists and is signalable → **alive** | `src/utils/process.rs:19` |
| `rc != 0`, errno `ESRCH` | no such process → **dead** | `src/utils/process.rs:24` |
| `rc != 0`, errno `EPERM` | the process exists but is not ours to signal → **alive** | `src/utils/process.rs:24` |
| `rc != 0`, any other / unknown errno | cannot prove absence → **alive** | `src/utils/process.rs:24` |

```rust
#[cfg(unix)]
pub(crate) fn classify_kill_zero_error(errno: Option<i32>) -> bool {
    !matches!(errno, Some(libc::ESRCH))
}
```
— `src/utils/process.rs:22-25`

**Only `ESRCH` proves absence. `EPERM` and unknown errnos mean the process is
live.** The function is deliberately conservative: it never reports a process
as dead unless the kernel said `ESRCH`.

The inverse of that rule is the important one for lock safety: because
"cannot prove absence" is reported as *alive*, an unknown errno can never
authorize reclaiming somebody else's lock.

### Windows semantics — `OpenProcess` + `GetExitCodeProcess`

```rust
#[cfg(windows)]
pub(crate) fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        ok != 0 && exit_code == STILL_ACTIVE as u32
    }
}
```
— `src/utils/process.rs:31-50`

| Step | Result | Source |
|------|--------|--------|
| `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid)` returns a null handle | reported **dead** | `src/utils/process.rs:42-44` |
| `GetExitCodeProcess` fails | reported **dead** | `src/utils/process.rs:48` |
| `GetExitCodeProcess` succeeds and the exit code is `STILL_ACTIVE` (259) | reported **alive** | `src/utils/process.rs:48` |
| `GetExitCodeProcess` succeeds with any other exit code | reported **dead** | `src/utils/process.rs:48` |

### Platform difference (important)

The two platforms are **not** equally conservative:

| Situation | Unix | Windows |
|-----------|------|---------|
| Process does not exist | `ESRCH` → dead | `OpenProcess` null → dead |
| Process exists but is protected (`EPERM` / access denied) | **alive** | **dead** (null handle, `GetLastError` not consulted) |
| Platform cannot determine the answer | **alive** | **dead** |

On Windows the null-handle branch collapses "no such process" and "access
denied" into the same answer (`src/utils/process.rs:42-44`), and `GetLastError`
is never inspected. A live but protected process is therefore reported as
**dead** on Windows while it is reported as **alive** on Unix. This is the
reason the Windows liveness result must not be used to authorize stealing a
lock — the same conservative policy the owned-record locks already apply for
the *unobservable start token* case (see
[persistence.md](../persistence.md)).

A second, smaller Windows edge case: `STILL_ACTIVE` is the value `259`, so a
process that legitimately exits with code `259` is indistinguishable from a
running process.

### PID 0 is never a valid owner

Both implementations short-circuit `pid == 0` to `false`
(`src/utils/process.rs:15-17`, `:33-35`). On Unix, `kill(0, 0)` targets the
caller's **process group** and would always succeed, so a lock record with
PID `0` would read as permanently alive. Treating PID 0 as dead removes that
trap.

### Consumers

| Consumer | Use | Source |
|----------|-----|--------|
| `ProcessIdentity::observe` | `None` (dead) vs `Some(identity)` for a live PID — the basis of transaction / local-data reclaim | `src/transaction.rs:62-70` |
| `auto_sync::execution_lock::process_alive` | public re-export used by `is_stale()` helpers | `src/auto_sync/execution_lock.rs:180`, `:189` |

`is_process_alive` is `pub(crate)`; the public surface is the re-export in
`src/auto_sync/execution_lock.rs:180-191`, which exists so the two lock
families share one implementation.

---

## `remove_owned_lock_file`

```rust
pub(crate) fn remove_owned_lock_file(
    path: &Path,
    owner_nonce: &str,
    owner_pid: u32,
    owner_start_token: Option<&str>,
) -> bool
```
— `src/utils/process.rs:60-65`

Removes a lock file **only if it still holds our ownership record**. It
returns `true` only when this call actually removed the file.

### Algorithm

1. **Open the file** and read the content *through that handle*
   (`src/utils/process.rs:70-76`). A read failure aborts with `false`.
2. **Parse as `toml::Table`**, not `toml::Value`
   (`src/utils/process.rs:81-83`). A lock record is a top-level TOML
   *document* with several keys; `toml::Value` only accepts a single
   standalone value and would reject it. A parse failure aborts with
   `false`.
3. **Compare all three identity fields** (`src/utils/process.rs:85-93`):
   `nonce` (string), `pid` (integer), and `start_token` (string or `None`).
   Any mismatch aborts with `false`.
4. **Confirm the file at `path` is still the file we verified** — on Unix,
   compare the opened handle's `(dev, ino)` against a fresh stat of `path`
   (`src/utils/process.rs:97-104`). A mismatch aborts with `false`.
5. **Unlink** — `fs::remove_file(path).is_ok()`
   (`src/utils/process.rs:112`).

### The TOCTOU window it closes

Steps 1–3 read the record and compare identities; step 5 unlinks. Between
those two points a concurrent writer could quarantine the lock and re-acquire
it, putting a *different* owner's record at `path`. Unlinking blindly would
destroy that new owner's lock file and let a second owner into the critical
section.

The Unix `(dev, ino)` comparison in step 4 collapses that window: if the file
at `path` is no longer the same inode whose contents were verified, the
removal is skipped.

On Windows there is no stable file-id API in `std`, so the handle-based
content check is the strongest verification available
(`src/utils/process.rs:105-110`). The same trade-off applies.

### When it is safe to use

| Situation | Safe? | Why |
|-----------|-------|-----|
| You currently hold the lock and are releasing it | **Yes** | The record on disk is yours unless a reclaim already replaced it — and then the comparison refuses |
| You are reclaiming a lock you never owned | **No** | The nonce / PID / start-token comparison will refuse; use the quarantine path instead |
| The lock file is empty or malformed | **No** | Step 2 cannot parse it, so the function refuses — the file is left in place for the quarantine path to handle |
| The lock file is a *different* owner's (same PID, new nonce) | **No** | The nonce comparison refuses |

### Consumers

| Consumer | Source |
|----------|--------|
| `TransactionLock::drop` | `src/transaction.rs:1007-1021` (call at `:1014`) |
| `LocalDataLock::drop` | `src/local_data.rs:87-100` (call at `:93`) |

Both call sites pass the guard's own `nonce`, `pid`, and `start_token`, so the
lock file is removed on a clean release and left alone if another owner has
already replaced it.

---

## Key Files

| Path | Role |
|------|------|
| `src/utils/process.rs` | `is_process_alive` (Unix + Windows) and `remove_owned_lock_file` |
| `src/utils/mod.rs` | Module inventory root; declares `pub mod process` (`src/utils/mod.rs:13`) |
| `src/transaction.rs` | `ProcessIdentity::observe` (`src/transaction.rs:62`) and `TransactionLock::drop` (`src/transaction.rs:1007`) — the liveness / ownership consumers |
| `src/local_data.rs` | `LocalDataLock::drop` (`src/local_data.rs:87`) — the other ownership consumer |
| `src/auto_sync/execution_lock.rs` | Public re-export `process_alive` (`src/auto_sync/execution_lock.rs:180`, `:189`) and the `is_stale` helpers |
| `architecture/process_file_lock.md` | The kernel-backed lock family that shares these semantics |
| `architecture/persistence.md` | The owned-record lock family (transaction / local-data) that owns the reclaim protocol |

---

## Invariants

1. **Only `ESRCH` proves a process is absent.** `EPERM` and unknown errnos
   are reported as alive (`src/utils/process.rs:23-25`).
2. **PID 0 is never a valid lock owner** and is reported dead on every
   platform (`src/utils/process.rs:15-17`, `:33-35`).
3. **A lock file is removed only when nonce, PID, and start token all match**
   (`src/utils/process.rs:85-93`).
4. **Removal additionally requires that the file at `path` is the same
   inode that was verified** on Unix (`src/utils/process.rs:97-104`).
5. **An unparseable or empty lock file is never removed by this helper** —
   it is left for the quarantine path (`src/utils/process.rs:81-83`).
6. **`remove_owned_lock_file` returns `true` only on an actual removal**
   (`src/utils/process.rs:112`).

---

## Gotchas

- **`is_process_alive` is `pub(crate)`.** Use the re-export at
  `src/auto_sync/execution_lock.rs:180` rather than widening this module's
  visibility.
- **Do not treat `false` as proof of death on Windows.** The null-handle
  branch (`src/utils/process.rs:42-44`) also fires for access-denied. The
  Unix path is the conservative one; the Windows path is not.
- **Do not use `is_process_alive` to authorize reclaiming a lock.** It is
  used for reclaim decisions only in combination with the start-token
  comparison in `ProcessIdentity::observe`
  (`src/transaction.rs:62-70`, `:1225-1227`), and for display/status in the
  kernel-backed family.
- **Do not replace `toml::Table` with `toml::Value`** in
  `remove_owned_lock_file` (`src/utils/process.rs:81`). A lock record has
  multiple top-level keys and `toml::Value` cannot parse it — the helper
  would silently stop removing anything.
- **Do not skip the `(dev, ino)` check** on Unix
  (`src/utils/process.rs:97-104`); it is the guard against unlinking a
  concurrently re-acquired lock.
- **Do not call `remove_owned_lock_file` on a lock file you did not create.**
  It will refuse, but relying on the refusal rather than the ownership
  discipline is how the TOCTOU bug gets reintroduced.
