# Library Module Code Review & Improvement Plan

## Verification of Architecture Claims

### ✅ Claim 1: Snippet data structure
**Doc claims:**
```rust
pub struct Snippet {
    pub id: Uuid,
    pub name: String,
    pub command: String,
    pub output: Option<String>,
    pub tags: Vec<String>,
    pub folders: Vec<String>,
    pub favorite: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted: bool,
}
```

**Actual (`library.rs:41-64`):**
```rust
pub struct Snippet {
    pub id: String,
    pub description: String,
    pub output: String,
    pub tags: Vec<String>,
    pub command: String,
    pub folders: Vec<String>,
    pub favorite: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub device_id: String,
    pub deleted: bool,
}
```

**Discrepancy:** Multiple fields differ from documentation:
- `name` vs `description` — doc says `name`, code has `description`
- `id` is `String` not `Uuid` — no Uuid type used
- `output` is `String` not `Option<String>`
- `created_at`/`updated_at` are `i64` not `DateTime<Utc>`
- `device_id` is present in code but not documented

### ✅ Claim 2: LibraryManager provides load_library(), save_library(), backup_library()
**Actual:** All three functions exist as standalone functions (lines 471-558), not methods on `LibraryManager`. They are module-level functions.

### ✅ Claim 3: TOML Handling — snippets sorted by updated_at descending on save
**Actual:** `save_library()` (lines 511-525) does NOT sort snippets before saving. No sorting is performed.

### ✅ Claim 4: Backup before save
**Actual:** `LibraryManager` has no `backup_library()` call in `save_library()`. The `backup_library()` function exists but is not called automatically during save. Callers must manually invoke it.

### ✅ Claim 5: Migration from single-file to multi-library
**Actual:** `migrate_from_single_file()` exists (lines 241-265). Only copies file, does not parse and restructure data.

### ✅ Claim 6: Soft delete with `deleted: true`
**Actual:** Field exists. However, no code in `library.rs` enforces exclusion of deleted snippets from UI queries — that responsibility appears to be in sync or commands modules.

### ✅ Claim 7: Error Handling — SnipError::LibraryNotFound
**Actual:** `LibraryNotFound` variant does NOT exist in `SnipError`. Code uses `SnipError::runtime_error("Library not found", ...)` instead.

---

## Bugs & Edge Cases

### Bug 1: No sorting on save
`save_library()` does not sort snippets by `updated_at` descending as documented. Snippets are saved in whatever order they happen to be in the `Vec`.

### Bug 2: backup_library() not called automatically
Despite being listed as a key behavior, `backup_library()` is never invoked by `save_library()`. Users or commands must manually call it before saves.

### Bug 3: TOML regex edge case with escaped quotes
The `TOML_STRING_PATTERN` regex `"([^"\\]*(?:\\.[^"\\]*)*)"` can misalign when strings contain escaped quotes (`\"`). The `fix_toml_strings` logic handles single quotes inside strings by escaping backslashes (line 31), but the regex match boundaries may be incorrect if a string contains `\"` that isn't a TOML escape sequence but a literal backslash followed by quote.

### Bug 4: create_library() and add_server_library() allow duplicate filenames with different casing
On case-insensitive filesystems (macOS default, Windows), `create_library("MyLib")` then `create_library("mylib")` will overwrite or conflict. The validation only checks exact string match.

### Bug 5: No validation of library_id format
`update_library_id()` and `add_server_library()` accept any string as `library_id`. No UUID validation.

### Bug 6: No confirmation before delete_library()
`delete_library()` immediately removes the file without confirmation prompt. No trash/recycle bin mechanism.

### Bug 7: Snippet fields renamed on deserialization (case sensitivity)
The `#[serde(rename = ...)]` attributes mean both `Description` and `description` (and `Description` vs `Command`) are accepted. But there's no `name` alias for `description`, so legacy data with `name` field would fail to deserialize properly.

### Bug 8: Empty folders array serializes inconsistently
Empty `Vec<String>` for `folders` may serialize as `folders = []` or be omitted depending on serde defaults. This could cause diff noise in backups.

---

## Potential Improvements

### 1. Add sorting before save
Sort snippets by `updated_at` descending in `save_library()` to match documented behavior.

### 2. Integrate backup into save flow
Either call `backup_library()` from `save_library()` automatically, or document clearly that it's a manual step.

### 3. Add library name case-insensitivity check
In `create_library()` and `add_server_library()`, check for case-insensitive duplicates on platforms that are case-insensitive.

### 4. Validate library_id as UUID
Use `Uuid::parse_str()` to validate before storing.

### 5. Add optional trash mechanism for delete_library()
Platforms with trash support (macOS) could move to `.Trash` instead of immediate deletion.

### 6. Add name -> description migration
Support deserializing legacy `name` field as alias for `description`.

### 7. Add sort_by_updated_at() method to Snippets
Allow callers to explicitly sort before save, making the operation explicit and testable.

### 8. Consider using chrono DateTime instead of i64
The `created_at`/`updated_at` fields are stored as `i64` unix timestamps. Using `DateTime<Utc>` would be more self-documenting and allow built-in formatting.

### 9. Expose library backup functionality through LibraryManager
Add `backup_library()` method to `LibraryManager` that calls the module-level function.

### 10. Document that deleted snippets are filtered elsewhere
Add a note that `deleted: true` filtering happens in `sync_commands.rs` or wherever queries are made, not in `library.rs` itself.

---

## Summary

| Aspect | Status |
|--------|--------|
| Data structure matches doc | ❌ Multiple discrepancies |
| load_library / save_library / backup_library exist | ✅ (standalone functions) |
| Sorting on save | ❌ Not implemented |
| Automatic backup | ❌ Not implemented |
| Migration | ✅ Partial |
| Soft delete field | ✅ Present |
| Error variant LibraryNotFound | ❌ Does not exist |