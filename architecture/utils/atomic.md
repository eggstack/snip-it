# Atomic File Writes

[← Back to Overview](../overview.md)

`src/utils/atomic.rs` provides the repository's write primitives: a simple
atomic write (`write_private_atomic`), a durability-aware atomic replace with
target validation and permission control (`atomic_replace`), a thin
convenience wrapper (`atomic_write_bytes`), and the canonical SHA-256 file
hashing helper (`hash_file`).

The module owns the **durability classes** — the rules that decide whether a
written file is fsynced, whether the parent directory is fsynced, and
whether a symlink target is rejected. It does **not** own the mutation gate,
the transaction journal, the lock hierarchy, or the TOML fidelity rules — see
[persistence.md](../persistence.md) for those.

---

## Module Inventory

| Item | Kind | Line | Role |
|------|------|------|------|
| `Durability` | enum | `src/utils/atomic.rs:19` | The four durability classes |
| `AtomicWriteOptions` | struct | `src/utils/atomic.rs:40` | Durability + permission + symlink policy |
| `AtomicWriteOptions::for_durability` | fn | `src/utils/atomic.rs:56` | Default options for a class |
| `AtomicWriteOptions::preserve_permissions` | fn | `src/utils/atomic.rs:66` | Builder setter |
| `AtomicWriteOptions::reject_symlink` | fn | `src/utils/atomic.rs:72` | Builder setter |
| `AtomicWriteReport` | struct | `src/utils/atomic.rs:81` | What `atomic_replace` actually did |
| `write_private_atomic` | fn | `src/utils/atomic.rs:273` | Simple atomic write of a string |
| `atomic_replace` | fn | `src/utils/atomic.rs:345` | Durability-aware atomic replace |
| `atomic_write_bytes` | fn | `src/utils/atomic.rs:443` | Convenience wrapper over `atomic_replace` |
| `hex_encode` | fn | `src/utils/atomic.rs:453` | Lowercase hex digest encoding |
| `hash_file` | fn | `src/utils/atomic.rs:470` | Canonical SHA-256 hex digest of a file |

---

## Durability Classes

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[allow(dead_code)]
pub enum Durability {
    DurableUserData,
    SensitiveConfig,
    RecoverableMetadata,
    EphemeralCoordination,
}
```
— `src/utils/atomic.rs:16-35`

`Durability` is `#[non_exhaustive]`, so a `match` over it must have a
wildcard arm — adding a class later is not a breaking change for downstream
code.

### Class matrix

| Class | File `sync_all` | Parent-dir fsync | Temp-file mode (Unix) | Symlink reject | Dir-fsync failure |
|-------|-----------------|------------------|-----------------------|----------------|-------------------|
| `DurableUserData` | **Yes** (`src/utils/atomic.rs:397-403`) | **Yes**, fail-closed (`:228-250`) | `0o600` (`:378-382`) | No | **Error** |
| `SensitiveConfig` | **Yes** (`src/utils/atomic.rs:397-403`) | **Yes**, fail-closed (`:228-250`) | `0o600` (`:378-382`) | **Yes** (`:57`) | **Error** |
| `RecoverableMetadata` | No | Best-effort, warn (`:252-259`) | `0o600` (`:378-382`) | No | Warning only |
| `EphemeralCoordination` | No | **Skipped** — `None` (`:190-192`) | `0o600` (`:378-382`) | No | n/a |

> **Correction — permissions are `0o600` for every class.** The `Durability`
> doc comments say "default permissions" for `RecoverableMetadata` and
> `EphemeralCoordination` (`src/utils/atomic.rs:28-34`), but the
> implementation sets `mode(0o600)` on the temp file **unconditionally** on
> Unix (`src/utils/atomic.rs:378-382`), and the renamed target inherits that
> inode's mode. The comments are stale relative to the code. The two classes
> genuinely differ only in fsync behavior (and `SensitiveConfig` in symlink
> rejection) — **not** in file mode.
>
> The one way to get a non-`0o600` target is `preserve_permissions(true)`,
> which snapshots the pre-existing mode and restores it after the rename
> (`src/utils/atomic.rs:356-368`, `:411-426`). `for_durability` defaults it to
> `false` (`src/utils/atomic.rs:60`).

### What "fail-closed" means

`sync_parent_dir` (`src/utils/atomic.rs:223-261`) branches on the class:

```rust
if matches!(
    durability,
    Durability::DurableUserData | Durability::SensitiveConfig
) {
    match parent_dir_sync_durable(parent) {
        Ok(()) => return Ok(Some(true)),
        Err(e) if allow_dir_fsync_failure() => { /* warn, Ok(Some(false)) */ }
        Err(e) => return Err(SnipError::io_error("fsync parent directory after atomic write", parent, e)),
    }
}
```
— `src/utils/atomic.rs:228-250`

For the two durable classes a parent-directory fsync failure is an **error**:
the caller is told the write did not achieve its durability contract. The
rename has already happened at that point, so the file content is written —
the error means "this may not survive power loss", not "the write was
rolled back".

`parent_dir_sync_durable` is Unix-only (`src/utils/atomic.rs:167-182`); on
other platforms it returns
`Err(std::io::Error::other("dir fsync unsupported on this platform"))`.

### The test-only fsync seam

`allow_dir_fsync_failure` (`src/utils/atomic.rs:212-221`) is the only place
the fail-closed behavior can be relaxed:

| Build | Behavior |
|-------|----------|
| `test-support` feature | `true` when `SNP_ALLOW_DIR_FSYNC_FAILURE=1` |
| Production (no `test-support`) | always `false` — production always fails closed |

This seam exists because some filesystems (FUSE, certain container mounts)
reject directory fsync entirely. Like the other test-only seams it must
never be enabled in a production build.

---

## `AtomicWriteOptions`

```rust
pub struct AtomicWriteOptions {
    pub durability: Durability,
    pub preserve_permissions: bool,
    pub reject_symlink: bool,
}
```
— `src/utils/atomic.rs:40-51`

| Field | Default from `for_durability` | Effect |
|-------|-------------------------------|--------|
| `durability` | the class passed in | selects fsync and dir-sync behavior |
| `preserve_permissions` | `false` (`src/utils/atomic.rs:60`) | when `true` **and** the target existed, the original mode is restored after the rename |
| `reject_symlink` | `true` **only** for `SensitiveConfig` (`src/utils/atomic.rs:57`) | when `true`, a symlink at the target path is an error rather than a replacement |

Both setters are consuming builder methods (`src/utils/atomic.rs:66-75`).

---

## `AtomicWriteReport`

```rust
pub struct AtomicWriteReport {
    pub target_existed: bool,
    pub bytes_written: u64,
    pub parent_sync_supported: Option<bool>,
}
```
— `src/utils/atomic.rs:81-89`

| Field | Meaning |
|-------|---------|
| `target_existed` | whether the target path existed before the write (`src/utils/atomic.rs:352`) |
| `bytes_written` | length of the byte slice handed to `atomic_replace` (`src/utils/atomic.rs:433`) |
| `parent_sync_supported` | `Some(true)` = dir fsync succeeded; `Some(false)` = failed or unsupported; `None` = not probed (ephemeral class) (`src/utils/atomic.rs:88`, `:434`) |

> **Subtlety.** `target_existed` is computed with `target.exists()`
> (`src/utils/atomic.rs:352`), which **follows** symlinks, while validation
> uses `symlink_metadata` (`src/utils/atomic.rs:108`), which does **not**. For
> a *broken* symlink the directory entry exists but `exists()` returns
> `false`, so `target_existed` reports `false` even though the link was
> replaced.

---

## `write_private_atomic`

```rust
pub fn write_private_atomic(path: &Path, content: &str, temp_prefix: &str) -> SnipResult<()>
```
— `src/utils/atomic.rs:273`

The simple path: write a string to `path` atomically, with a private temp
file. It takes **no `Durability` argument** — its behavior is fixed.

### Algorithm

1. Resolve the parent directory and `create_dir_all` it
   (`src/utils/atomic.rs:274-278`).
2. Compute the temp path in the **same directory**:
   `{temp_prefix}.{uuid}.tmp` (`src/utils/atomic.rs:280`). Same-directory
   placement is what makes the later `rename` atomic.
3. Arm the [`TempFileGuard`](tempfile_guard.md) on that path
   (`src/utils/atomic.rs:281`).
4. Create the temp file with `create_new(true)`:
   - Unix: `mode(0o600)` (`src/utils/atomic.rs:283-287`).
   - Non-Unix: no creation-time permission knob in `std`, so the file
     inherits the parent directory's ACLs (`src/utils/atomic.rs:297-307`).
5. `write_all(content.as_bytes())` (`src/utils/atomic.rs:291`, `:308`).
6. `sync_all()` — **always**, on every platform
   (`src/utils/atomic.rs:293-294`, `:310-311`).
7. `fs::rename(&tmp_path, path)` (`src/utils/atomic.rs:314`).
8. `guard.persist()` — disarm the cleanup (`src/utils/atomic.rs:315`).
9. **Unix only**: `sync_parent_dir(parent, path, Durability::SensitiveConfig)?`
   (`src/utils/atomic.rs:321-322`).

### Properties

| Property | Value |
|----------|-------|
| File fsync | Always |
| Parent-dir fsync | Unix only, with `SensitiveConfig` semantics (fail-closed) |
| Target validation | **None** — `validate_target` is not called |
| Symlink handling | The rename **replaces** the symlink; it is never followed |
| Permission preservation | Not supported — the target ends up `0o600` on Unix |
| Report | None — the function returns `SnipResult<()>` |

Because step 9 is `#[cfg(unix)]`, **`write_private_atomic` performs no
parent-directory sync on Windows**. Windows installs rely on the
user-private `%APPDATA%` directory ACLs instead of a file-level DACL
(`src/utils/atomic.rs:268-272`).

---

## `atomic_replace`

```rust
pub fn atomic_replace(
    target: &Path,
    bytes: &[u8],
    options: &AtomicWriteOptions,
) -> SnipResult<AtomicWriteReport>
```
— `src/utils/atomic.rs:345`

The full-featured path. Ten steps, in order:

| # | Step | Source |
|---|------|--------|
| 1 | Resolve the parent directory, creating it if missing | `src/utils/atomic.rs:350` |
| 2 | Record whether the target existed | `src/utils/atomic.rs:352` |
| 3 | Validate the existing target (directories, FIFOs, sockets, devices; optionally symlinks) | `src/utils/atomic.rs:353` |
| 4 | Snapshot the original permissions when `preserve_permissions` is set and the target existed | `src/utils/atomic.rs:356-368` |
| 5 | Compute the temp path `{uuid}.tmp` in the same directory and arm the guard | `src/utils/atomic.rs:370-371` |
| 6 | Create the temp file with `create_new(true)`; on Unix `mode(0o600)` | `src/utils/atomic.rs:374-386` |
| 7 | `write_all(bytes)` then `flush()` | `src/utils/atomic.rs:388-393` |
| 8 | `sync_all()` — only for `DurableUserData` and `SensitiveConfig` | `src/utils/atomic.rs:397-403` |
| 9 | `fs::rename` over the target, then `guard.persist()` | `src/utils/atomic.rs:407-409` |
| 10 | Restore the original permissions if requested (best-effort; errors ignored) | `src/utils/atomic.rs:411-426` |

Then the parent directory is synced and the report is returned
(`src/utils/atomic.rs:429-435`).

Note the ordering: **the rename happens before the permission restore and
before the parent-dir sync.** A failure in either of those last two steps
returns an error while the new content is already in place.

---

## Permissions Handling

| Stage | Unix behavior | Source |
|-------|---------------|--------|
| Temp file creation | `mode(0o600)` in `atomic_replace`; `mode(0o600)` in `write_private_atomic` | `src/utils/atomic.rs:381`, `:287` |
| After rename | The target inherits the temp inode's mode → `0o600` | — |
| `preserve_permissions(true)` | The pre-existing mode is snapshotted **before** the write and re-applied **after** the rename | `src/utils/atomic.rs:356-368`, `:411-426` |
| Permission restore failure | Ignored — the write is not failed | `src/utils/atomic.rs:417-420` |
| Non-Unix | No creation-time knob; the file inherits the parent directory's ACLs | `src/utils/atomic.rs:268-272`, `:362-365`, `:422-425` |

The intent of `0o600` is that newly written config and library files never
briefly exist with broader default permissions
(`src/utils/atomic.rs:265-266`).

Two tests pin this behavior:

| Test | Assertion | Source |
|------|-----------|--------|
| `test_atomic_replace_durable_user_data_syncs` | a `DurableUserData` target is `0o600` — "User data must not be group/world-readable either" | `src/utils/atomic.rs:597-613` |
| `test_atomic_replace_preserves_permissions` | with `preserve_permissions = true` and a pre-existing `0o644` file, the result is `0o644` | `src/utils/atomic.rs:625-642` |

---

## Symlink Rejection Rule

`validate_target` (`src/utils/atomic.rs:107-162`) uses `symlink_metadata`, so
it inspects the **link itself**, never the link target.

```rust
if reject_symlink && meta.file_type().is_symlink() {
    return Err(SnipError::runtime_error(
        "target path is a symlink and reject_symlink is enabled",
        Some(&path.display().to_string()),
    ));
}

// The final rename replaces the destination directory entry itself. An
// allowed symlink must therefore be replaced, never followed; this also
// permits safely replacing broken links and links to special files.
if meta.file_type().is_symlink() {
    return Ok(());
}
```
— `src/utils/atomic.rs:114-126`

| Target at `path` | `reject_symlink = false` | `reject_symlink = true` |
|------------------|--------------------------|-------------------------|
| Missing | allowed | allowed |
| Regular file | allowed | allowed |
| **Symlink** | allowed — the link is **replaced**, never followed | **error** |
| Directory | error (`src/utils/atomic.rs:130-135`) | error |
| FIFO / socket / device node | error (Unix only, `src/utils/atomic.rs:137-159`) | error |

`reject_symlink` defaults to `true` **only** for `SensitiveConfig`
(`src/utils/atomic.rs:57`) — i.e. only API-key / credential style files are
protected by default.

---

## `hash_file` and `hex_encode`

```rust
pub fn hash_file(path: &Path) -> SnipResult<String>
```
— `src/utils/atomic.rs:470`

Computes the SHA-256 hex digest of a file by streaming through a `BufReader`
with an 8 KiB buffer (`src/utils/atomic.rs:475-486`), so arbitrarily large
files never require an in-memory copy. `hex_encode`
(`src/utils/atomic.rs:453-460`) renders the digest as lowercase hex with a
single pre-allocated `String`.

`crate::transaction::hash_file` delegates here
(`src/transaction.rs:2488-2490`) so the two cannot diverge. The digest is
computed **from the live file on disk**, not from a source buffer.

---

## Call Sites

### `write_private_atomic`

| Caller | Content | Temp prefix | Source |
|--------|---------|-------------|--------|
| `write_library_file` | library file content | caller's | `src/library/persistence.rs:16-20` |
| `save_library_internal` | `toml::to_string_pretty` of the snippet library | library file stem, else `"snippets"` | `src/library/persistence.rs:286-290` |
| `save_snippets` | `toml::to_string_pretty` of the config | `"config"` | `src/commands/mod.rs:228-231` |
| Transaction journals | journal TOML | `"txn"` | `src/transaction.rs:1405`, `:1415` |
| Sync settings | sync settings TOML | `"sync"` | `src/config/sync_settings.rs:488` |
| Import report | JSON report | `"import-report"` | `src/commands/import_cmd.rs:549` |
| Backup manifest | manifest TOML | `"manifest"` | `src/commands/backup_archive.rs:318` |
| Migration writes | minimal / updated config | `"migration"` | `src/migration.rs:160`, `:177` |
| Usage index | usage index | `"usage"` | `src/usage.rs:161` |

### `atomic_replace`

| Caller | Class | Source |
|--------|-------|--------|
| Transaction rollback — restore the original file bytes | `DurableUserData` | `src/transaction.rs:2250-2253` |
| `snp select --output` — write the selected command | `DurableUserData` | `src/commands/select_cmd.rs:111-115` |

### `atomic_write_bytes`

| Caller | Class | Source |
|--------|-------|--------|
| Theme config save | `SensitiveConfig` | `src/ui/theme.rs:563-567` |
| Auto-sync status file | `DurableUserData` | `src/auto_sync/status.rs:167-171` |
| Auto-sync pending marker | `DurableUserData` | `src/auto_sync/pending.rs:390-394` |

> **Gap worth noting.** `RecoverableMetadata` and `EphemeralCoordination` have
> **no production call sites** today — every caller uses one of the two
> durable classes. The classes exist for the durability taxonomy, not because
> the current writers need them.

---

## Key Files

| Path | Role |
|------|------|
| `src/utils/atomic.rs` | The write primitives, the durability classes, target validation, and file hashing |
| `src/utils/tempfile_guard.rs` | RAII cleanup for the temp file the write helpers create |
| `src/utils/mod.rs` | Declares `pub mod atomic` (`src/utils/mod.rs:11`) |
| `src/library/persistence.rs` | Library / config saves through `write_private_atomic` |
| `src/transaction.rs` | Journal writes and rollback restore; delegates `hash_file` here |
| `src/config/sync_settings.rs` | Sync settings writes |
| `architecture/persistence.md` | The mutation gate, the lock hierarchy, and the transaction state machine that surround these writes |
| `.skills/persistence-and-toml-fidelity.md` | The durability-class rules and the save-path fidelity contract |

---

## Invariants

1. **Every write goes through a same-directory temp file and a rename.**
   The temp file is never partially visible at the target path.
2. **The temp file is `0o600` on Unix for every durability class**
   (`src/utils/atomic.rs:381`, `:287`).
3. **`DurableUserData` and `SensitiveConfig` fail closed** when the parent
   directory cannot be fsynced (`src/utils/atomic.rs:242-248`).
4. **`EphemeralCoordination` never fsyncs** — neither the file nor the
   directory (`src/utils/atomic.rs:190-192`, `:397-403`).
5. **A symlink at the target is replaced, never followed**
   (`src/utils/atomic.rs:121-126`).
6. **`reject_symlink` defaults to `true` only for `SensitiveConfig`**
   (`src/utils/atomic.rs:57`).
7. **The temp file is cleaned up on every failure path** — see
   [tempfile_guard.md](tempfile_guard.md).
8. **`hash_file` hashes the live file**, not a source buffer
   (`src/utils/atomic.rs:462-488`).

---

## Gotchas

- **Do not rely on `Durability` to change the file mode.** It does not — the
  mode is always `0o600` on Unix (`src/utils/atomic.rs:378-382`). The
  `Durability` doc comments at `src/utils/atomic.rs:28-34` say otherwise and
  are stale.
- **Do not use `write_private_atomic` where target validation matters.** It
  never calls `validate_target`; use `atomic_replace` when the target must be
  checked for directories, devices, or symlinks.
- **Do not expect `write_private_atomic` to preserve permissions.** There is
  no `preserve_permissions` path; the target ends up `0o600` on Unix.
- **Do not expect a parent-directory sync from `write_private_atomic` on
  Windows.** The call is `#[cfg(unix)]` (`src/utils/atomic.rs:321-322`).
- **Do not treat an error from `atomic_replace` after the rename as a
  rollback.** The rename is step 9 of 10; the permission restore and the
  parent-dir sync come after it (`src/utils/atomic.rs:407-429`).
- **Do not enable `SNP_ALLOW_DIR_FSYNC_FAILURE` outside a
  `test-support` build.** The production implementation ignores it entirely
  (`src/utils/atomic.rs:217-221`).
- **Do not match on `Durability` without a wildcard arm.** The enum is
  `#[non_exhaustive]` (`src/utils/atomic.rs:17`).
- **Do not post-process the written TOML.** The save path writes the
  serializer output verbatim — see
  [.skills/persistence-and-toml-fidelity.md](../../.skills/persistence-and-toml-fidelity.md)
  and [persistence.md](../persistence.md).
