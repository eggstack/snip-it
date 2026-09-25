# Configuration

[← Back to Overview](overview.md)

## Overview

Two configuration concerns live in different layers:

- **Path resolution** (`src/utils/config.rs`, core layer): `get_config_dir()`,
  `ensure_config_dir()`, per-file helpers, macOS migration. Pure path
  builders — no I/O except `ensure`/`migrate`.
- **Sync settings** (`src/config/`, sync-client layer): `SyncSettings`,
  `SyncDirection`, `AutoSyncFailureMode`, keychain-backed API-key storage,
  CRC32 integrity, and the shared `toml_cache` persistence utility that
  core (`library`) reuses through a documented carve-out.

## Config Directory Resolution

**File**: `src/utils/config.rs:14-28`

```rust
pub fn get_config_dir() -> PathBuf             // $XDG_CONFIG_HOME/snp or ~/.config/snp
pub fn ensure_config_dir() -> std::io::Result<PathBuf> // mkdir + 0o700, idempotent
pub fn get_config_path(filename: &str) -> PathBuf
pub fn get_snippets_path() -> PathBuf          // snippets.toml (legacy single-file)
pub fn get_sync_config_path() -> PathBuf       // sync.toml
pub fn derive_sync_state_dir() -> PathBuf      // parent of sync.toml; auto-sync state home
pub fn migrate_macos_config_dir() -> std::io::Result<()>
```

### Directory Layout

```
~/.config/snp/                    # XDG_CONFIG_HOME or ~/.config
├── snippets.toml                 # Legacy single-file (migrated)
├── libraries.toml                # Library metadata
├── sync.toml                     # Sync settings (+ CRC32 header)
├── usage.toml                    # Local usage metadata (not synced)
├── audit.log                     # Audit trail
├── libraries/                    # Individual library files
│   ├── snippets.toml
│   ├── work.toml
│   └── personal.toml
├── premade/                      # Downloaded premade libraries
├── logs/                         # Rolling log files (daily rotation)
└── backups/                      # Timestamped backups
```

`logs/` and `audit.log` are created lazily; `snp version` and
`snp completions bash` do not create them.

### macOS Migration — `src/utils/config.rs:112-156`

Old path (`~/Library/Application Support/snp/`) moves to
`~/.config/snp/` on first run via `LibraryManager::new()`. Files are
moved (rename, cross-device copy+fsync fallback), not copied; the legacy
dir is removed if empty. Interrupted cross-device moves resume next
startup because the legacy path is returned even when the new path
exists (`get_legacy_macos_config_dir`, `:69-80`).

## Sync Settings

**Files**: `src/config/mod.rs`, `src/config/sync_settings.rs`,
`src/config/toml_cache.rs`

### `SyncSettings` — `src/config/sync_settings.rs:91-132`

```rust
pub struct SyncSettings {
    pub enabled: bool,
    pub server_url: String,                 // default http://localhost:50051
    pub api_key: String,                    // keychain-backed; custom serde
    pub device_id: String,
    pub sync_interval_minutes: u32,         // default 30
    pub auto_sync: bool,                    // default false
    pub auto_sync_debounce_seconds: u64,    // default 2, clamped 0..=300
    pub auto_sync_failure: AutoSyncFailureMode, // default Warn
    pub auto_sync_max_delay_seconds: Option<u64>, // default 300, clamped 0..=600
    pub auto_sync_timeout_seconds: Option<u64>,   // default 30, clamped 5..=120
    pub sync_direction: SyncDirection,      // default Push
    pub clipboard_auto_clear_seconds: Option<u32>,
    pub sync_limit: Option<i32>,            // must be > 0; default 1000 via sync_limit_value()
    pub credential_revision: u64,           // monotonic key-change counter
}
pub fn SyncSettings::sync_limit_value(&self) -> i32
pub fn SyncSettings::auto_sync_debounce(&self) -> Duration
pub fn SyncSettings::auto_sync_max_delay(&self) -> Duration
pub fn SyncSettings::auto_sync_timeout(&self) -> Duration
pub fn SyncSettings::sync_config_file_exists() -> bool
```

Effective-value helpers clamp at read time, so hand-edited out-of-range
TOML cannot break scheduling. `auto_sync_timeout_seconds` bounds only
the detached auto-sync helper (request + backoff + recovery); manual
sync and cron keep their own timeouts, and local FS ops are never
force-cancelled.

### `SyncDirection` — `src/config/sync_settings.rs:419-425`

```rust
pub enum SyncDirection { Push, Pull, Bidirectional }
```

`Push` = local → server only; `Pull` = server → local only;
`Bidirectional` = merge (see `sync.md`).

### `AutoSyncFailureMode` — `src/config/sync_settings.rs:46-82`

```rust
pub enum AutoSyncFailureMode { Ignore, Warn, Error }
```

Failure behavior for post-mutation auto-sync. `Error` returns a nonzero
exit **after** the local mutation commits — it never implies rollback.
`Display`/`FromStr` use lowercase `ignore`/`warn`/`error`.

### TOML Format (`~/.config/snp/sync.toml`)

```toml
# integrity: 1234567890
[settings.sync]
enabled = true
server_url = "https://sync.example.com"
api_key = "@keychain"
device_id = "device-uuid"
sync_interval_minutes = 30
auto_sync = false
auto_sync_debounce_seconds = 2
auto_sync_timeout_seconds = 30
auto_sync_failure = "warn"
sync_direction = "Bidirectional"
clipboard_auto_clear_seconds = 30
```

### Load / Save — `src/config/sync_settings.rs:464-563`

```rust
pub fn save_sync_settings(settings: &SyncSettings) -> SnipResult<()>
pub fn load_sync_settings() -> SnipResult<SyncSettings>
pub fn get_sync_settings() -> SyncSettings // never fails; defaults on error
```

Save rejects non-positive `sync_limit`, serializes under
`[settings.sync]`, prepends `# integrity: <crc32>`, writes via
`write_private_atomic` (0600), invalidates the TOML cache, and
invalidates the clipboard settings cache (known cross-layer call noted
in `mod.rs:8-10`). Load verifies integrity first: failure backs up to
`sync.toml.corrupt.bak`, warns on stderr, and returns defaults —
legacy files without a header are accepted and gain one on next save.

## Keychain via keyring v4 — `src/config/sync_settings.rs:236-391`

- `KEYCHAIN_SERVICE = "snp-sync"`, user `"api-key"`, marker `"@keychain"`.
  `serialize_api_key` stores the real key in the OS keychain and writes
  only the marker to `sync.toml`; `deserialize_api_key` resolves the
  marker back through the keychain.
- Production **refuses plaintext**: if the keychain is unavailable, save
  and load fail rather than persisting/leaking the key. `Debug` prints
  `[REDACTED]`; `Drop` zeroizes `api_key` (`:134-169`).
- Test-only seams (gated on `test-support`, inactive in prod — see
  `scripts/ci/test-production-seams.sh`): `SNP_ALLOW_PLAINTEXT_API_KEY=true`
  writes/reads plaintext (but refuses to treat a stored `@keychain`
  marker as a credential), and `SNP_TEST_CREDENTIAL_FILE` points at a
  file holding the real key. Never remove these seams; tests set the
  former on every command.
- First load migrates an existing plaintext key into the keychain
  (`migrate_plaintext_api_key`, `:353-391`), keeping the in-memory value
  so the current operation still authenticates. Keep `keyring = "4"`
  default features or persistence silently falls back to the mock store.

## CRC32 integrity + `toml_cache` — `src/config/toml_cache.rs`

```rust
pub fn cached_read_toml(path: &Path) -> SnipResult<String>
pub fn invalidate_toml_cache(path: &Path)
pub(crate) fn compute_crc32(data: &str) -> u32
pub(crate) fn verify_integrity(content: &str) -> bool
pub(crate) fn strip_integrity_line(content: &str) -> String
```

- CRC32 (`crc32fast`) detects accidental corruption (partial writes,
  disk errors) — **not** an attacker defense; anyone who can write the
  config dir can recompute it (`toml_cache.rs:221-228`).
- The header must be the very first line so a user comment like
  `# integrity: 42` deeper in the file is not mistaken for it.
- `cached_read_toml` (`:250-350`) is a bounded (100-entry) mtime+len+
  nanos+inode/device cache with symlink-safe keys, poison recovery, and
  a read-verify-retry loop for atomic-replace races. Every writer
  (`save_library`, `save_config`, `save_sync_settings`) must call
  `invalidate_toml_cache` after mutating (including alt/raw key forms).
- Pure persistence utility: core (`library`) imports only
  `cached_read_toml`/`invalidate_toml_cache` via the documented carve-out
  (`mod.rs:13-17`); sync/keychain parts stay out of core per
  `tests/architecture.rs` layer rules.

## Environment Variables

| Variable | Used By | Default |
|----------|---------|---------|
| `XDG_CONFIG_HOME` | `utils/config.rs` | `~/.config` |
| `SNP_THEME` | `ui/theme.rs` | `"dark"` |
| `COLORFGBG` | `ui/theme.rs` (theme detection) | — |
| `SHELL` | `run_cmd.rs` | `"/bin/sh"` |
| `EDITOR` | `new_cmd.rs` | `"vim"` |
| `RUST_LOG` | `tracing-subscriber` | `"snp=info"` |
| `SNP_LOG` | `logging.rs` | — (per-module filter) |
| `SNP_COMMAND_TIMEOUT` | `run_cmd.rs` | 0 (disabled) |
| `SNP_CLIPBOARD_TIMEOUT` | `clipboard.rs` | `5` |
| `SNP_ALLOW_PLAINTEXT_API_KEY` | `src/config/` (test-only) | `false` (exact `=true` required) |
| `SNP_TEST_CREDENTIAL_FILE` | `src/config/` (test-only) | — |
| `SNP_TEST_EVENTS_DIR` | `auto_sync/test_events.rs` (test-only) | unset (no events unless set) |
| `SNP_SKIP_WORKER_SPAWN` | `auto_sync/` (test-only) | unset |
| `SNP_TEST_FAILPOINT` | `test_failpoints.rs` (test-only) | unset |
| `SNP_TEST_MUTATION_BARRIER_DIR` | `local_data.rs` (test-only) | unset |
| `SNP_ALLOW_DIR_FSYNC_FAILURE` | `utils/atomic.rs` (test-only) | unset |
| `SNIP_SYNC_ALLOW_HTTP` | client + server (truthy `true/1/yes/on`; server: loopback only) | `false` |
| `SNIP_UPDATE_CRATES_API_URL` / `SNIP_UPDATE_RELEASE_BASE_URL` | `update.rs` (test-only) | crates.io / GitHub releases |
| `SNP_SYNC_CONNECT_TIMEOUT` | `sync.rs` | `10` |
| `SNP_SYNC_REQUEST_TIMEOUT` | `sync.rs` | `30` |

Test-only vars are inert in production builds (proven by `scripts/ci/test-production-seams.sh`).

## Key Files

- `src/utils/config.rs` — config dir, per-file paths, macOS migration.
- `src/config/sync_settings.rs` — `SyncSettings`, direction/failure
  enums, keychain serde, save/load/get, clamping helpers.
- `src/config/toml_cache.rs` — shared TOML cache + CRC32 helpers.
- `src/config/tests.rs` — defaults, limits, round-trips, integrity cases.
