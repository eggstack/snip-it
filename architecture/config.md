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

Everything that writes inside the config directory derives its paths from
`get_config_dir()`, never from the path of the file being written. That
single rule is what keeps the transaction gate, the local-data lock, and the
test override aligned — see [Config Directory Resolution](#config-directory-resolution).

## Config Directory Resolution

**File**: `src/utils/config.rs`

```rust
// src/utils/config.rs:63-82
pub fn get_config_dir() -> PathBuf

// src/utils/config.rs:91-116
pub fn ensure_config_dir() -> std::io::Result<PathBuf>

// src/utils/config.rs:212-232
pub fn get_config_path(filename: &str) -> PathBuf
pub fn get_snippets_path() -> PathBuf       // snippets.toml (legacy single-file)
pub fn get_sync_config_path() -> PathBuf    // sync.toml
pub fn derive_sync_state_dir() -> PathBuf   // parent of sync.toml
```

### Resolution Order

`get_config_dir()` (`src/utils/config.rs:63-82`) resolves, in order:

1. the test-only thread-local override (see
   [Test-Only Config Directory Override](#test-only-config-directory-override));
2. `$XDG_CONFIG_HOME/snp` when `XDG_CONFIG_HOME` is set and non-empty;
3. otherwise `~/.config/snp`.

There is **no per-platform `#[cfg]` branch**: macOS and Windows use the same
`~/.config` layout as Linux. If the home directory cannot be resolved the
function warns and falls back to `.` (the current directory).

`ensure_config_dir()` creates the directory if missing and tightens it to
`0o700` on Unix (it also tightens an existing directory whose mode has group
or world bits, `src/utils/config.rs:94-105`).

### Directory Layout

```
~/.config/snp/                    # XDG_CONFIG_HOME or ~/.config
├── snippets.toml                 # Legacy single-file (migrated)
├── libraries.toml                # Library metadata
├── sync.toml                     # Sync settings (+ CRC32 header)
├── usage.toml                    # Local usage metadata (not synced)
├── audit.log                     # Audit trail
├── .transaction/                 # Transaction journals + local-data.lock
├── libraries/                    # Individual library files
│   ├── snippets.toml
│   ├── work.toml
│   └── personal.toml
├── premade/                      # Downloaded premade libraries
├── logs/                         # Rolling log files (daily rotation)
└── backups/                      # Timestamped backups
```

`libraries/`, `premade/`, `libraries.toml` come from
`src/library/manager.rs:49-51`; `.transaction/` from
`src/library/manager.rs:628` and `src/local_data.rs:398-400`; `backups/` from
`src/library/persistence.rs:316` and `src/commands/backup_cmd.rs:63`;
`logs/` from `src/logging.rs:72`; `audit.log` from `src/logging.rs:380-384`.

`logs/` and `audit.log` are created lazily; `snp version` and
`snp completions bash` do not create them.

### The Sync State Directory

`derive_sync_state_dir()` (`src/utils/config.rs:227-232`) returns the **parent
directory of `sync.toml`** — which is the config directory itself. It is the
canonical home for the auto-sync state files (pending marker, worker lock,
execution lock, status file). `src/local_data.rs:398-400` builds on it:

```rust
// src/local_data.rs:398-400
pub fn transaction_dir() -> PathBuf {
    crate::config::derive_sync_state_dir().join(".transaction")
}
```

So `<config_dir>/.transaction/` holds both the transaction journals and the
local-data lock. The two paths are distinct but sibling:

| Path | Purpose |
|------|---------|
| `derive_sync_state_dir()` = `<config_dir>` | pending marker / auto-sync state |
| `transaction_dir()` = `<config_dir>/.transaction` | journals + local-data lock |

`save_library` uses **both** — the gate needs the state directory (where the
pending marker lives) and the transaction directory (where journals and locks
live):

```rust
// src/library/persistence.rs:223-228
let sync_state_dir = crate::config::derive_sync_state_dir();
let transaction_dir = crate::local_data::transaction_dir();
crate::transaction::gate_mutation_on_interrupted_transactions(
    &sync_state_dir,
    &transaction_dir,
)?;
```

Both are derived from the **config directory**, not from the target library
path. That matters: deriving the lock path from the library path would put a
lock next to each library file, so two libraries would not exclude each other
and the lock would not be the one the transaction machinery expects.

### macOS Migration — `src/utils/config.rs:123-134`, `:166-210`

Old path (`~/Library/Application Support/snp/`) moves to
`~/.config/snp/` on first run via `LibraryManager::new()`. Files are
**moved** (rename, cross-device copy+fsync fallback), not copied; the legacy
dir is removed if empty. Interrupted cross-device moves resume next startup
because the legacy path is returned even when the new path exists
(`get_legacy_macos_config_dir`, `src/utils/config.rs:123-134`).

### Test-Only Config Directory Override

```rust
// src/utils/config.rs:35-49
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub struct ScopedConfigDir {
    previous: Option<PathBuf>,
}

#[cfg(any(test, feature = "test-support"))]
impl ScopedConfigDir {
    pub fn new(dir: impl Into<PathBuf>) -> Self
}
```

`ScopedConfigDir` is an RAII override of `get_config_dir()` for the **current
thread**. It is compiled only into unit-test builds and `test-support` feature
builds (`#[cfg(any(test, feature = "test-support"))]` on the struct and both
impls), so production binaries cannot be steered by it.

The backing state is a thread-local:

```rust
// src/utils/config.rs:15-22
#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static CONFIG_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}
```

`get_config_dir()` consults it first (`src/utils/config.rs:64-67`), and the
`Drop` impl restores the previous value (`src/utils/config.rs:51-57`) —
including on assertion panic — so a failing test cannot leak the redirect into
the tests running beside it. Nested overrides unwind LIFO.

**Why it is thread-local, not process-global.** Cargo runs unit tests in
parallel threads inside one process. An `XDG_CONFIG_HOME` mutation is
process-global and would race with every other test that resolves a path; a
thread-local override keeps each test hermetic without touching
process-global environment state. The trade-off is that a thread-local
override does **not** propagate to helper threads spawned by the test — a test
that spawns a worker thread and expects the redirected config directory must
set `XDG_CONFIG_HOME` instead.

**Why it exists.** Before `ScopedConfigDir` existed, `save_library` derived
its transaction-gate and local-data-lock paths from the *target library
path*, so unit tests wrote to the developer's real `~/.config/snp/` — mutating
a real `audit.log`, a real `usage.toml`, and the real `.transaction/`
directory — and contended on one process-wide lock file. That contention is
what made `test_serialization_matrix_makefile_leading_tabs_with_trailing_newline`
fail on a 30 s local-data-lock timeout. With the paths derived from the
config directory and the config directory redirected per test, each test gets
its own gate, its own journal directory, and its own lock.

**This is a test-only seam.** It is not part of production behavior and is not
something a production reviewer audits; `#[cfg(any(test, feature =
"test-support"))]` means it does not exist in a release build.

## Sync Settings

**Files**: `src/config/mod.rs`, `src/config/sync_settings.rs`,
`src/config/toml_cache.rs`

`src/config/mod.rs` re-exports the pre-split `crate::config::*` paths:
`derive_sync_state_dir` and `get_sync_config_path` from `utils::config`,
everything from `sync_settings`, and `cached_read_toml` /
`invalidate_toml_cache` from `toml_cache` (`src/config/mod.rs:24-26`).

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
pub fn SyncSettings::sync_limit_value(&self) -> i32          // :196-198
pub fn SyncSettings::auto_sync_debounce(&self) -> Duration    // :202-207
pub fn SyncSettings::auto_sync_max_delay(&self) -> Duration   // :211-217
pub fn SyncSettings::auto_sync_timeout(&self) -> Duration     // :222-228
pub fn SyncSettings::sync_config_file_exists() -> bool        // :231-233
```

### TOML Keys

| Rust field | TOML key | Default | Notes |
|-------------|----------|---------|-------|
| `enabled` | `enabled` | `false` | `snp sync` refuses to run when false |
| `server_url` | `server_url` | `http://localhost:50051` (`src/config/sync_settings.rs:24`) | `https://` in production |
| `api_key` | `api_key` | `""` | custom serde; stored as `@keychain` marker |
| `device_id` | `device_id` | `""` | `#[serde(default)]` |
| `sync_interval_minutes` | `sync_interval_minutes` | `30` | |
| `auto_sync` | `auto_sync` | `false` | `#[serde(default)]` |
| `auto_sync_debounce_seconds` | `auto_sync_debounce_seconds` | `2` | clamped `0..=300` (`src/config/sync_settings.rs:27-29`) |
| `auto_sync_failure` | `auto_sync_failure` | `warn` | `snake_case` |
| `auto_sync_max_delay_seconds` | `auto_sync_max_delay_seconds` | `300` | clamped `0..=600` (`src/config/sync_settings.rs:31-33`) |
| `auto_sync_timeout_seconds` | `auto_sync_timeout_seconds` | `30` | clamped `5..=120` (`src/config/sync_settings.rs:35-39`) |
| `sync_direction` | `sync_direction` | `Push` | `Push`/`Pull`/`Bidirectional` (PascalCase) |
| `clipboard_auto_clear_seconds` | `clipboard_auto_clear_seconds` | `None` | |
| `sync_limit` | `sync_limit` | `None` → 1000 | non-positive values rejected at parse and at save (`src/config/sync_settings.rs:439-449`, `:465-470`) |
| `credential_revision` | `credential_revision` | `0` | monotonic counter; detects credential replacement without persisting the key |

Effective-value helpers clamp at read time, so hand-edited out-of-range TOML
cannot break scheduling. `auto_sync_timeout_seconds` bounds only the detached
auto-sync helper (request + backoff + recovery); manual sync and cron keep
their own timeouts, and local FS ops are never force-cancelled.

### `SyncDirection` — `src/config/sync_settings.rs:418-425`

```rust
pub enum SyncDirection { Push, Pull, Bidirectional }
```

`Push` = local → server only; `Pull` = server → local only;
`Bidirectional` = merge (see [sync.md](sync.md)). The default is `Push`
(`src/config/sync_settings.rs:421`).

### `AutoSyncFailureMode` — `src/config/sync_settings.rs:46-58`, `:70-82`

```rust
pub enum AutoSyncFailureMode { Ignore, Warn, Error }
```

Log level for post-mutation auto-sync scheduling failures. The mode selects the
`tracing` level only (`Ignore`: debug, `Warn`: warn, `Error`: error —
`src/auto_sync/notification.rs:242-254`). It never implies rollback, never
changes the exit code, and never suppresses the `warning:` that
`report_notification_result` prints on stderr.
`Display`/`FromStr` use lowercase `ignore`/`warn`/`error`, and the TOML value
is snake_case (`#[serde(rename_all = "snake_case")]`,
`src/config/sync_settings.rs:47`).

### `sync.toml` On-Disk Layout

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
auto_sync_max_delay_seconds = 300
auto_sync_timeout_seconds = 30
auto_sync_failure = "warn"
sync_direction = "Bidirectional"
clipboard_auto_clear_seconds = 30
sync_limit = 1000
credential_revision = 0
```

The file is a single `[settings.sync]` table: `SyncConfigFile { settings:
SyncConfigSettings { sync: SyncSettings } }`
(`src/config/sync_settings.rs:452-461`). The first line is the CRC32
integrity header; everything after it is the TOML body.

### Load / Save — `src/config/sync_settings.rs:464-563`

```rust
pub fn save_sync_settings(settings: &SyncSettings) -> SnipResult<()>  // :464
pub fn load_sync_settings() -> SnipResult<SyncSettings>              // :498
pub fn get_sync_settings() -> SyncSettings // never fails; defaults on error // :555
```

**Save** (`src/config/sync_settings.rs:464-496`):

1. Rejects a non-positive `sync_limit` (`:465-470`).
2. Acquires the local-data lock on the transaction directory
   (`:472-473`) — the same lock the library mutation path uses.
3. Serializes under `[settings.sync]` with `toml::to_string_pretty` (`:482`).
4. Prepends `# integrity: <crc32>` (`:485-486`).
5. Writes via `write_private_atomic` (0600) (`:488`).
6. Invalidates the TOML cache and the clipboard settings cache
   (`:492-493`). The clipboard call is a known cross-layer dependency noted
   in `src/config/mod.rs:8-10`.

**Load** (`src/config/sync_settings.rs:498-553`):

1. A missing file yields `SyncSettings::default()` (`:501-503`).
2. The content is read through `cached_read_toml` (`:505`).
3. The integrity check runs **first**: on failure the file is copied to
   `sync.toml.corrupt.bak`, a warning is printed on stderr, and the defaults
   are returned (`:507-533`).
4. The header is stripped and invalid TOML escapes are repaired
   (`:535-536`, `src/utils/toml_helpers.rs:205-209`).
5. The TOML is parsed and the `settings.sync` table is extracted (`:538-541`).
6. An existing plaintext API key is migrated into the keychain, keeping the
   plaintext value in memory so the current operation still authenticates
   (`:543-550`).

Legacy files without an integrity header are accepted (`verify_integrity`
returns `true` when no header is present, `src/config/toml_cache.rs:236-241`)
and gain one on the next save.

## Keychain via keyring v4 — `src/config/sync_settings.rs:236-391`

- `KEYCHAIN_SERVICE = "snp-sync"`, user `"api-key"`, marker `"@keychain"`
  (`src/config/sync_settings.rs:20-22`).
  `serialize_api_key` (`:236-270`) stores the real key in the OS keychain and
  writes only the marker to `sync.toml`; `deserialize_api_key` (`:272-334`)
  resolves the marker back through the keychain.
- Production **refuses plaintext**: if the keychain is unavailable, save
  returns a serde error rather than persisting the key, and load returns a
  serde error rather than substituting a bogus credential
  (`:263-268`, `:320-329`). `Debug` prints `[REDACTED]`; `Drop` zeroizes
  `api_key` (`:134-169`).
- First load migrates an existing plaintext key into the keychain
  (`migrate_plaintext_api_key`, `:353-391`), keeping the in-memory value so
  the current operation still authenticates. Keep `keyring = "4"` default
  features. keyring 4's `default` is `v1`, which hard-enables the three
  platform stores; `keyring::Entry` is gated on `v1`, so `default-features =
  false` is a compile error, not a silent downgrade. There is no mock store.

### The Two Test-Only Credential Seams

Both are gated behind the `test-support` **Cargo feature**, not just an
environment variable, so they are physically absent from production builds.
`scripts/ci/test-production-seams.sh` proves they are inert in a prod build.

| Variable | Effect | Source |
|----------|--------|--------|
| `SNP_ALLOW_PLAINTEXT_API_KEY=true` | The API key is written and read as plaintext in `sync.toml` instead of going through the keychain. The value must be exactly `true`. | `:250-259`, `:302-317`, `:366-375` |
| `SNP_TEST_CREDENTIAL_FILE=<path>` | The real key lives in a file; `sync.toml` and the file stay in sync and the keychain is bypassed entirely. | `:248-253`, `:280-301`, `:366-369` |

Details a reviewer should know:

- `SNP_ALLOW_PLAINTEXT_API_KEY=true` **refuses to treat a stored `@keychain`
  marker as a credential** (`:302-317`). Returning the literal marker would
  authenticate every subsequent sync with the bogus string `"@keychain"`, so
  the load fails fast and tells the user to re-save the settings.
- `SNP_TEST_CREDENTIAL_FILE` takes precedence over the plaintext seam
  (`:248-253` runs first) and an empty or unreadable file is warned about
  rather than silently producing an empty key (`:285-300`).
- The plaintext seam also suppresses the keychain migration on first load
  (`:366-375`) — migrating would overwrite the authoritative source.

Never remove these seams; tests set the former on every command.

## CRC32 Integrity + `toml_cache` — `src/config/toml_cache.rs`

```rust
pub fn cached_read_toml(path: &Path) -> SnipResult<String>       // :250-350
pub fn invalidate_toml_cache(path: &Path)                        // :83-112
pub(crate) fn compute_crc32(data: &str) -> u32                   // :206-208
pub(crate) fn verify_integrity(content: &str) -> bool             // :228-242
pub(crate) fn strip_integrity_line(content: &str) -> String       // :244-248
```

### Integrity Header

- CRC32 (`crc32fast`) detects accidental corruption (partial writes, disk
  errors) — **not** an attacker defense; anyone who can write the config dir
  can recompute it (`src/config/toml_cache.rs:221-227`).
- The header must be the **very first line** so a user comment like
  `# integrity: 42` deeper in the file is not mistaken for it
  (`src/config/toml_cache.rs:229-230`, `:210-219`).
- A file with **no** header is a legacy file and is treated as valid rather
  than reset to defaults, which would lose data on upgrade
  (`src/config/toml_cache.rs:236-241`). The header is added on the next save.
- A malformed header (non-numeric checksum) fails the check
  (`src/config/toml_cache.rs:232-234`).

### Cache Semantics

- Bounded at `MAX_TOML_CACHE_SIZE = 100` entries, evicted in insertion order
  (`src/config/toml_cache.rs:57`, `:314-321`).
- The cache key is the **canonicalized** path, with a parent+filename
  fallback for paths that do not yet exist, and a raw-path fallback for paths
  with no filename (`:114-134`). This keeps symlink aliases from serving
  divergent entries.
- A cache hit requires the metadata to match on `mtime`, `mtime_nanos`, `len`,
  **and** the file identity — `inode` + `device` on Unix, `file_index` +
  `volume_serial` on Windows (`:267-285`). On Windows, an unreadable
  identity bypasses the cache entirely rather than trusting a synthesized
  shared identity (`:255-266`, `:305-311`).
- Reads use a read-verify-retry loop: the file's metadata is sampled before
  and after the read, and a changed snapshot is retried once before giving up
  with `runtime_error("read toml file", "file changed while being read")`
  (`:291-304`, `:346-349`). This keeps content and cache metadata tied to one
  snapshot even when the path is atomically replaced underneath.
- A poisoned mutex is recovered by **clearing the whole cache** rather than
  trusting entries a panicking holder may have left half-written
  (`:69-81`).

### Invalidation

Every writer must call `invalidate_toml_cache(path)` after mutating:
`save_library` (via the library persistence path), `save_config`, and
`save_sync_settings` (`src/config/sync_settings.rs:492`). `invalidate_toml_cache`
removes the canonical key **and** the alternative string forms — the
parent+filename key and the raw path key — because the cache key can diverge
when a symlink is atomically replaced (`src/config/toml_cache.rs:83-112`).

### Layer Carve-Out

`toml_cache` is a pure persistence utility: no keyring, no sync, no serde
dependency (`src/config/toml_cache.rs:1-7`). Core (`library`) imports only
`cached_read_toml` / `invalidate_toml_cache` via the documented carve-out
(`src/config/mod.rs:12-17`); the sync/keychain parts stay out of core per the
`tests/architecture.rs` layer rules.

## Environment Variables

| Variable | Used By | Default |
|----------|---------|---------|
| `XDG_CONFIG_HOME` | `utils/config.rs` | `~/.config` |
| `SNP_THEME` | `ui/theme.rs` | `"dark"` |
| `COLORFGBG` | `ui/theme.rs` (theme detection) | — |
| `SHELL` | `run_cmd.rs` | `"/bin/sh"` |
| `EDITOR` | `new_cmd.rs` | `"vim"` |
| `RUST_LOG` | `tracing-subscriber` | `"snip_it=info"` (the built-in fallback) |
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

- `src/utils/config.rs` — config dir resolution, per-file paths, macOS
  migration, the test-only `ScopedConfigDir` override.
- `src/config/mod.rs` — module split and the pre-split re-exports.
- `src/config/sync_settings.rs` — `SyncSettings`, direction/failure enums,
  keychain serde, save/load/get, clamping helpers.
- `src/config/toml_cache.rs` — shared TOML cache + CRC32 helpers.
- `src/config/tests.rs` — defaults, limits, round-trips, integrity cases.
- `src/local_data.rs:398-400` — `transaction_dir()`, built on
  `derive_sync_state_dir()`.
- `src/library/persistence.rs:217-236` — `save_library`, whose gate and lock
  paths come from the config directory.

## Invariants and Gotchas

- **Every path inside the config directory derives from
  `get_config_dir()`** — never from the path of the file being written. The
  transaction gate, the local-data lock, the journals, and the auto-sync
  state all hang off the config directory.
- **The transaction directory is `<config_dir>/.transaction`** and is shared
  by the journals and the local-data lock
  (`src/local_data.rs:398-400`).
- **A CRC32 header is corruption detection, not authentication.** An attacker
  with write access to the config dir can recompute it.
- **A missing integrity header means "legacy", not "corrupt."** Resetting to
  defaults there would lose the user's configuration.
- **Plaintext API keys are refused in production.** Both plaintext seams are
  behind the `test-support` Cargo feature, so they do not exist in a release
  build.
- **`SNP_ALLOW_PLAINTEXT_API_KEY=true` must not resurrect the `@keychain`
  marker as a credential** — the load fails fast instead.
- **Keep `keyring = "4"` with default features.** `default` is `v1`, which
  hard-enables the three platform stores; `keyring::Entry` is gated on `v1`, so
  `default-features = false` is a compile error, not a silent downgrade. There
  is no mock store.
- **`ScopedConfigDir` is `#[cfg(any(test, feature = "test-support"))]` and
  thread-local.** It does not exist in production, and it does not propagate to
  threads the test spawns.
