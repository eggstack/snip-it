# config.rs — Config Directory Resolution & Path Helpers

[← Back to Overview](../overview.md)

## Overview

`src/utils/config.rs` resolves the snp config directory, exposes the
per-file path helpers built on top of it, performs the one-way macOS legacy
migration, and provides the `#[cfg(any(test, feature = "test-support"))]`-only
`ScopedConfigDir` override that keeps unit tests out of the developer's real
`~/.config/snp/`.

**File**: `src/utils/config.rs` (303 lines, including the unit-test tail that
starts at `src/utils/config.rs:234`).

**Not covered here:** sync settings and the `toml_cache` read cache (those
live in `src/config/` — see [config.md](../config.md)), the library registry
(`src/library/manager.rs`), and the transaction/local-data lock hierarchy
(these *consume* the paths derived here — see
[persistence.md](../persistence.md)).

There is **no per-platform `#[cfg]` branch**. macOS and Windows use the same
`~/.config` layout as Linux; the only platform-specific behaviour in this
module is the macOS *legacy* migration, which is a one-way move of an older
directory into that shared layout.

## Config Directory Resolution

```rust
// src/utils/config.rs:63-82
pub fn get_config_dir() -> PathBuf
```

`get_config_dir()` is a **pure path builder** — it touches no filesystem, so
it is safe to call from any layer, including read-only paths. It resolves, in
order:

1. **The test-only thread-local override** —
   `#[cfg(any(test, feature = "test-support"))]` only; see
   [Test-Only Config Directory Override](#test-only-config-directory-override).
2. **`$XDG_CONFIG_HOME/snp`** when `XDG_CONFIG_HOME` is set **and non-empty**
   (`src/utils/config.rs:69-72`). An empty value is filtered out and falls
   through to the home-directory branch.
3. **`<home>/.config/snp`** otherwise (`src/utils/config.rs:73-81`).

If `dirs::home_dir()` cannot be resolved the function logs a warning and
falls back to the current directory (`src/utils/config.rs:74-79`), which
yields **`./.config/snp`** — note the `.config` segment is still applied by
the enclosing `.join(".config")` before the final `.join("snp")`.

The function is deterministic across calls (`src/utils/config.rs:262-270`
asserts it), which is what lets callers treat it as a cheap path builder.

### ensure_config_dir()

```rust
// src/utils/config.rs:91-116
pub fn ensure_config_dir() -> std::io::Result<PathBuf>
```

Creates the directory if missing and tightens permissions to `0o700` on Unix.
Idempotent: calling it twice returns the same path and succeeds both times
(`src/utils/config.rs:290-302`).

- If the directory already exists, its mode is checked and **tightened** to
  `0o700` if any group or world bit is set (`src/utils/config.rs:94-105`).
- If it does not exist, it is created and then set to `0o700`
  (`src/utils/config.rs:106-113`).

Call it once during startup before any I/O inside the config directory.
Individual I/O helpers (logging init at `src/logging.rs:177`, library
creation, premade downloads, audit logging) also call it defensively in case
the startup hook was skipped.

## Path Helpers

| Function | Lines | Returns |
|----------|-------|---------|
| `get_config_path(filename)` | `:212-214` | `get_config_dir().join(filename)` |
| `get_snippets_path()` | `:216-218` | `<config>/snippets.toml` (legacy single-file library) |
| `get_sync_config_path()` | `:220-222` | `<config>/sync.toml` |
| `derive_sync_state_dir()` | `:227-232` | Parent directory of `get_sync_config_path()` |

```rust
// src/utils/config.rs:227-232
pub fn derive_sync_state_dir() -> PathBuf {
    get_sync_config_path()
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf()
}
```

`derive_sync_state_dir()` is the canonical home for auto-sync state files
(pending marker, worker lock, execution lock, status file). Because
`get_sync_config_path()` is `<config>/sync.toml`, its parent **is** the config
directory. `src/local_data.rs:398-400` builds the transaction directory on top
of it:

```rust
// src/local_data.rs:398-400
pub fn transaction_dir() -> PathBuf {
    crate::config::derive_sync_state_dir().join(".transaction")
}
```

| Path | Purpose |
|------|---------|
| `derive_sync_state_dir()` = `<config_dir>` | pending marker / auto-sync state |
| `transaction_dir()` = `<config_dir>/.transaction` | journals + `local-data.lock` |

Both are **derived from the config directory, never from the target library
path**. `save_library` needs both — the gate reads the pending marker from
the state directory and the journals live in the transaction directory:

```rust
// src/library/persistence.rs:264-269
let sync_state_dir = crate::config::derive_sync_state_dir();
let transaction_dir = crate::local_data::transaction_dir();
crate::transaction::gate_mutation_on_interrupted_transactions(
    &sync_state_dir,
    &transaction_dir,
)?;
```

Deriving the lock path from the library path instead would put a lock next to
each library file — two libraries would not exclude each other, and the lock
would not be the one the transaction machinery expects.

Other config paths (`libraries/`, `premade/`, `libraries.toml`, `logs/`,
`audit.log`, `usage.toml`, `backups/`) are **not** helpers in this module;
they are derived at their use sites. See [config.md](../config.md) for the
full directory layout.

## macOS Legacy Migration

```rust
// src/utils/config.rs:123-134
pub fn get_legacy_macos_config_dir() -> Option<PathBuf>

// src/utils/config.rs:166-210
pub fn migrate_macos_config_dir() -> std::io::Result<()>
```

The legacy location is `~/Library/Application Support/snp/` (obtained via
`dirs::config_dir()`).

`get_legacy_macos_config_dir()` returns `None` on every non-macOS platform
(`src/utils/config.rs:124-126` — a runtime `cfg!` check, not a compile-time
`#[cfg]`). On macOS it returns the legacy path when it **exists and differs
from** the canonical path (`src/utils/config.rs:127-133`).

The "exists" test is deliberately independent of whether the new path already
exists: returning the legacy path even when `~/.config/snp/` is populated lets
an **interrupted cross-device migration resume** on the next startup.

`migrate_macos_config_dir()` moves every entry of the legacy directory into
the canonical one:

1. No legacy directory → no-op (`src/utils/config.rs:167-170`).
2. `create_dir_all` the canonical directory (`src/utils/config.rs:179`).
3. For each entry, try `rename` first (`src/utils/config.rs:185`).
4. On `CrossesDevices` **or** when the destination already exists, fall back
   to `copy_recursively` + `sync_recursively`, then delete the source
   (`src/utils/config.rs:187-195`). The helpers are at
   `src/utils/config.rs:136-161`.
5. Any other error aborts the migration and is returned
   (`src/utils/config.rs:196`).
6. `sync_all` the canonical directory on Unix (`src/utils/config.rs:200-201`).
7. Remove the legacy directory if it is now empty
   (`src/utils/config.rs:204-206`).

Files are **moved**, not copied — the legacy directory is removed once empty.

The migration is triggered from `LibraryManager::new()`
(`src/library/manager.rs:44-47`); a failure is logged as a warning and does
not abort startup.

## Test-Only Config Directory Override

```rust
// src/utils/config.rs:15-22
#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static CONFIG_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

// src/utils/config.rs:35-39
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub struct ScopedConfigDir {
    previous: Option<PathBuf>,
}

// src/utils/config.rs:41-49
#[cfg(any(test, feature = "test-support"))]
impl ScopedConfigDir {
    pub fn new(dir: impl Into<PathBuf>) -> Self
}

// src/utils/config.rs:51-57
#[cfg(any(test, feature = "test-support"))]
impl Drop for ScopedConfigDir {
    fn drop(&mut self) { /* restore previous */ }
}
```

`ScopedConfigDir` is an RAII override of `get_config_dir()` for the **current
thread**:

- `ScopedConfigDir::new(dir)` installs `dir` and remembers the previous value
  (`src/utils/config.rs:44-48`).
- `Drop` restores the previous value (`src/utils/config.rs:51-57`), including
  on an assertion panic, so a failing test cannot leak the redirect into the
  tests running beside it.
- Nested overrides unwind LIFO (`src/utils/config.rs:278-287` asserts this).

`get_config_dir()` consults the override **first**, before any environment
lookup:

```rust
// src/utils/config.rs:63-67
pub fn get_config_dir() -> PathBuf {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(dir) = CONFIG_DIR_OVERRIDE.with(|slot| slot.borrow().clone()) {
        return dir;
    }
    ...
```

### Why it is thread-local, not process-global

Cargo runs unit tests in parallel threads inside one process. An
`XDG_CONFIG_HOME` mutation is process-global and would race with every other
test that resolves a path. A thread-local override keeps each test hermetic
without touching process-global environment state.

The trade-off: a thread-local override does **not** propagate to helper
threads spawned by the test. A test that spawns a worker thread and expects
the redirected config directory must set `XDG_CONFIG_HOME` instead.

### Why it exists

Before `ScopedConfigDir` existed, `save_library` derived its
transaction-gate and local-data-lock paths from the **target library path**
rather than from the config directory. Unit tests therefore wrote to the
developer's real `~/.config/snp/` — mutating a real `audit.log`, a real
`usage.toml`, and the real `.transaction/` directory — and contended on one
process-wide lock file. That contention is what made
`test_serialization_matrix_makefile_leading_tabs_with_trailing_newline` fail
on a 30 s local-data-lock timeout.

With the paths derived from the config directory
(`src/library/persistence.rs:264-265`, `:233`) and the config directory
redirected per test, each test gets its own gate, its own journal directory,
and its own lock.

### Scope: tests and the `test-support` feature

**This is a test-only seam.** `#[cfg(any(test, feature = "test-support"))]`
appears on the struct and on both impls, so the type does not exist in a
release build and production binaries cannot be steered by it.

Integration tests under `tests/**` compile the crate **without** `cfg(test)`,
so `ScopedConfigDir` is unreachable from them *unless* they enable the
`test-support` feature. The crate root re-exports it as
`#[doc(hidden)] pub use utils::config::ScopedConfigDir;`
(`src/lib.rs:97`) under the same cfg, so an integration test that needs it
must be declared with `required-features = ["test-support"]`. In-tree,
`tests/identity_contract.rs` is the only such target (`Cargo.toml:84`); every
other integration test isolates by setting `XDG_CONFIG_HOME` instead.

The three in-crate unit-test callers need the audit log, usage counters, or
the local-data lock to land in a `TempDir` instead of the developer's real
config directory:

| Caller | What it isolates |
|--------|------------------|
| `src/library/tests.rs:16-37` | `TestDir` helper — holds the guard for the lifetime of a `TempDir` |
| `src/logging.rs:623-626` | The audit-log sentinel test (the log path resolves from the config dir) |
| `src/commands/run_cmd.rs:435-439` | `isolated_config_dir()` — audit entry + usage counter + local-data lock |

All three keep the guard alive for the duration of the test, and none of them
uses `XDG_CONFIG_HOME`, precisely so the redirect cannot leak into the
parallel lib tests.

## Key Files

| File | Role |
|------|------|
| `src/utils/config.rs` | Config-dir resolution, per-file path helpers, macOS migration, `ScopedConfigDir` |
| `src/config/mod.rs:24` | Re-exports `derive_sync_state_dir` and `get_sync_config_path` at `crate::config::*` |
| `src/local_data.rs:398-400` | `transaction_dir()` — `<config>/.transaction` |
| `src/library/persistence.rs:264-269` | Consumer that derives gate + lock paths from the config dir |
| `src/library/manager.rs:44-53` | Triggers the macOS migration; derives `libraries/`, `premade/`, `libraries.toml` |
| `src/library/tests.rs:16-37` | In-crate test helper holding a `ScopedConfigDir` guard |

## Invariants

- **One source of truth for the config directory.** Every path inside
  `~/.config/snp/` derives from `get_config_dir()`, never from the path of the
  file being written.
- **No per-platform branch.** macOS and Windows share the `~/.config` layout.
  The macOS-specific code is the *legacy* migration only.
- **The migration is one-way and resumable.** Legacy entries are moved, not
  copied; the legacy path is still reported while it exists so an interrupted
  cross-device move resumes.
- **Config directory permissions are `0o700` on Unix** — enforced on create
  *and* tightened on an existing directory.
- **`derive_sync_state_dir()` is the auto-sync state root.** The pending
  marker, worker lock, execution lock, and status file live there.
- **The test override is compiled out of release builds.** It is
  `#[cfg(any(test, feature = "test-support"))]`, and release builds enable
  neither `cfg(test)` nor the `test-support` feature.

## Gotchas

- **`get_config_dir()` does not create anything.** A caller that writes
  inside the config directory must call `ensure_config_dir()` first (or rely
  on one of the defensive call sites).
- **The empty-`XDG_CONFIG_HOME` case falls through.** `XDG_CONFIG_HOME=""`
  is filtered out (`src/utils/config.rs:71`), so the result is
  `~/.config/snp`, not `/snp`.
- **The no-home fallback is `./.config/snp`,** not `./snp` — the `.config`
  segment is joined before the final `snp`.
- **Do not derive a lock path from a library path.** The local-data lock must
  be the config-directory one, or two libraries stop excluding each other.
- **`ScopedConfigDir` is not available to integration tests by default.**
  `tests/**` compiles the crate without `cfg(test)`; use `XDG_CONFIG_HOME`
  there. The one exception is a target declared with
  `required-features = ["test-support"]`, which unlocks the `#[doc(hidden)]`
  re-export at `src/lib.rs:97` — `tests/identity_contract.rs` is the only
  in-tree user.
- **`ScopedConfigDir` does not cross thread boundaries.** Spawned worker
  threads see the real config directory.
- **`migrate_macos_config_dir()` moves, it does not copy.** A destination
  that already exists triggers the copy fallback and then the source is
  removed — the legacy entry is not left behind as a duplicate.
