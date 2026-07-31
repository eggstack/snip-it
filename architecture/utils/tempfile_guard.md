# tempfile_guard.rs — RAII Temporary File Cleanup

## Overview

Provides an RAII guard that deletes a temporary file on drop unless `persist()` is called. Used by atomic-write functions to ensure orphaned temp files are cleaned up on failure.

**File**: `src/utils/tempfile_guard.rs`

## Type

```rust
pub struct TempFileGuard {
    path: PathBuf,
}
```

## API

### TempFileGuard::new(path)

Creates a new guard for the given temporary file path.

### TempFileGuard::persist(self)

Consumes the guard without deleting the file. Call after a successful `fs::rename` to prevent cleanup on drop.

### Drop impl

Deletes the file at `path` if `persist()` was not called. Ignores errors (file may already be removed).

## Usage Pattern

```rust
let guard = TempFileGuard::new(temp_path.clone());
// ... write to temp file ...
fs::rename(&temp_path, &target_path)?;
guard.persist(); // prevent cleanup
// if we reach here without persist(), drop() cleans up
```

## Integration

Used by `utils/atomic.rs` in `write_private_atomic()` and `atomic_replace()` to ensure temp files don't accumulate on write failures.
