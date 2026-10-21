# Utilities

[← Back to Overview](overview.md)

## Overview

`src/utils/` is the shared-utility subtree of the `snip-it` binary crate: a
single layer of leaf modules with no knowledge of CLI commands, the TUI, sync
transport, or the library model. Every module here is **crate-internal** — the
crate root declares `pub(crate) mod utils;` (`src/lib.rs:70`), so nothing in
`src/utils/` is part of the supported public API even though each submodule is
declared `pub mod` inside `src/utils/mod.rs`.

This inventory documents *what each module is for and who calls it*. It does
**not** duplicate the per-module contracts — each module has its own deep-dive
document linked from the index table below, and those documents own the
behavioural detail (scanner semantics, resolution order, escape rules).

**Not covered here:** the transaction / local-data / execution lock hierarchy
(which uses `utils::process` but lives elsewhere), the library persistence
model, the sync protocol, and the atomic-write durability contract beyond a
pointer.

## Module Inventory

**Directory**: `src/utils/` (declared in `src/utils/mod.rs`, 23 lines)

| Module | File | Lines | Role | Visibility | Deep dive |
|--------|------|-------|------|------------|-----------|
| `atomic` | `atomic.rs` | 704 | Durability-classed atomic file writes | `pub mod` | [utils/atomic.md](utils/atomic.md) |
| `config` | `config.rs` | 303 | Config-directory resolution, path helpers, macOS migration | `pub mod` | [utils/config.md](utils/config.md) |
| `process` | `process.rs` | 166 | Process-liveness checks, ownership-verified lock-file removal | `pub mod` | [utils/process.md](utils/process.md) |
| `redact` | `redact.rs` | 72 | Best-effort secret redaction for display/log paths | `pub mod` | [utils/redact.md](utils/redact.md) |
| `shell_keywords` | `shell_keywords.rs` | 198 | 190 shell command names for syntax highlighting | `pub mod` | [utils/shell_keywords.md](utils/shell_keywords.md) |
| `tempfile_guard` | `tempfile_guard.rs` | 38 | RAII cleanup for orphaned temp files | `pub mod` | [utils/tempfile_guard.md](utils/tempfile_guard.md) |
| `toml_helpers` | `toml_helpers.rs` | 456 | TOML token scanner for legacy escape repair | `pub mod` | [utils/toml_helpers.md](utils/toml_helpers.md) |
| `variables` | `variables.rs` | 1947 | Variable parsing, expansion, escape stripping | `pub mod` | [utils/variables.md](utils/variables.md) |
| *(root)* | `mod.rs` | 23 | Module declarations + variable re-exports | — | — |

`src/utils/mod.rs:11-18` declares all eight submodules as `pub mod`; the
submodules themselves are only reachable through the crate-private `utils`
module.

### Re-exports

`src/utils/mod.rs:20-23` re-exports five items from `variables` at the
`utils` root, which is why callers write `crate::utils::parse_variables(...)`
rather than `crate::utils::variables::parse_variables(...)`:

```rust
// src/utils/mod.rs:20-23
pub use variables::{
    expand_command, extract_variables_for_display, has_unmatched_angle_bracket, parse_variables,
    strip_escape_sequences,
};
```

Everything else in the subtree is reached through its own module path
(`crate::utils::config::get_config_dir`, `crate::utils::atomic::atomic_replace`,
…). `VariableAssignments` and `parse_variables_diagnostics` are **not**
re-exported — consumers import them from `crate::utils::variables::` directly.

At the crate root, only the `atomic` module is surfaced as supported API:

```rust
// src/lib.rs:78-80
pub use utils::atomic::{
    AtomicWriteOptions, AtomicWriteReport, Durability, atomic_replace, write_private_atomic,
};
```

## Layer Usage

Which layers reach into `src/utils/`. No module here is used by `snip-sync`
or `snip-proto` — the server crate is a separate workspace member with its own
dependencies.

| Module | Consumer layers (representative call sites) |
|--------|----------------------------------------------|
| `config` | library (`src/library/manager.rs:15`), commands (`src/commands/backup_cmd.rs:13`, `restore_cmd.rs:7`, `repair_cmd.rs:874`, `doctor_cmd.rs:491`, `validate_cmd.rs:540`), diagnostics (`src/status_snapshot.rs:146`), UI (`src/ui/theme.rs:20`), usage (`src/usage.rs:21`), logging (`src/logging.rs:72`) |
| `variables` | commands (`src/commands/get_cmd.rs:11`, `src/commands/mod.rs:318`, `src/commands/import_cmd.rs:219`, `src/commands/doctor_cmd.rs:312`), UI (`src/ui/mod.rs:36-38`, `src/ui/variables.rs`) |
| `toml_helpers` | commands (`src/commands/mod.rs:37`, `src/commands/pet_analysis.rs:5`), library (`src/library/persistence.rs:10`, `src/library/manager.rs:16`), config (`src/config/sync_settings.rs:16`) |
| `atomic` | library (`src/library/persistence.rs`, `src/library/manager.rs`), config (`src/config/sync_settings.rs`), commands (`src/commands/{mod,select_cmd,import_cmd,restore_cmd,backup_archive}.rs`), auto-sync (`src/auto_sync/{pending,status}.rs`), transactions (`src/transaction.rs`), sync merge (`src/sync_commands.rs`), migration (`src/migration.rs`), UI (`src/ui/theme.rs`), usage (`src/usage.rs`) |
| `process` | locks only (`src/transaction.rs`, `src/local_data.rs`, `src/auto_sync/execution_lock.rs`) |
| `redact` | logging (`src/logging.rs:314`), errors (`src/error.rs:190`), auto-sync status (`src/auto_sync/status.rs:27`) |
| `shell_keywords` | UI only (`src/ui/highlight.rs:8`) |
| `tempfile_guard` | `utils::atomic` only (`src/utils/atomic.rs:376`) |

## Module Summaries

### Variables

**File**: `src/utils/variables.rs` (1947 lines — parser, Pet-choice support,
diagnostics, `VariableAssignments`, and the unit-test tail that starts at
`src/utils/variables.rs:794`).

Full reference: [utils/variables.md](utils/variables.md).

| Function | Purpose |
|----------|---------|
| `parse_variables(command)` | Extract `Variable` structs, in source order |
| `parse_variables_diagnostics(command)` | Variables plus `VariableDiagnostic` warnings (`choice.*`, `var.duplicate`) |
| `extract_variables_for_display(command)` | Format variables for TUI display |
| `expand_command(command, values)` | Substitute `<var>` tokens with supplied values |
| `strip_escape_sequences(command)` | Convert `\<` → `<`, `\>` → `>`, `\\` → `\` |
| `has_unmatched_angle_bracket(command)` | Detect an unclosed `<` (nesting- and escape-aware) |
| `VariableAssignments::parse_arg` / `from_pairs` | Parse `--var key=value` CLI assignments |

### TOML Helpers

**File**: `src/utils/toml_helpers.rs` (456 lines — a hand-written TOML token
scanner at `:34-172` plus the test tail).

Full reference: [utils/toml_helpers.md](utils/toml_helpers.md).

| Function | Direction | Trigger |
|----------|-----------|---------|
| `fix_invalid_toml_escapes(toml_str)` | **load path only** (`pub`) | `\<` or `\>` |
| `quote_strings_containing_backslashes(toml_str)` | hand-written TOML only (`pub(crate)`, `#[allow(dead_code)]`) | any `\` |

Triple-quoted regions are always passed through verbatim. Neither helper may
run on `toml::to_string_pretty` output — see
[persistence.md](persistence.md) and
`.skills/persistence-and-toml-fidelity.md`.

### Shell Keywords

**File**: `src/utils/shell_keywords.rs` (198 lines — pure data, no tests).

Full reference: [utils/shell_keywords.md](utils/shell_keywords.md).

`SHELL_KEYWORDS: &[&str]` holds exactly **190** command names
(`src/utils/shell_keywords.rs:4-195`); `SHELL_KEYWORDS_SET` is the derived
`LazyLock<HashSet<&'static str>>` built at `src/utils/shell_keywords.rs:197-198`
for O(1) membership probes. The only consumer is the TUI syntax highlighter
(`src/ui/highlight.rs:8`, probe at `src/ui/highlight.rs:62`).

### Config Paths

**File**: `src/utils/config.rs` (303 lines — path resolution, macOS migration,
and the `#[cfg(any(test, feature = "test-support"))]`-only `ScopedConfigDir`
override).

Full reference: [utils/config.md](utils/config.md).

| Function | Returns |
|----------|---------|
| `get_config_dir()` | `$XDG_CONFIG_HOME/snp`, else `~/.config/snp` (`:63-82`) |
| `ensure_config_dir()` | Same, created with `0o700` on Unix (`:91-116`) |
| `get_config_path(filename)` | `get_config_dir().join(filename)` (`:212-214`) |
| `get_snippets_path()` | `get_config_path("snippets.toml")` (`:216-218`) |
| `get_sync_config_path()` | `get_config_path("sync.toml")` (`:220-222`) |
| `derive_sync_state_dir()` | Parent of the sync config path (`:227-232`) |
| `get_legacy_macos_config_dir()` | Old macOS path if it exists (`:123-134`) |
| `migrate_macos_config_dir()` | Move entries from the old path to the new one (`:166-210`) |
| `ScopedConfigDir::new(dir)` | `#[cfg(any(test, feature = "test-support"))]` thread-local config-dir override (`:35-57`) |

### Atomic Writes, Temp Files, Process Liveness, Redaction

- **Atomic writes** (`atomic.rs`): `write_private_atomic` for simple TOML
  saves, `atomic_replace` with `Durability` classes for the rest —
  see [utils/atomic.md](utils/atomic.md).
- **Temp-file cleanup** (`tempfile_guard.rs`): RAII guard consumed by
  `persist()` after a successful rename — see
  [utils/tempfile_guard.md](utils/tempfile_guard.md).
- **Process checks** (`process.rs`): shared `is_process_alive` (`kill(pid, 0)`;
  ESRCH-dead / EPERM-alive) and ownership-verified lock-file removal used by
  the transaction, local-data, and execution locks — see
  [utils/process.md](utils/process.md).
- **Secret redaction** (`redact.rs`): best-effort `redact_secrets` for
  logs/errors/status; backups use a *separate*, line-oriented
  `redact_sync_config` in `src/commands/backup_archive.rs:358` — see
  [utils/redact.md](utils/redact.md).

## Key Files

| File | Role |
|------|------|
| `src/utils/mod.rs` | Submodule declarations and the five `variables` re-exports |
| `src/utils/atomic.rs` | Durability-classed atomic write primitives |
| `src/utils/config.rs` | Config-dir resolution, per-file path helpers, macOS migration, `ScopedConfigDir` |
| `src/utils/process.rs` | Process liveness and ownership-verified lock-file removal |
| `src/utils/redact.rs` | Shared secret-redaction regexes |
| `src/utils/shell_keywords.rs` | 190 shell command names + derived hash set |
| `src/utils/tempfile_guard.rs` | RAII deletion of orphaned temp files |
| `src/utils/toml_helpers.rs` | Hand-written TOML token scanner; legacy escape repair |
| `src/utils/variables.rs` | Variable parsing, expansion, escape stripping |

## Invariants

- **The subtree is crate-internal.** `pub(crate) mod utils;` (`src/lib.rs:70`);
  only `utils::atomic` items are re-exported at the crate root.
- **No module in `src/utils/` may depend on a higher layer.** They take
  primitives (`&str`, `&Path`, `&[u8]`) and return primitives; the single
  exception is `redact.rs`, which returns an owned `String`.
- **`utils::config` is the single source of config-directory truth.** Every
  path inside `~/.config/snp/` — including the transaction gate, the
  local-data lock, and the auto-sync state — derives from `get_config_dir()`,
  never from the path of the file being written.
- **The save path never post-processes TOML.** `utils::toml_helpers` runs on
  the load path only.
- **`utils::process` is the single implementation of lock-owner liveness**, so
  the transaction, local-data, and execution locks cannot diverge.
- **Credentials never enter a log or error string.** `utils::redact` is the
  one shared definition; `status::redact_secrets` is a re-export, not a fork.

## Gotchas

- **Adding a submodule means editing `src/utils/mod.rs` twice** — the `pub mod`
  declaration and (if it belongs at the `utils` root) the re-export block.
  Nothing else in the crate re-exports `utils` wholesale.
- **`ScopedConfigDir` does not exist in a release build.** It is
  `#[cfg(any(test, feature = "test-support"))]` on the struct and both impls,
  so it is unreachable from production binaries. Integration tests reach it
  only through the `#[doc(hidden)] pub use` at `src/lib.rs:97`, which requires
  the `test-support` feature — `tests/identity_contract.rs` is the one
  in-tree integration test that enables it (declared with
  `required-features = ["test-support"]` in `Cargo.toml:84`).
- **A thread-local override does not follow spawned threads.** A unit test
  that spawns a worker and expects the redirected config directory must set
  `XDG_CONFIG_HOME` instead.
- **`SHELL_KEYWORDS` is data, but it is behaviour.** Appending a name changes
  the TUI's syntax highlighting for every snippet that contains it.
- **`quote_strings_containing_backslashes` is `#[allow(dead_code)]`.** Its
  presence is deliberate — it is the trap that the save path used to fall
  into. Do not wire it back into serialization.
- **Line counts in the index table are verified against the current tree**
  and drift whenever a module is edited; re-check them before quoting them
  elsewhere.
