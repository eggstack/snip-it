# Temp File Guard

[← Back to Overview](../overview.md)

`src/utils/tempfile_guard.rs` is a 38-line RAII guard that deletes a temporary
file when it goes out of scope, unless the caller explicitly takes ownership of
it with `persist()`. It exists so the atomic-write helpers in
[`src/utils/atomic.rs`](atomic.md) can never leave an orphaned temp file
behind when a write fails partway through.

The guard does **not** manage the write itself, does not decide durability,
does not fsync anything, and does not sweep up temp files left behind by other
processes or by earlier crashes. It is a single-path, best-effort unlink on
drop.

---

## Module Inventory

| Item | Kind | Line | Role |
|------|------|------|------|
| `TempFileGuard` | struct | `src/utils/tempfile_guard.rs:6` | Holds the temp file path to delete on drop |
| `TempFileGuard::new` | fn | `src/utils/tempfile_guard.rs:11` | Construct a guard for `path` |
| `TempFileGuard::persist` | fn | `src/utils/tempfile_guard.rs:17` | Consume the guard *without* deleting the file |
| `impl Drop for TempFileGuard` | impl | `src/utils/tempfile_guard.rs:22` | Deletes the file on drop |

```rust
pub struct TempFileGuard {
    path: PathBuf,
}
```
— `src/utils/tempfile_guard.rs:6-8`

---

## Lifecycle

### Construction

```rust
pub fn new(path: PathBuf) -> Self {
    Self { path }
}
```
— `src/utils/tempfile_guard.rs:11-13`

The guard is created **immediately after** the temp path is computed and
**before** the temp file is created. In both atomic-write helpers the guard is
armed first and the file is created second, so a failure to create the file
still leaves the guard armed for a path that does not exist — the resulting
`NotFound` on drop is swallowed
(`src/utils/atomic.rs:280-281`, `:370-371`).

### Persist — the success path

```rust
/// Consume the guard without deleting the file. Call this after a
/// successful `fs::rename` to prevent cleanup on drop.
pub fn persist(mut self) {
    self.path = PathBuf::new();
}
```
— `src/utils/tempfile_guard.rs:15-19`

`persist` takes `self` by value, so the guard is **consumed** — it cannot be
persisted twice, and the compiler enforces that the caller no longer holds it.
It clears the path to an empty `PathBuf`; `Drop` then checks
`self.path.as_os_str().is_empty()` and does nothing
(`src/utils/tempfile_guard.rs:24`).

### Drop — the failure path

```rust
impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            match std::fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    tracing::warn!(
                        path = %self.path.display(),
                        error = %e,
                        "TempFileGuard failed to delete temp file"
                    );
                }
            }
        }
    }
}
```
— `src/utils/tempfile_guard.rs:22-37`

| Drop condition | Action |
|----------------|--------|
| `persist()` was called (path is empty) | nothing — the file is intentionally kept |
| File exists | `fs::remove_file` removes it |
| File is already gone | `NotFound` is swallowed silently |
| Removal fails for another reason | logged via `tracing::warn!` with the path and error; **never panics, never propagates** |

Because `Drop` runs during unwinding, the temp file is also cleaned up when
the write path panics.

---

## What It Cleans Up — and What It Does Not

### It cleans up

- The single temp file at the exact path the guard was constructed with, when
  the surrounding scope exits without `persist()`.
- That covers every early `return Err(...)` between guard construction and the
  successful rename: temp-file creation failure, write failure, flush failure,
  `sync_all` failure, and rename failure.

### It does **not** clean up

| Not cleaned up | Why it matters |
|----------------|----------------|
| The **target** file after a successful rename | The content now lives at the target path; the guard is disarmed by `persist()` |
| Temp files left by **another process** | Each process only guards its own path |
| Temp files left by an **earlier crash** (SIGKILL) | `Drop` never runs; there is no startup sweep of `*.tmp` files anywhere in the codebase |
| The **parent directory** | Only the file is unlinked; the directory stays |
| Any **fsync / directory sync** | The guard has no durability role at all — see [atomic.md](atomic.md) for the durability classes |
| The **quarantine** files of the owned-record locks | Those are managed by `prune_quarantine_files` (`src/transaction.rs:1310`), not by this guard |
| A file that **replaced** the temp path after creation | The guard deletes **by path, not by inode**. The temp name is UUID-based, so a collision is effectively impossible, but the guard does not verify identity |

---

## Interaction with the Atomic Write Path

`TempFileGuard` has exactly two production call sites, both in
`src/utils/atomic.rs`:

| Helper | Temp path format | Guard armed | `persist()` called |
|--------|------------------|-------------|--------------------|
| `write_private_atomic` | `{temp_prefix}.{uuid}.tmp` (`src/utils/atomic.rs:280`) | `src/utils/atomic.rs:281` | `src/utils/atomic.rs:315` |
| `atomic_replace` | `{uuid}.tmp` (`src/utils/atomic.rs:370`) | `src/utils/atomic.rs:371` | `src/utils/atomic.rs:409` |

In both helpers the ordering is the same and is the whole point of the guard:

```text
compute temp path  →  arm guard  →  create temp  →  write  →  [flush]  →  [sync_all]
                                                                   ↓
                                                     rename over target
                                                                   ↓
                                                       persist()  →  disarm guard
```

Because `persist()` is called **only after** `fs::rename` succeeds, any
failure before the rename leaves the guard armed and the temp file is removed
on drop. Any failure *after* the rename leaves the guard disarmed, so the
newly written target file is never deleted.

### Ordering detail in `atomic_replace`

`atomic_replace` continues past the rename with two more steps
(`src/utils/atomic.rs:411-429`):

1. restore the original permissions if `preserve_permissions` was requested
   (best-effort; a failure is ignored, `src/utils/atomic.rs:412-426`);
2. sync the parent directory (`src/utils/atomic.rs:429`).

Both run **after** `persist()` (`src/utils/atomic.rs:409`). That matters: if
the parent-directory fsync fails and the call returns an error
(`src/utils/atomic.rs:429`), the target file has already been written and the
guard will not delete it. The error means "the rename may not survive power
loss", not "the write was rolled back".

### Test coverage

The guard has no dedicated unit tests of its own — it is covered through the
atomic-write tests:

- `test_atomic_replace_no_temp_file_on_success`
  (`src/utils/atomic.rs:645-659`) — no `*.tmp` files remain after a
  successful replace.
- `test_atomic_replace_no_temp_file_on_failure`
  (`src/utils/atomic.rs:661`) — no `*.tmp` files remain after a failed
  replace.
- `tests/auto_sync_concurrency.rs:401-449` — no UUID-based `.tmp` files
  accumulate across a burst of mutations.

---

## Key Files

| Path | Role |
|------|------|
| `src/utils/tempfile_guard.rs` | The guard itself: `new`, `persist`, `Drop` |
| `src/utils/atomic.rs` | The only production consumer — `write_private_atomic` (`:281`, `:315`) and `atomic_replace` (`:371`, `:409`) |
| `src/utils/mod.rs` | Declares `pub mod tempfile_guard` (`src/utils/mod.rs:16`) |
| `tests/auto_sync_concurrency.rs` | Integration check that no `.tmp` files accumulate |

---

## Invariants

1. **`persist()` consumes the guard** — it cannot be called twice, and the
   caller no longer holds it afterwards (`src/utils/tempfile_guard.rs:17`).
2. **`persist()` is called only after a successful `fs::rename`**
   (`src/utils/atomic.rs:315`, `:409`), so the target file is never removed by
   the guard.
3. **A failed write never leaves a temp file behind** — every error path
   between guard construction and the rename drops the guard armed
   (`src/utils/tempfile_guard.rs:22-37`).
4. **`Drop` never panics and never propagates an error.** A removal failure
   is logged, not raised (`src/utils/tempfile_guard.rs:28-34`).
5. **`NotFound` on drop is expected and silent** (`src/utils/tempfile_guard.rs:27`).
6. **The guard touches exactly one path** and no other filesystem state
   (`src/utils/tempfile_guard.rs:6-8`).

---

## Gotchas

- **Do not call `persist()` before the rename succeeds.** Disarming the guard
  early defeats its entire purpose — the temp file survives the failed write.
- **Do not arm the guard on a path you did not create.** The guard deletes by
  path; arming it on a shared or predictable path risks deleting someone
  else's file.
- **Do not expect `persist()` to be required for correctness of the success
  path.** After a successful rename the temp path no longer exists, so the
  drop would hit `NotFound` and be swallowed. `persist()` is the *explicit*
  contract; skipping it leaves the intent implicit and makes the code
  fragile if the write strategy ever changes.
- **Do not treat the guard as a crash-cleanup mechanism.** A `SIGKILL`ed
  process never runs `Drop`; orphaned `*.tmp` files from a crash are not
  swept by anything.
- **Do not use the guard for the lock files.** The kernel-backed lock family
  deliberately leaves its lock file on disk
  ([process_file_lock.md](../process_file_lock.md)); the owned-record family
  removes it only after ownership verification
  ([persistence.md](../persistence.md)). Neither uses `TempFileGuard`.
- **Do not add fsync or directory-sync behavior here.** Durability is
  entirely the business of `src/utils/atomic.rs` — see
  [atomic.md](atomic.md) for the four durability classes.
