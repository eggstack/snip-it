# tempfile_guard.rs — RAII Temporary File Cleanup

[← Back to Overview](../overview.md)

## Overview

Provides an RAII guard that deletes a temporary file on drop unless `persist()` is called. Used by atomic-write functions to ensure orphaned temp files are cleaned up on failure.

**File**: `src/utils/tempfile_guard.rs` (38 lines, no dedicated tests — covered via atomic-write tests)

## Type

```rust
pub struct TempFileGuard {
    path: PathBuf,
}
```

A single-path struct with no `Option`: the armed/disarmed state is encoded by clearing the path.

## API

### TempFileGuard::new(path)

```rust
pub fn new(path: PathBuf) -> Self
```

Creates a new guard for the given temporary file path. The guard is armed immediately — if dropped without `persist()`, the file is removed.

### TempFileGuard::persist(self)

```rust
pub fn persist(mut self)
```

Consumes the guard without deleting the file by resetting the path to empty. Call after a successful `fs::rename` to prevent cleanup on drop. Takes `self` by value, so forgetting the call is impossible to do silently — the drop path runs by default.

### Drop impl

```rust
impl Drop for TempFileGuard {
    fn drop(&mut self) { /* remove_file unless disarmed */ }
}
```

- Empty path (post-`persist`) → no-op.
- `NotFound` → ignored (file already renamed or never created).
- Other errors → `tracing::warn!`, never panics (a cleanup failure must not mask the original write error or crash drop).

## Usage Pattern

```rust
let guard = TempFileGuard::new(temp_path.clone());
// ... write to temp file ...
fs::rename(&temp_path, &target_path)?;
guard.persist(); // prevent cleanup
// if we reach here without persist(), drop() cleans up
```

The guard must be created *before* the temp file is opened so every early `return Err(...)` between creation and rename still triggers cleanup.

## Integration

Used by `utils/atomic.rs` in `write_private_atomic()` and `atomic_replace()` to ensure temp files don't accumulate on write failures. `tests/persistence_unit.rs` and the in-module atomic tests assert no `.tmp` files remain after both success and failure.

## Invariants

- Default is cleanup: every exit path deletes unless explicitly disarmed.
- `Drop` never panics and never returns errors — cleanup is best-effort by design.
- After `rename`, the temp path no longer exists, so `persist()` is strictly a disarm; there is nothing left to delete either way.
