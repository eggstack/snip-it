# Core Module Review

**Reviewed files**: `src/library.rs`, `src/error.rs`, `src/commands/mod.rs`, `src/utils/config.rs`, `src/utils/toml_helpers.rs`, `src/sync_commands.rs`, `src/config.rs`, `src/commands/library_cmd.rs`, `src/main.rs`
**Architecture doc**: `architecture/core.md`

---

## 1. Document Accuracy

### Verified Correct

- `Snippet` struct fields match the doc exactly: `id`, `description`, `command`, `output`, `tags`, `folders`, `favorite`, `created_at`, `updated_at`, `device_id`, `deleted` — all present, correct types (`src/library.rs:41-64`).
- `Snippets` struct: `snippets: Vec<Snippet>`, `folders: Vec<String>` — matches doc (`src/library.rs:27-33`).
- `Snippet::new()` constructor exists with described signature (`src/library.rs:124`).
- `LibraryManager` struct exists with described fields and operations (`src/library.rs:148-153`).
- Library validation: non-empty, max 50 chars, no slashes, no null bytes — matches doc (`src/library.rs:98-121`).
- `SnipError` enum variants: `Io`, `Toml`, `Clipboard`, `Command`, `Runtime` — matches doc (`src/error.rs:27-66`).
- All convenience constructors exist: `io_error`, `toml_error`, `clipboard_error`, `command_error`, `runtime_error` (`src/error.rs:134-172`).
- `From<io::Error>` conversion exists (`src/error.rs:122-130`).
- `SnipResult<T>` type alias exists (`src/error.rs:176`).
- `load_library()`, `save_library()`, `backup_library()` free functions exist (`src/library.rs:453-540`).
- `load_snippets()`, `save_snippets()` are thin wrappers in `commands/mod.rs` (`src/commands/mod.rs:96-164`).
- `LibraryConfig`, `LibraryMeta` structs match doc (`src/library.rs:69-85`).
- Library operations listed in doc all exist: `create_library`, `delete_library`, `set_primary`, `migrate_from_single_file`, `add_server_library`, `load_library`, `save_library`, `backup_library`.
- `SnippetData` struct in `main.rs` matches usage (`src/main.rs:27-33`).
- Merge strategy: last-write-wins based on `updated_at` — confirmed in code (`src/sync_commands.rs:410`).
- Sync preserves local-only fields (`output`, `folders`, `favorite`) when server wins — confirmed (`src/sync_commands.rs:411-422`).
- Deleted snippets excluded from merge — confirmed (`src/sync_commands.rs:385-407`).
- Snippets sorted by `updated_at` descending after merge — confirmed (`src/sync_commands.rs:450`).
- pet format compatibility works — confirmed by tests (`src/library.rs:548-568`).

### Discrepancies

1. **TOML field names in doc are wrong.** The architecture doc (`core.md:43-52`) shows:
   ```toml
   Description = "git commit"
   Command = "git commit -m \"<msg>\""
   Tag = ["git", "version-control"]
   ```
   But the actual serde serialization produces **lowercase** field names because no `rename` attribute is set on those fields. The actual TOML format is:
   ```toml
   description = "git commit"
   command = "git commit -m \"<msg>\""
   tags = ["git", "version-control"]
   ```
   Only `Id` and `Output` have `rename` attributes. The doc's TOML example also omits the `folders`, `device_id`, and `deleted` fields which are serialized.

2. **Backup extension inconsistency.** The doc doesn't mention backup naming, but the code uses three different extensions:
   - `load_library()` corrupt backup: `.toml.corrupt.bak` (`src/library.rs:474`)
   - `LibraryManager::new()` config corrupt backup: `.toml.corrupt` (`src/library.rs:177`)
   - `backup_library()` timestamped backup: `.toml.bak` (`src/library.rs:533`)
   - `commands/mod.rs` load_snippets corrupt backup: `.toml.bak` (`src/commands/mod.rs:124`)
   These should be consistent.

3. **Architecture doc claims `Snippet.id` is "generated on first sync"** but the code generates it lazily during `run_sync()` (`src/sync_commands.rs:266-272`), not during `Snippet::new()`. This is technically accurate but misleading — the doc implies it happens during sync, when it actually happens just before the push phase.

---

## 2. Bugs & Issues

### Critical

- **None found.** No logic errors that would cause data loss or corruption in normal usage.

### High

1. **`set_primary()` silently succeeds when filename doesn't exist** (`src/library.rs:346-352`).
   If you call `set_primary("nonexistent")`, all libraries have `is_primary` set to `false` and none is set to `true`. The config is saved in this state. The method returns `Ok(())` even though the requested library doesn't exist. This leaves the system in an invalid state with no primary library.

2. **`update_library_id()` silently does nothing when library not found** (`src/library.rs:354-360`).
   If the filename doesn't match any library, the config is still saved (via `save_config()`), producing a needless write. The caller has no way to know the operation didn't actually update anything.

3. **`update_last_sync()` silently does nothing when library not found** (`src/library.rs:379-385`).
   Same issue — config is saved unnecessarily, caller gets no feedback.

4. **`add_server_library()` always pushes a new entry even if filename already exists** (`src/library.rs:387-413`).
   The method creates a new `LibraryMeta` and pushes it unconditionally. If a library with the same filename already exists (e.g., imported on a previous run), you get a duplicate entry. The `create_library()` method correctly checks for existence, but `add_server_library()` does not.

### Medium

5. **`load_library()` error recovery is inconsistent with `commands/mod.rs` `load_snippets()`.**
   `load_library()` creates a backup with extension `.toml.corrupt.bak` (`src/library.rs:474`), while `load_snippets()` creates a backup with `.toml.bak` (`src/commands/mod.rs:124`). Both return `Snippets::default()` on parse failure — user data from the corrupted file is silently replaced with an empty collection unless the backup is manually inspected.

6. **`migrate_from_single_file()` copies but doesn't delete the legacy file** (`src/library.rs:241-265`).
   After copying `snippets.toml` to `libraries/snippets.toml`, the original file remains. On subsequent runs, `ensure_library_mode()` checks if `libraries_dir.exists()` — since the directory now exists, migration won't re-run. But the old file sits there unused, which could confuse users.

7. **No validation of `library_id` format in `update_library_id()`** (`src/library.rs:354-360`).
   The method accepts any string. While the caller should provide a UUID, no validation is performed. A malformed ID would silently be stored.

8. **`Snippet::new()` sets `created_at: 0` and `updated_at: 0`** (`src/library.rs:133-134`).
   These should arguably be set to `chrono::Utc::now().timestamp()` at creation time. Having timestamps of 0 means new snippets appear at the bottom of sorted lists and have `updated_at` that's never meaningful until sync runs.

### Low

9. **`test_library_manager_new` test is vacuous** (`src/library.rs:755-759`).
   ```rust
   assert!(mgr.is_ok() || mgr.is_err());
   ```
   This assertion is always true and tests nothing.

10. **`backup_library()` returns `Ok(None)` for nonexistent files** (`src/library.rs:509-512`).
    While this is a valid design choice, the doc doesn't document this behavior. Callers using the `?` operator won't get errors for missing files, which could mask issues.

---

## 3. Design Issues

### Tight Coupling

1. **`SnippetData` parallel arrays are fragile.** `get_snippet_data()` (`src/commands/mod.rs:169-194`) creates five parallel `Vec`s that must stay in sync. A struct-of-arrays approach would be safer, or better yet, a `Vec<SnippetView>` with references.

2. **`SnippetData` is defined in `main.rs`** (`src/main.rs:27-33`), not in the library module where `Snippet` lives. This couples the TUI data structure to the binary crate.

### Unclear Responsibilities

3. **`commands/mod.rs` re-implements load/save logic** (`src/commands/mod.rs:96-164`). The `load_snippets()` and `save_snippets()` functions duplicate error handling patterns from `library.rs`'s `load_library()` and `save_library()`. The doc acknowledges these are "thin wrappers" but they actually have different behavior (logging, different backup extension, return `Ok(default)` on IO read errors instead of propagating).

4. **`LibraryManager` mixes concerns.** It handles:
   - Config file I/O
   - Library CRUD operations
   - Premade library management
   - Single-file to library mode migration
   - macOS config directory migration (via `migrate_macos_config_dir()`)

   The premade operations could be a separate struct.

### Dead Code / Unused

5. **`folders` field on `Snippets` is rarely used.** The `Snippets.folders` field exists but the merge logic in `sync_commands.rs:454` just clones `local.folders`. The server `ProtoSnippet` doesn't carry folder data. This field appears to be vestigial from a planned feature.

---

## 4. Security Concerns

1. **No path traversal validation on `library_id`** (`src/library.rs:354`).
   While `library_id` is a UUID in practice, there's no validation that it can't contain path separators or other injection characters. Since it's only stored in TOML config (not used as a file path), the practical risk is low, but defense-in-depth is missing.

2. **`add_server_library()` doesn't validate `server_name`** (`src/library.rs:387-413`).
   The filename is derived from `server_name.to_lowercase().replace(' ', "-")`, but other characters (dots, special chars) pass through. This could create unexpected file paths. The `validate_library_name()` function is NOT called here, unlike in `create_library()`.

3. **Corrupted config backup doesn't use atomic write** (`src/library.rs:177-178`).
   If the process crashes between `fs::copy()` to backup and `LibraryConfig::default()` being used, the backup and runtime state could be inconsistent. This is a minor concern since config is re-read on next startup.

4. **`eprintln!` used for error reporting throughout** (multiple locations).
   Error messages printed to stderr could leak sensitive information (e.g., file paths containing usernames, API keys if they appear in error messages). The sync module appears to handle API keys carefully, but other error paths don't sanitize.

---

## 5. Performance Issues

1. **`get_snippet_data()` clones everything** (`src/commands/mod.rs:169-194`).
   All descriptions, commands, tags, and folders are cloned into new `Vec`s. For large snippet libraries, this creates unnecessary allocations. The TUI could reference the original `Snippets` data instead.

2. **`load_library()` always creates a backup of corrupted files** (`src/library.rs:474-485`).
   If a corrupted file is loaded on every startup (e.g., a persistent parse error), a new backup is created each time. No deduplication or cleanup.

3. **`merge_snippets()` uses `HashMap::collect()` from an iterator of references** (`src/sync_commands.rs:376-377`).
   This creates `HashMap<String, &Snippet>` which is fine, but the `local_by_id` map is rebuilt on every merge call. For single-library sync this is negligible.

4. **`backup_library()` creates timestamped backups without cleanup** (`src/library.rs:509-540`).
   Over time, the `backups/` directory can accumulate many files. No pruning mechanism exists.

---

## 6. Priority Ranking

| # | Severity | Finding | Location | Impact |
|---|----------|---------|----------|--------|
| 1 | High | `set_primary()` no-ops on missing filename, leaves no primary | `src/library.rs:346-352` | Config state corruption |
| 2 | High | `add_server_library()` creates duplicates | `src/library.rs:387-413` | Duplicate config entries |
| 3 | High | `update_library_id()` / `update_last_sync()` silent no-ops | `src/library.rs:354-360,379-385` | Lost data, misleading success |
| 4 | Medium | Inconsistent backup file extensions | Multiple files | User confusion |
| 5 | Medium | `Snippet::new()` uses zero timestamps | `src/library.rs:133-134` | Sort order / UX issues |
| 6 | Medium | `SnippetData` defined in `main.rs` not library crate | `src/main.rs:27-33` | Architectural coupling |
| 7 | Medium | `migrate_from_single_file()` doesn't clean up legacy file | `src/library.rs:241-265` | Confusion, wasted space |
| 8 | Low | TOML field names in doc are wrong | `architecture/core.md:43-52` | Doc inaccuracy |
| 9 | Low | Vacuous test `test_library_manager_new` | `src/library.rs:755-759` | No coverage |
| 10 | Low | No backup pruning mechanism | `src/library.rs:509-540` | Disk usage over time |

---

## 7. Recommendations

1. **Fix `set_primary()`** to return an error if the filename isn't found in the library list, rather than silently invalidating the config.

2. **Add existence check to `add_server_library()`** before pushing a new `LibraryMeta`. If a library with the same filename already exists, update its `library_id` instead of creating a duplicate.

3. **Make `update_library_id()` and `update_last_sync()` return errors on missing filename** instead of silently saving unchanged config.

4. **Standardize backup extensions** across the codebase. Pick one convention (e.g., `.toml.corrupt.bak` for parse failures, `.toml.bak` for pre-save backups) and use it consistently.

5. **Set timestamps in `Snippet::new()`** to `Utc::now().timestamp()` so new snippets have meaningful creation times.

6. **Move `SnippetData` to `src/library.rs`** or create a new module for TUI data structures to avoid coupling the library crate to the binary.

7. **Clean up legacy file after migration** in `migrate_from_single_file()` to avoid leaving orphaned files.

8. **Replace vacuous test** with meaningful assertions or remove it.

9. **Update `architecture/core.md`** to reflect actual TOML serialization field names.

10. **Consider adding backup pruning** to `backup_library()` — e.g., keep only the last N backups per library.
