# atomic.rs — Atomic File Writes

[← Back to Overview](../overview.md)

## Overview

Provides atomic file-write helpers with durability-aware persistence, permission control, and fsync guarantees.

**File**: `src/utils/atomic.rs` (699 lines, including unit tests)

## Durability Classes

```rust
pub enum Durability {
    DurableUserData,      // sync_all before rename (snippets, libraries)
    SensitiveConfig,      // 0o600 permissions, reject symlinks (API keys)
    RecoverableMetadata,  // no fsync, default permissions (usage counters)
    EphemeralCoordination,// no fsync, default permissions (lock files)
}
```

| Class | File fsync | Dir fsync | Permissions | Symlink reject |
|-------|-----------|-----------|-------------|----------------|
| `DurableUserData` | Yes | Yes (fail-closed) | Default | No |
| `SensitiveConfig` | Yes | Yes (fail-closed) | `0o600` | Yes |
| `RecoverableMetadata` | No | Best-effort (warn) | Default | No |
| `EphemeralCoordination` | No | Skipped (`None`) | Default | No |

`DurableUserData` and `SensitiveConfig` fail closed on parent-dir fsync errors. A `test-support`-gated `SNP_ALLOW_DIR_FSYNC_FAILURE=1` seam downgrades that to a warning for FUSE/container mounts; production builds never honor it.

## Key Functions

### write_private_atomic()

```rust
pub fn write_private_atomic(path: &Path, content: &str, temp_prefix: &str) -> SnipResult<()>
```

Simple atomic write: UUID-named temp file in the same directory, `0o600` on Unix, write + `sync_all`, rename, parent-dir sync. Used for most TOML persistence (snippets, libraries, usage, config, transaction journals).

### atomic_replace()

```rust
pub fn atomic_replace(
    target: &Path,
    bytes: &[u8],
    options: &AtomicWriteOptions,
) -> SnipResult<AtomicWriteReport>
```

Enhanced atomic write: `Durability`-based fsync, optional permission preservation, symlink rejection for sensitive files (allowed symlinks are replaced as directory entries, never dereferenced), parent-dir fsync probing, `AtomicWriteReport` result.

### atomic_write_bytes() / hash_file()

```rust
pub fn atomic_write_bytes(target: &Path, bytes: &[u8], durability: Durability) -> SnipResult<()>
pub fn hash_file(path: &Path) -> SnipResult<String>
```

Thin `atomic_replace` wrapper for byte-holding callers; canonical streaming SHA-256 hasher (`transaction::hash_file` delegates here) that verifies installed destinations from the live file.

## AtomicWriteOptions

```rust
pub struct AtomicWriteOptions {
    pub durability: Durability,
    pub preserve_permissions: bool,
    pub reject_symlink: bool,
}
```

Builder pattern: `AtomicWriteOptions::for_durability(d).preserve_permissions(true)`. `for_durability` sets `reject_symlink = true` only for `SensitiveConfig`.

## AtomicWriteReport

```rust
pub struct AtomicWriteReport {
    pub target_existed: bool,
    pub bytes_written: u64,
    pub parent_sync_supported: Option<bool>,
}
```

`parent_sync_supported` is `None` for `EphemeralCoordination` (never probed), `Some(true/false)` otherwise.

## Algorithm

`atomic_replace` executes 11 steps:

1. Resolve parent directory (create if missing via `create_dir_all`)
2. Record whether the target exists
3. Validate target — reject directories, FIFOs, sockets, block/character devices, and optionally symlinks
4. Snapshot original permissions if `preserve_permissions` is set
5. Create UUID-named temp file in the same directory (`create_new`, `0o600` on Unix)
6. Write bytes, flush to kernel buffer
7. For `DurableUserData`/`SensitiveConfig`, call `sync_all` on the file
8. Atomic `rename` over the target
9. `persist()` the `TempFileGuard` (disarm cleanup)
10. Restore original permissions if `preserve_permissions` was set (best-effort)
11. Sync parent directory; on any failure before step 8, `TempFileGuard` cleans up

## Invariants

- Temp file and target always share a directory, so `rename` is atomic on one filesystem.
- `TempFileGuard` guarantees no `.tmp` orphans on success or failure (asserted by tests).
- Allowed symlinks are replaced, never followed — the rename swaps the directory entry itself.
- Permission snapshot happens before the write; restore is best-effort and never fails the write.

## Integration

- `library.rs` / `usage.rs` / `config.rs` — snippet, usage, sync-setting saves via `write_private_atomic()`
- `transaction.rs` — journals via `write_private_atomic()` (with `DurableUserData` parent-dir sync)
- `restore_cmd.rs` — restores via `atomic_replace()` with permission control

## Tests

In-module unit tests plus `tests/persistence_unit.rs`: durability classes, permission preservation, symlink rejection/allowed-replacement, temp-file cleanup, builder chaining.
