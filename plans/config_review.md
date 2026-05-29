# Config Module Review

## Document Accuracy

### Verified Correct

| Claim (architecture/config.md) | Source |
|---|---|
| `get_config_dir()` uses `$XDG_CONFIG_HOME/snp` or `~/.config/snp` | `src/utils/config.rs:3-14` — matches exactly |
| macOS legacy path is `~/Library/Application Support/snp/` | `src/utils/config.rs:28` — uses `dirs::config_dir()` which resolves to this on macOS |
| `SyncSettings` struct fields and defaults | `src/config.rs:17-31` — all fields match; `clipboard_auto_clear_seconds: Option<u32>` confirmed |
| `SyncDirection` enum: Push, Pull, Bidirectional | `src/config.rs:52-58` — correct, `Push` is `#[default]` |
| `load_sync_settings()` falls back to defaults | `src/config.rs:104-120` — returns `Ok(SyncSettings::default())` when file missing |
| `save_sync_settings()` uses backslash-safe quoting | `src/config.rs:97` — calls `quote_strings_containing_backslashes` |
| `get_sync_settings()` never fails | `src/config.rs:122-124` — uses `unwrap_or_default()` |
| TOML stored in `~/.config/snp/sync.toml` | `src/utils/config.rs:103-105` — `get_sync_config_path()` returns `get_config_path("sync.toml")` |
| Environment variables table | Verified: `XDG_CONFIG_HOME` at `utils/config.rs:4`, `SHELL`/`EDITOR` in respective commands |
| `LibraryConfig` / `LibraryMeta` structs | `src/library.rs:69-85` — fields match: `filename`, `library_id`, `is_primary`, `last_sync` |

### Discrepancies

| Claim | Reality | Severity |
|---|---|---|
| Architecture doc says macOS migration: "Files are moved, not copied" | Code first tries `rename()`, then falls back to `copy_recursively()` + delete — so files ARE copied if rename fails (cross-device) | Minor — behavior is correct, wording is imprecise |
| Architecture doc says legacy dir is "removed if empty" | Code at `utils/config.rs:87` checks `read_dir().next().is_none()` and calls `remove_dir` — correct | None |
| `SyncSettings` doc says `sync_direction` default is "Push" | `SyncDirection::default()` returns `Push` via `#[default]` attribute — correct | None |
| Doc doesn't mention `SyncConfigFile` / `SyncConfigSettings` wrapper structs | `src/config.rs:68-78` — these exist to wrap `SyncSettings` under a `[sync]` TOML section | Documentation gap |

## Bugs & Issues

### Bug 1: Migration silently skips files that already exist in destination (Medium)
**Location**: `src/utils/config.rs:72-73`

```rust
if dst.exists() {
    continue;
}
```

If a file with the same name already exists in `~/.config/snp/`, the migration skips it with no warning. If the legacy file has newer content, data is silently lost. There's no merge or backup.

### Bug 2: Migration errors are silently swallowed (Medium)
**Location**: `src/utils/config.rs:75-83`

```rust
if std::fs::rename(&src, &dst).is_ok() {
    continue;
}
copy_recursively(&src, &dst)?;
if src.is_dir() {
    let _ = std::fs::remove_dir_all(&src);
} else {
    let _ = std::fs::remove_file(&src);
}
```

- If `copy_recursively` fails, the error propagates but files already copied are left in an inconsistent state.
- If `rename` fails and `copy_recursively` succeeds but `remove_dir_all`/`remove_file` fails, the error is silently ignored (`let _ =`). The user ends up with duplicate data.
- If `rename` succeeds but the source was a directory with contents, the old directory is left behind (no cleanup path for renamed dirs).

### Bug 3: `save_sync_settings` doesn't create parent directories of `sync.toml` in all cases (Low)
**Location**: `src/config.rs:83-86`

```rust
if let Some(parent) = path.parent() {
    fs::create_dir_all(parent)?;
}
```

This is fine for `sync.toml` (parent is `~/.config/snp/`), but the pattern is repeated inconsistently. `save_sync_settings` creates parent dirs, but `load_sync_settings` does not. If the config dir doesn't exist, load returns defaults but save creates it. This is a minor inconsistency rather than a bug.

### Bug 4: `get_sync_settings()` swallows all errors (Low)
**Location**: `src/config.rs:122-124`

```rust
pub fn get_sync_settings() -> SyncSettings {
    load_sync_settings().unwrap_or_default()
}
```

If `sync.toml` is corrupted, the user gets silent default settings with no indication anything is wrong. At minimum, this should log a warning.

### Bug 5: No validation of `sync_interval_minutes` (Low)
**Location**: `src/config.rs:24`

A user could set `sync_interval_minutes = 0`, which would cause extremely frequent sync attempts, or a very large value that effectively disables sync. No bounds checking exists.

### Bug 6: `clipboard_auto_clear_seconds` has no upper bound validation (Low)
**Location**: `src/config.rs:30`

Could be set to an absurdly large value. No validation at load time.

### Bug 7: Race condition in migration check (Low)
**Location**: `src/utils/config.rs:24-27`

```rust
let new_dir = get_config_dir();
if new_dir.exists() {
    return None;
}
```

Between the check and the actual migration in `migrate_macos_config_dir()`, another process could create `new_dir`. This is unlikely for a CLI tool but is a TOCTOU issue.

## Design Issues

### 1. Split responsibility between `src/config.rs` and `src/utils/config.rs`
**Files**: `src/config.rs`, `src/utils/config.rs`

The architecture doc acknowledges this but the split is confusing:
- `utils/config.rs` owns: `get_config_dir()`, path helpers, macOS migration
- `config.rs` owns: `SyncSettings`, `SyncDirection`, load/save for sync config

Meanwhile `LibraryConfig` and `LibraryMeta` live in `library.rs`. There's no clear principle for what goes where. `config.rs` re-exports `get_sync_config_path` from `utils/config.rs` (line 7), creating a circular-ish dependency path.

**Recommendation**: Consolidate into a single `config` module, or use a clear naming convention (e.g., `config/paths.rs`, `config/sync.rs`, `config/library.rs`).

### 2. `SyncConfigFile` and `SyncConfigSettings` are unnecessary wrappers
**Location**: `src/config.rs:68-78`

These wrapper structs exist solely to produce a `[sync]` TOML section. This adds complexity. The TOML format could use `[sync]` as the top-level key directly with `#[serde(rename = "sync")]` on a module-level struct.

### 3. Inconsistent error handling strategies
- `get_sync_settings()`: swallows all errors, returns default
- `load_sync_settings()`: propagates errors
- `LibraryManager::new()`: backs up corrupted files, logs warnings, returns default
- `load_library()`: backs up corrupted files, logs warnings, returns default

There's no consistent policy. Some callers get errors, some get defaults.

### 4. `migrate_from_single_file` doesn't remove the legacy file
**Location**: `src/library.rs:250-256`

After copying `snippets.toml` to `libraries/snippets.toml`, the original is left in place. This means the user has two copies of their data with no guidance on which to use.

### 5. `is_single_file_mode()` check is fragile
**Location**: `src/library.rs:217-219`

```rust
pub fn is_single_file_mode(&self) -> bool {
    !self.libraries_dir.exists()
}
```

This checks if the `libraries/` directory exists, but doesn't check if it's a file, a symlink, or has unexpected permissions. Edge case but could cause confusing errors.

### 6. Dead test
**Location**: `src/config.rs:150-154`

```rust
fn test_save_and_load_sync_settings() {
    let settings = SyncSettings::default();
    assert!(!settings.enabled);
}
```

This test name says "save and load" but only checks the default value. It doesn't actually test save/load roundtrip.

### 7. `add_server_library` can create duplicate metadata
**Location**: `src/library.rs:387-413`

If called twice with the same `server_name`, it pushes a new `LibraryMeta` each time without checking if one already exists for that filename. Compare with `add_existing_library` (line 362) which does check.

## Security Concerns

### 1. API key stored in plaintext TOML
**Location**: `src/config.rs:19-20`

`api_key` is stored as a plain string in `sync.toml`. The file has no special permissions set. On a multi-user system, other users could read it.

**Recommendation**: Set file permissions to `0600` on `sync.toml`, or consider OS keychain integration.

### 2. Server URL not validated
**Location**: `src/config.rs:19`

No URL validation is performed. A user could set `server_url` to an arbitrary string, which could be used for SSRF-like attacks if the sync client follows redirects or parses responses unsafely.

### 3. Migration prints paths to stderr
**Location**: `src/utils/config.rs:60-64`

```rust
eprintln!(
    "Migrating config from {} to {}",
    legacy_dir.display(),
    new_dir.display()
);
```

This is benign but in a shared environment, it reveals filesystem paths.

## Performance Issues

### 1. `get_config_dir()` called on every invocation
**Location**: `src/utils/config.rs:3-14`

`dirs::home_dir()` involves a system call on every invocation. For a CLI tool this is negligible, but it could be cached with `LazyLock` like `CONFIG_PATH` in `main.rs:41`.

### 2. `quote_strings_containing_backslashes` uses regex on every save
**Location**: `src/utils/toml_helpers.rs:57-58`

The regex `TOML_STRING_PATTERN` is compiled once via `Lazy`, but the full TOML content is scanned character by character on every save. For the small config files in this project, this is fine, but worth noting.

## Test Coverage Gaps

| Area | Status |
|---|---|
| `get_config_dir()` with custom `XDG_CONFIG_HOME` | Not tested — tests only check the path ends with "snp" |
| `get_legacy_macos_config_dir()` | No tests — macOS-only, not tested on other platforms |
| `migrate_macos_config_dir()` | No tests — complex logic with no unit tests |
| `copy_recursively()` | No tests |
| `save_sync_settings()` / `load_sync_settings()` roundtrip | No actual test — `test_save_and_load_sync_settings` doesn't test it |
| `SyncSettings` serialization/deserialization roundtrip | Partially tested — serialization tested, deserialization not |
| `SyncDirection` case sensitivity in TOML | Not tested — `"push"` vs `"Push"` behavior unclear |
| `get_sync_settings()` error swallowing | Not tested |
| File permissions on `sync.toml` | Not tested |
| Migration edge cases (rename fails, copy fails, remove fails) | Not tested |
| `load_sync_settings()` with corrupted TOML | Not tested |

## Priority Ranking

| Priority | Item | Location |
|---|---|---|
| High | Migration silently loses data when destination exists | `utils/config.rs:72` |
| High | `add_server_library` creates duplicate metadata | `library.rs:387-413` |
| High | API key stored in plaintext with no file permissions | `config.rs:19` |
| Medium | Migration errors silently swallowed on cleanup | `utils/config.rs:75-83` |
| Medium | `get_sync_settings()` swallows errors with no logging | `config.rs:122` |
| Medium | `migrate_from_single_file` doesn't remove legacy file | `library.rs:250-256` |
| Medium | Dead test `test_save_and_load_sync_settings` | `config.rs:150` |
| Medium | No validation of `sync_interval_minutes` bounds | `config.rs:24` |
| Low | Split config module responsibilities unclear | `config.rs` + `utils/config.rs` |
| Low | `SyncConfigFile`/`SyncConfigSettings` wrapper bloat | `config.rs:68-78` |
| Low | `is_single_file_mode()` fragile existence check | `library.rs:217` |
| Low | No `SyncDirection` case sensitivity tests | `config.rs:52-58` |

## Recommendations

1. **Add roundtrip tests** for `save_sync_settings()` → `load_sync_settings()` using a temp directory (set `XDG_CONFIG_HOME`).
2. **Fix `add_server_library`** to check for existing metadata before pushing, matching `add_existing_library`'s pattern.
3. **Add file permissions** (`chmod 600`) to `sync.toml` after writing.
4. **Log warnings** in `get_sync_settings()` when the file is corrupted.
5. **Test `migrate_macos_config_dir`** with mock filesystems or integration tests using temp dirs.
6. **Consider consolidating** `config.rs` and `utils/config.rs` into a `config/` module with clear submodules.
7. **Delete or fix** `test_save_and_load_sync_settings` to actually test the roundtrip.
8. **Add bounds validation** for `sync_interval_minutes` (e.g., min=1, max=10080 for weekly).
