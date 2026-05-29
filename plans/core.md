# Core Architecture Review - Improvement Plan

## Claim Verification

### 1. Snippet struct
**Doc claim**: `id`, `description`, `command`, `output`, `tags`, `folders`, `favorite`, `created_at`, `updated_at`, `device_id`, `deleted` — all match actual struct at `src/library.rs:41-64` ✓

**Bug**: Field order in doc is `id, description, command, output, tags, folders, favorite, created_at, updated_at, device_id, deleted` but actual is `id, description, output, tags, command, folders, favorite, created_at, updated_at, device_id, deleted`. Minor documentation inaccuracy.

### 2. Snippets struct
**Doc claim**: `snippets: Vec<Snippet>`, `folders: Vec<String>` — matches `src/library.rs:28-33` ✓

### 3. TOML Format
**Doc claim**: Uses `[[Snippets]]` with `Id`, `Description`, `Command`, `Tag`, `Output`, `favorite` — matches actual aliases at `src/library.rs:42-50` ✓

**Alias verification**:
- `Id` (alias `ID`) ✓
- `Description` ✓
- `Command` ✓
- `Tag` (alias `Tags`) ✓
- `Output` (alias `output`) ✓
- Pet format compatibility documented and tested ✓

### 4. LibraryManager Modes
**Doc claim**: Single-file mode (legacy, `~/.config/snp/snippets.toml`) and library mode (default, `~/.config/snp/libraries/*.toml`) — **PARTIALLY INACCURATE**

**Actual behavior** (`src/library.rs:217-219`):
```rust
pub fn is_single_file_mode(&self) -> bool {
    !self.libraries_dir.exists()
}
```
The code checks if `libraries/` directory exists, NOT whether libraries are configured. If the legacy `snippets.toml` exists but `libraries/` doesn't, it's single-file mode. But `LibraryManager::new()` never auto-migrates — migration only happens when `ensure_library_mode()` is called. This means the system can be in an ambiguous state where single-file mode is detected but migration hasn't occurred.

### 5. Library Configuration
**Doc claim**: `libraries.toml` with `filename`, `library_id`, `is_primary`, `last_sync` — matches `src/library.rs:75-85` ✓

### 6. LibraryManager Operations
| Operation | Doc | Code | Status |
|-----------|-----|------|--------|
| `create_library(name)` | Yes | Yes (line 289) | ✓ |
| `delete_library(name)` | Yes | Yes (line 323) | ✓ |
| `set_primary(name)` | Yes | Yes (line 346) | ✓ |
| `migrate_from_single_file()` | Yes | Yes (line 241) | ✓ |
| `add_server_library(name, id)` | Yes | Yes (line 398) | ✓ |
| `load_library(path)` | Yes | Yes (line 471) | ✓ |
| `save_library(path, snippets)` | Yes | Yes (line 511) | ✓ |
| `backup_library(path)` | Yes | Yes (line 527) | ✓ |

All documented operations exist.

### 7. Validation
**Doc claim**: Non-empty, max 50 chars, no slashes, no null bytes — matches `src/library.rs:98-121` ✓

### 8. SnipError enum
**Doc claim**: `Io`, `Toml`, `Clipboard`, `Command`, `Runtime` variants — matches `src/error.rs:27-66` ✓

**Discrepancy**: Doc shows `source` as direct `io::Error` but actual has `source: Box<dyn std::error::Error + Send + Sync>` for Toml variant (line 42). Doc is incomplete.

### 9. Convenience Constructors
**Doc claim**: All 5 constructors documented — matches `src/error.rs:134-172` ✓

### 10. Conversions
**Doc claim**: `From<io::Error>`, `SnipResult<T>` — matches `src/error.rs:122-130, 176` ✓

### 11. Key Files
**Doc claim**: `src/library.rs`, `src/error.rs`, `src/commands/mod.rs` — matches actual ✓

**Missing from doc**: `src/utils/config.rs` is a key file referenced by library.rs but not listed.

---

## Bugs & Edge Cases

### Bug 1: Silent Migration on Library Access
**Severity**: Medium
**Location**: `src/commands/mod.rs:59-63`
```rust
let mut mgr = LibraryManager::new()?;
if let Err(e) = mgr.ensure_library_mode() {
    eprintln!("Warning: Failed to ensure library mode: {}", e);
}
```
The `get_library_path()` and `init_library_manager()` functions silently migrate from single-file to library mode when a user just wants to list libraries or view a snippet. There's no confirmation prompt, and the migration is one-way. User could lose track of where their data migrated to.

**Fix**: Add user confirmation or `--force` flag for automatic migration.

### Bug 2: Primary Library Selection Logic
**Severity**: Low
**Location**: `src/library.rs:338-339`
```rust
if was_primary && !self.config.libraries.is_empty() {
    self.config.libraries[0].is_primary = true;
}
```
When deleting the primary library, the code just promotes the first remaining library without considering if that library was synced from a server vs local-only.

### Bug 3: `save_config` Error Handling
**Severity**: Medium
**Location**: `src/library.rs:456-468`
The `save_config()` method silently fails on disk full or permission errors because it's called from multiple places but errors aren't propagated consistently. For instance, `update_library_id()` and `update_last_sync()` both swallow errors:
```rust
pub fn update_library_id(&mut self, filename: &str, library_id: &str) -> SnipResult<()> {
    if let Some(lib) = self.get_library_by_filename_mut(filename) {
        lib.library_id = library_id.to_string();
        self.save_config()?;  // Error propagates
    }
    Ok(())  // But this always succeeds even if save_config failed
}
```

### Bug 4: Empty Snippet Commands Not Validated
**Severity**: Medium
**Location**: `src/library.rs`
`Snippet::new()` and `create_library()` don't validate that command is non-empty. Empty commands could be saved and cause issues downstream (e.g., in `run_cmd.rs` execution).

### Bug 5: Library Name Validation Missing Path Traversal Check
**Severity**: Low (mitigated by other factors)
**Location**: `src/library.rs:98-121`
The `validate_library_name()` checks for `/`, `\`, and null bytes but doesn't explicitly check for `..` or other path traversal patterns. While `create_library()` correctly creates files in `libraries_dir` (not user-controlled path), the validation could be more explicit.

---

## Potential Improvements

### Improvement 1: Atomic Config Saves
**Priority**: High
The `save_config()` writes directly to `libraries.toml` without using temp files or atomic rename. On crash/power failure during write, config could be corrupted.

### Improvement 2: Snippet ID Uniqueness Not Enforced
**Priority**: Medium
Multiple snippets can have the same `id` since there's no uniqueness check in `load_library()` or `save_library()`. This could cause sync issues.

### Improvement 3: No Limits on Snippet Count or Size
**Priority**: Low
No pagination or lazy loading for large snippet collections. All snippets are loaded into memory.

### Improvement 4: Missing `deleted` Flag Handling in TUI
**Priority**: Medium
The `deleted` flag is stored but `select_snippet_inner()` and `get_snippet_data()` don't filter out deleted snippets. Users may see soft-deleted snippets in the TUI.

### Improvement 5: No Encryption Key Validation on Load
**Priority**: Medium (sync-related but impacts core)
`load_library()` doesn't validate that the file structure is intact after encryption. If decrypt fails partially, there's no recovery.

### Improvement 6: LibraryManager Should Track Unsaved Changes
**Priority**: Low
If `save_config()` fails, the in-memory state is out of sync. No rollback mechanism.

### Improvement 7: `get_library_path` Discards LibraryManager Errors
**Severity**: Medium
**Location**: `src/commands/mod.rs:56-84`
```rust
if let Err(e) = mgr.ensure_library_mode() {
    eprintln!("Warning: Failed to ensure library mode: {}", e);
}
```
If migration fails, the code continues anyway and may return a path to the wrong file.

---

## Discrepancies Summary

| Item | Doc Says | Actual |
|------|----------|--------|
| Snippet field order | `id, description, command...` | `id, description, output, tags, command...` |
| Toml source type | `source` (implied io::Error) | `Box<dyn std::error::Error + Send + Sync>` |
| Key files | 3 files | 4 files (missing `src/utils/config.rs`) |
| Single-file detection | "Legacy" mode | Only when `libraries/` dir doesn't exist |
| `add_server_library` | Listed in operations | Correctly present at line 398 |

---

## Security Concerns

1. **No input sanitization on snippet `command` field**: Commands are executed via shell without sanitization beyond variable expansion. See `src/commands/run_cmd.rs`.

2. **Permission issue**: `libraries.toml` and snippet files are created with default umask. Sensitive data (API keys in sync config) could be readable.

---

## Performance Considerations

1. **TOML parsing on every command**: `load_snippets()` parses entire TOML file even for `snp list` with just 1 snippet visible. No caching.

2. **No lazy loading**: All snippets + their data (even large command strings) are loaded into memory.

3. **Repeated directory scanning**: `LibraryManager` creates new instance each call, re-reading config file.

---

## Recommendations (Priority Order)

1. **High**: Add atomic writes for `libraries.toml` using temp file + rename
2. **High**: Filter `deleted: true` snippets from TUI display
3. **Medium**: Validate non-empty `command` in `Snippet::new()`
4. **Medium**: Add confirmation prompt for single-file migration
5. **Medium**: Add `id` uniqueness enforcement
6. **Low**: Document `src/utils/config.rs` as a key file
7. **Low**: Add explicit path traversal check to library name validation
8. **Low**: Consider caching Layer for snippet loading