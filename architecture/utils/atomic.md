# atomic.rs — Atomic File Writes

## Overview

Provides atomic file-write helpers with durability-aware persistence, permission control, and fsync guarantees.

**File**: `src/utils/atomic.rs`

## Durability Classes

```rust
pub enum Durability {
    DurableUserData,      // sync_all before rename (snippets, libraries)
    SensitiveConfig,      // 0o600 permissions, reject symlinks (API keys)
    RecoverableMetadata,  // no fsync, default permissions (usage counters)
    EphemeralCoordination,// no fsync, default permissions (lock files)
}
```

## Key Functions

### write_private_atomic()

```rust
pub fn write_private_atomic(path: &Path, content: &[u8]) -> SnipResult<()>
```

Simple atomic write: writes to temp file, renames to target. Used for most TOML persistence.

### atomic_replace()

```rust
pub fn atomic_replace(
    path: &Path,
    content: &[u8],
    options: &AtomicWriteOptions,
) -> SnipResult<AtomicWriteReport>
```

Enhanced atomic write with:
- `Durability`-based fsync behavior
- Optional permission preservation
- Symlink rejection for sensitive files
- Allowed destination symlinks are replaced as directory entries rather than
  dereferenced; broken links are safe to replace
- Parent directory fsync probing
- `AtomicWriteReport` with metadata

## AtomicWriteOptions

```rust
pub struct AtomicWriteOptions {
    pub durability: Durability,
    pub preserve_permissions: bool,
    pub reject_symlink: bool,
}
```

Builder pattern: `AtomicWriteOptions::for_durability(d).preserve_permissions(true)`

## AtomicWriteReport

```rust
pub struct AtomicWriteReport {
    pub target_existed: bool,
    pub bytes_written: u64,
    pub parent_sync_supported: Option<bool>,
}
```

## Integration

- `library.rs` — saves snippets/libraries via `write_private_atomic()`
- `usage.rs` — saves usage counters via `write_private_atomic()` with `RecoverableMetadata`
- `config.rs` — saves sync settings via `write_private_atomic()`
- `transaction.rs` — saves journals with `DurableUserData`
- `restore_cmd.rs` — restores files via `atomic_replace()` with permission control
