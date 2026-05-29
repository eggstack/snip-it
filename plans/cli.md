# CLI Architecture Review - Improvement Plan

## Architecture Document Verification

### Document Claims vs Actual Implementation

| Claim | Status | Notes |
|-------|--------|-------|
| `src/main.rs` is 331 lines | **INACCURATE** | Actual: 317 lines |
| `src/commands/mod.rs` is 271 lines | **INACCURATE** | Actual: 265 lines |
| Panic handler installed on startup | ✓ Verified | `setup_panic_handler()` at line 307 |
| Signal handlers registered | ✓ Verified | `setup_signal_handler()` at line 308, SIGINT/SIGTERM on Unix |
| Default tracing logging initialized | ✓ Verified | `init_default_logging()` at line 309 |
| CLI args parsed, command dispatched via `dispatch_command()` | ✓ Verified | Line 312-313 |
| `CONFIG_PATH: LazyLock<PathBuf>` | ✓ Verified | Line 41 |
| `RUNTIME: LazyLock<Runtime>` | ✓ Verified | Lines 43-44 |
| Subcommands map 1:1 to modules | ✓ Verified | All 13 command modules present |
| `premade_cmd` and `library_cmd` use subcommand-dispatched functions | ✓ Verified | `run_list`, `run_get`, etc. |

---

## Bugs & Edge Cases

### 1. **TUI Exit Behavior Inconsistent with Documentation**

**Location**: `src/commands/mod.rs:252-260`

**Issue**: The documentation states TUI commands optionally trigger sync on exit. However, the actual behavior:
- `run_snippet_selection` ALWAYS calls `run_default_sync(runtime)` when `do_sync` is true, regardless of HOW the user exits (Cancel, Done, or even early break).
- `clip` command passes `sync: true` but never checks sync result - errors are silently ignored.

**Code**:
```rust
// mod.rs:261-263
if do_sync {
    crate::sync_commands::run_default_sync(runtime);
}
```

**Impact**: Unintended sync operations may trigger on every exit path.

---

### 2. **Race Condition in `run_cmd::process_snippet`**

**Location**: `src/commands/run_cmd.rs:97-100`

**Issue**: Between `validate_output_path()` and `fs::File::create()`, the path could be modified (symlink attack, path traversal via symlink).

**Code**:
```rust
validate_output_path(&snippet.output)?;
let output_file = fs::File::create(&snippet.output)  // TOCTOU race here
```

**Impact**: Security vulnerability - an attacker could replace a benign path with a symlink to a sensitive file after validation but before file creation.

---

### 3. **Unchecked Error in `edit_cmd::run`**

**Location**: `src/commands/edit_cmd.rs:26`

**Issue**: `std::env::var("EDITOR")` returns `Err` if the variable is not set, but the code uses `unwrap_or_else` to default to "vim". However, if `"vim"` doesn't exist in PATH, no validation occurs until `resolve_editor()` is called. This is actually correct, but the fallback chain is confusing.

The actual bug: `resolve_editor` is called AFTER `Command::new` uses `resolved_editor`. The editor path resolution error is properly handled, but the error message could be clearer.

---

### 4. **Missing `_config` Parameter Handling in `run_cmd`**

**Location**: `src/commands/run_cmd.rs:141`

**Issue**: The `_config` parameter is accepted but never used. Commands that use `run_snippet_selection` (run, clip, search) cannot override the config path - they always use the library path. This is documented but may surprise users.

**Code**:
```rust
pub fn run(
    filter: Option<String>,
    do_sync: bool,
    library: Option<String>,
    _config: Option<PathBuf>,  // UNUSED
    runtime: &tokio::runtime::Runtime,
) -> SnipResult<()>
```

---

### 5. **Multiline Input Reads Until Double-Empty-Line (Not EOF)**

**Location**: `src/commands/new_cmd.rs:8-25`

**Issue**: `read_multiline_command()` reads until TWO consecutive empty lines. If a user wants to input a multiline command with an empty line in the middle, they cannot. The empty line is indistinguishable from the terminator.

**Code**:
```rust
if line.trim().is_empty() && prev_was_empty {
    break;
}
```

**Impact**: Cannot create snippets with internal empty lines.

---

### 6. **`clip_cmd` and `search_cmd` Ignore `sync` Result**

**Location**: `src/commands/clip_cmd.rs`, `src/commands/search_cmd.rs`

**Issue**: Both commands call `run_snippet_selection` with `do_sync` but don't check if sync succeeded or failed. Errors are silently discarded.

---

### 7. **`sync_cmd::run` Silently Ignores Sync Failures**

**Location**: `src/commands/sync_cmd.rs:185-192`

**Issue**: `run_sync()` result is not checked or propagated.

**Code**:
```rust
crate::sync_commands::run_sync(
    &sync_settings,
    library.as_deref(),
    non_interactive,
    push_only,
    pull_only,
    runtime,
);
// No Result checking!
```

---

### 8. **Inconsistent Error Handling in `register_cmd`**

**Location**: `src/commands/register_cmd.rs:51-54`

**Issue**: If `save_sync_settings` fails, the function prints an error and returns `Err(e)`, but the registration was already successful on the server. The user has a valid API key/device_id that won't be saved.

**Code**:
```rust
if let Err(e) = save_sync_settings(&sync_settings) {
    eprintln!("Failed to save sync settings: {}", e);
    return Err(e);
}
```

---

## Security Concerns

### 1. **Output Path Traversal (Partial)**

**Location**: `src/commands/run_cmd.rs:10-46`

The `validate_output_path` function checks for `..` components and rejects absolute paths. However, it only checks the string representation. On Unix, a path like `foo/../bar` passes validation but could still be exploited if `foo` is a symlink.

**Recommendation**: Use `std::fs::canonicalize` to resolve the final path and verify it stays within an allowed directory.

---

### 2. **Editor Path Resolution Could Execute Arbitrary Code**

**Location**: `src/commands/edit_cmd.rs:39-130`

If a user sets `EDITOR` to a relative path with directory components (e.g., `./malicious`), the code resolves it relative to CWD. An attacker could potentially place a malicious binary in a directory and wait for the user to edit from that location.

**Current behavior is documented but risky for untrusted environments.**

---

### 3. **API Key in Memory**

**Location**: `src/commands/register_cmd.rs:57-62`

The API key is printed to stdout (masked) but stored in `SyncSettings` in memory. No attempt to clear it from memory after use.

---

## Performance Considerations

### 1. **Runtime Created on Every Async Command**

**Location**: `src/main.rs:43-44`

The Tokio runtime is created lazily but persists for the entire process lifetime once created. For short-lived commands, this is overhead.

**Not a bug**, but could be optimized by dropping the runtime after sync operations complete.

---

### 2. **Multiple Library Manager Instances**

**Location**: Throughout command modules

`init_library_manager()` and `get_library_path()` each create new `LibraryManager` instances. Each creation involves file system reads.

**Code duplication in**: `run_cmd`, `clip_cmd`, `search_cmd`, `list_cmd`, `edit_cmd`, `new_cmd`

---

## Missing Documentation / Discrepancies

### 1. **No Mention of `Cron` Command in Main Dispatch**

The architecture doc lists `cron` command but the main CLI definition shows it correctly. However, the description "Generate crontab entry for auto-sync" could clarify it doesn't execute sync, just generates the entry.

---

### 2. **`run_snippet_selection` Doesn't Support Config Override**

Unlike `list_cmd` and `new_cmd`, the TUI commands (run, clip, search) don't accept a `--config` argument. This is by design but undocumented.

---

### 3. **No Documentation of `expand_snippet_command` Failure Modes**

The function returns `ExpandedCommand::Cancel` or `Skip` on user interaction but propagates errors. The difference between user cancellation and error is blurred.

---

## Potential Improvements

1. **Add `--config` support to run/clip/search commands** for consistency
2. **Use `canonicalize` for output path validation** to prevent symlink attacks
3. **Check sync result in clip/search commands** and surface errors to user
4. **Allow custom multiline terminator** or EOF-based input
5. **Add `--dry-run` flag to sync command**
6. **Clear sensitive data from memory** after registration/sync
7. **Reuse LibraryManager instance** within a single command invocation
8. **Add `--json` output option** to `list` command for scripting
9. **Validate EDITOR exists before attempting edit** with better error message
10. **Add timeout to editor launch** in case editor hangs

---

## Verified Correct Behavior

- Signal handling correctly differs between Unix and Windows
- Clipboard copy uses platform-appropriate backend (copypasta/clipboard-win)
- TOML escaping/unescaping for backslashes works correctly
- Fuzzy matching in list command combines description + command
- Library mode transitions handled gracefully with warnings
- Audit logging failures are non-fatal (silently ignored)