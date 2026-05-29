# Config Module Improvement Plan

## Architecture Claims vs. Implementation Verification

### 1. Config Directory Resolution (`src/utils/config.rs`)

| Claim | Status | Notes |
|-------|--------|-------|
| `get_config_dir()` uses `$XDG_CONFIG_HOME/snp` if set | ✅ Verified | Lines 4-13 |
| Default fallback is `~/.config/snp` | ✅ Verified | Lines 8-12 |
| macOS migration from `~/Library/Application Support/snp/` | ✅ Verified | `migrate_macos_config_dir()` at line 53 |
| Files are moved, not copied | ✅ Verified | Uses `fs::rename()` first (line 75), falls back to copy (line 78) |
| Legacy dir removed if empty | ✅ Verified | Lines 86-89 |

### 2. Sync Settings (`src/config.rs`)

| Claim | Status | Notes |
|-------|--------|-------|
| `SyncSettings` struct with all documented fields | ✅ Verified | Lines 21-39 |
| `SyncDirection` enum: Push, Pull, Bidirectional | ✅ Verified | Lines 118-123 |
| Default `sync_direction` is Push | ✅ Verified | Line 120 |
| Default `server_url` is `http://localhost:50051` | ✅ Verified | Line 128 |
| Default `sync_interval_minutes` is 30 | ✅ Verified | Line 132 |
| Default `enabled` is false | ✅ Verified | Line 101 |
| `load_sync_settings()` falls back to defaults if file missing | ✅ Verified | Lines 174-176 |
| `save_sync_settings()` writes with backslash-safe quoting | ✅ Verified | Line 164 |
| `get_sync_settings()` convenience wrapper | ✅ Verified | Lines 204-206 |

### 3. TOML Handling

| Claim | Status | Notes |
|-------|--------|-------|
| `fix_invalid_toml_escapes()` handles `\<` and `\>` | ✅ Verified | toml_helpers.rs:70-74 |
| `quote_strings_containing_backslashes()` on save | ✅ Verified | Called at config.rs:164 |
| Single-quoted strings used for backslash content | ✅ Verified | toml_helpers.rs:36-38 |
| Triple-quoted strings not processed | ✅ Verified | toml_helpers.rs:68-69 comment |

### 4. Environment Variables

| Variable | Status | Notes |
|----------|--------|-------|
| `XDG_CONFIG_HOME` used by `utils/config.rs` | ✅ Verified | config.rs:4 |
| `SNP_THEME`, `COLORFGBG`, `SHELL`, `EDITOR`, `RUST_LOG` | ⚠️ Not in config module | Documented in arch but used elsewhere |

---

## Bugs Found

### 1. Keychain Migration Silent Failure (Medium)
**Location**: `src/config.rs:188-199`

When migrating a plaintext API key to keychain on first load, failures are logged but not propagated. If `keychain_store()` fails, the plaintext key remains in `sync.toml` and the marker is not saved. The next load will attempt migration again.

**Impact**: API key may remain in plaintext config file if keychain is unavailable.

### 2. Keychain Unavailable on Deserialization Returns Empty Key (Medium)
**Location**: `src/config.rs:73`

When keychain is unavailable and `KEYCHAIN_MARKER` is encountered, deserialization returns `Ok(String::new())` instead of an error. The user sees no indication their API key failed to load.

**Impact**: Sync may fail silently with empty API key.

### 3. No Atomic Write for sync.toml (Low)
**Location**: `src/config.rs:166`

`save_sync_settings()` writes directly to the file without atomic rename. If the process crashes mid-write, `sync.toml` may be corrupted.

**Fix**: Write to temp file, then rename.

### 4. Race Condition in macOS Migration (Low)
**Location**: `src/utils/config.rs:68-84`

Multiple processes could race during migration. The `dst.exists()` check at line 72 is a TOCTOU race.

---

## Potential Improvements

### 1. Error Propagation for Keychain Failures
Currently keychain failures log warnings and fall back to plaintext storage/retrieval. Consider:
- Adding a `SyncSettings::keychain_available()` method
- Propagating keychain errors on load instead of returning empty string

### 2. Atomic Config Writes
Implement write-through-temp-file pattern for all config saves:
```rust
let temp_path = path.with_extension("tmp");
fs::write(&temp_path, &content)?;
temp_path.rename(&path)?;
```

### 3. Config Validation
Add validation to `load_sync_settings()`:
- Validate `server_url` is a valid URL
- Validate `sync_interval_minutes` > 0
- Validate `sync_direction` is a valid variant

### 4. Migration Status Tracking
Track migration completion in a versioned marker file to avoid repeated migration attempts and TOCTOU races.

### 5. Missing Documentation
- `device_id` field has no documentation on generation/rotation
- `clipboard_auto_clear_seconds` behavior when `None` (disabled?) not documented
- Keychain integration not documented in architecture

### 6. Test Coverage Gaps
- No tests for keychain failure/recovery path
- No tests for migration with partial state
- No tests for concurrent config access
- No tests for corrupt `sync.toml` recovery

---

## Discrepancies

1. **Documentation claims `get_sync_settings()` "never fails"** - While it returns defaults on error, this conflates "never returns an error" with "never fails." The `unwrap_or_default()` hides failures silently.

2. **Architecture lists `backups/` directory** - Not implemented in `get_config_dir()`. No backup creation logic exists.

3. **Architecture lists `premade/` directory** - Exists but managed by `snip-sync` server, not the config module.

4. **Architecture shows `audit.log` in layout** - Audit logging is in `src/logging.rs`, not config module.

---

## Security Considerations

1. **API key in plaintext fallback**: When keychain fails, API key is stored in plaintext `sync.toml`. Consider encrypting the plaintext fallback.

2. **No permission hardening**: Config files are created with default permissions (readable by others on multi-user systems). Consider `0o600` on `sync.toml`.

3. **No integrity checking**: `sync.toml` has no checksum or signature. Tampering is undetected.

---

## Performance Considerations

1. **TOML parsing on every load**: `load_sync_settings()` parses TOML every time. For frequently accessed settings, consider caching with invalidation.

2. **Regex compilation**: `TOML_STRING_PATTERN` uses `once_cell::Lazy` which is good, but the regex is recompiled per-string processed. Acceptable for small configs.
