# premade_cmd — Browse and Download Premade Libraries

[← Back to Overview](../overview.md)

## Overview

`src/commands/premade_cmd.rs` (288 lines) is the client for the server's
premade-library catalog. It lists the catalog, downloads one or all
premade libraries into the local premade cache
(`<config>/premade/*.toml`), searches the catalog, and refreshes a single
cached entry.

It does **not** promote premade content into a live snippet library —
that is an explicit `snp import` step. It never writes `libraries.toml`,
never touches live libraries, and never triggers auto-sync.

Related: [sync_cmd.md](sync_cmd.md), [register_cmd.md](register_cmd.md),
[../library.md](../library.md).

## CLI surface

`snp premade` (alias `p`) — `src/main.rs:126-131`; the subcommand enum is
`PremadeCommands` (`src/main.rs:209-225`).

| Form | Alias | Handler | Server RPC |
|------|-------|---------|-----------|
| `premade list` | `l` | `run_list(&RUNTIME)` (`premade_cmd.rs:31`) | `ListPremadeLibraries` |
| `premade get <name>` | — | `run_get(name, all, &RUNTIME)` (`premade_cmd.rs:67`) | `GetPremadeLibrary` (or `ListPremadeLibraries` for `all`) |
| `premade sync` | `s` | `run_sync(&RUNTIME)` (`premade_cmd.rs:167`) | `ListPremadeLibraries` + `GetPremadeLibrary` (via `sync_commands::run_premade_sync`) |
| `premade search <query>` | `se` | `run_search(query, &RUNTIME)` (`premade_cmd.rs:179`) | `SearchPremadeLibraries` |
| `premade update <name>` | `u` | `run_update(name, &RUNTIME)` (`premade_cmd.rs:225`) | `GetPremadeLibrary` |

`premade get` takes an optional positional `name`; the literal `all`
selects the batch path (the check is at `src/main.rs:742`; the optional
positional is declared at `src/main.rs:215`).

## The credential guard

Every subcommand starts with the same guard:

```rust
// src/commands/premade_cmd.rs:18-28
fn require_sync_credentials(sync_settings: &crate::config::SyncSettings) -> bool {
    if !sync_settings.enabled {
        eprintln!("Sync is not enabled. Configure sync settings first.");
        return false;
    }
    if sync_settings.api_key.is_empty() {
        eprintln!("Sync is enabled but no API key is configured. Run 'snp register --force'.");
        return false;
    }
    true
}
```

**Both halves are required.** `sync.toml` deserializes `api_key` with
`#[serde(default)]`, so a headerless legacy config parses with
`enabled = true` and an empty key. Checking only `enabled` let `list`,
`get`, `search`, and `update` reach `add_api_key_metadata` with an empty
key — which tripped its `debug_assert!` in debug builds and otherwise sent
an unauthenticated request the server rejects with a bare 401 instead of
the actionable message printed here. The assertion has since been removed
(`src/sync.rs:291-299`).

The guard returns `false` → the handler prints the message and returns
`Ok(())` — exit 0, not an error.

As a second line of defence, every authenticated `SyncClient` method also
calls `require_api_key()` (`src/sync.rs:800-808`), which fails fast with a
typed error naming `snp register --force`:

| Method | Guard call |
|--------|-----------|
| `list_libraries` | `src/sync.rs:955` |
| `create_library` | `src/sync.rs:1016` |
| `list_premade_libraries` | `src/sync.rs:1051` |
| `get_premade_library` | `src/sync.rs:1083` |
| `search_premade_libraries` | `src/sync.rs:1127` |

## Flow / steps

### `run_list` (`premade_cmd.rs:31`)

`get_sync_settings()` → guard → `SyncClient::create` (failure →
`runtime_error("Failed to create sync client")`) →
`list_premade_libraries` (failure → `runtime_error("Failed to list premade
libraries")`). Prints `No premade libraries available.` or
`Available premade libraries:` plus `  : <description>` lines.

### `run_get` (`premade_cmd.rs:67`)

**Batch path (`all`)** (`:87-137`): list the catalog, skip entries where
`mgr.premade_exists(&lib.filename)` is true, download the rest with
`get_premade_library`, and persist each with `save_premade_library`.
Per-item results print as `  + <name> → <path>` or `  ✗ <name>: <error>`;
an empty delta prints `All premade libraries already downloaded.`
Per-item failures do **not** fail the command.

**Single path** (`:139-163`): the name must be present and must not be
`all`; otherwise the handler prints a usage hint and returns
`runtime_error("Invalid usage")`. It then fetches the content, saves it,
and prints `Downloaded premade library '<name>' to <path>`.

### `run_sync` (`premade_cmd.rs:167`)

Delegates to `sync_commands::run_premade_sync(&settings, runtime)`
(`src/sync_commands.rs:503-584`), which:

- returns `Ok(())` immediately when `!enabled || api_key.is_empty()`
  (`:507-509`);
- creates the client (`ConnectFailed` on failure, `:511-519`);
- lists the catalog — a listing failure is logged as a warning and
  returns `Ok(())` (`:521-527`);
- downloads only the missing entries (`premade_exists` skip, `:545-563`);
- prints `Premade libraries:` plus `  + <name> → <path>` / `  ✗ <name>: <err>`;
- and returns `sync_failure(PremadePartialFailure)` if **any** item
  failed (`:575-580`).

Unlike `premade get all`, `premade sync` **does** fail the command on a
partial failure.

### `run_search` (`premade_cmd.rs:179`)

`search_premade_libraries(&query)` → prints
`Found N premade libraries matching '<query>':` plus
`  : <description> (<n> snippets)[ <tag, tag>]` lines. Tags are
appended only when non-empty (`:209-219`).

### `run_update` (`premade_cmd.rs:225`)

1. Read the cached copy, if any. The cache path is
   `mgr.get_premade_dir().join(format!("{name}.toml"))` (`:243-244`).
   A `NotFound` read error is tolerated and yields an empty baseline
   (`:247`); any other I/O error becomes `io_error("read premade cache")`
   (`:249-253`).
2. Fetch the remote copy (`:260-267`).
3. If the bytes are identical, print `Premade library '<name>' is already
   up to date.` and stop (`:269-272`).
4. Otherwise print a **line-set** diff: `+added / -removed` counts
   computed by set difference over `str::lines()` (`:278-283`), or
   `<n> new lines` when there was no local copy (`:274-277`).
5. `save_premade_library(&name, &new_content)` and print
   `Updated '<name>' → <path>` (`:285-286`).

The diff is a line-set count, not a semantic merge — review before
importing.

## The TOCTOU-safe canonical-path write

`LibraryManager::save_premade_library` (`src/library/manager.rs:717-751`)
is the only write path for the premade cache, and it is where the
path-traversal defence lives:

```rust
pub fn save_premade_library(&self, filename: &str, content: &str) -> SnipResult<PathBuf> {
    let _lock = self.acquire_local_data_lock()?;
    self.init_premade_dir()?;

    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename.contains('\0')
        || filename.contains("..")
    {
        return Err(SnipError::runtime_error(
            "Invalid premade library filename",
            Some(filename),
        ));
    }

    let path = self.premade_dir.join(format!("{filename}.toml"));

    let canonical_premade = self.premade_dir.canonicalize().map_err(|e| {
        SnipError::io_error("resolve premade directory", self.premade_dir.clone(), e)
    })?;
    let canonical_path = path
        .canonicalize()
        .unwrap_or_else(|_| canonical_premade.join(format!("{filename}.toml")));
    if !canonical_path.starts_with(&canonical_premade) {
        return Err(SnipError::runtime_error(
            "Invalid premade library path",
            Some("Filename resolves outside premade directory"),
        ));
    }

    write_library_file(&path, content, filename)?;

    Ok(path)
}
```

Two independent checks:

1. **Lexical rejection** — the filename may not be empty and may not
   contain `/`, `\`, `\0`, or `..`.
2. **Canonical containment** — the *resolved* path (via
   `Path::canonicalize`, falling back to the canonicalised parent joined
   with the filename when the target does not exist yet) must still start
   with the canonicalised premade directory. This is the check that
   survives a symlink pointing outside the cache.

Note the asymmetry: `premade_exists` (`src/library/manager.rs:709-711`)
is a plain `Path::exists()` probe and does no canonicalisation — it is
only used to decide whether to download, never to authorise a write.

## Mutation vs read-only

| Subcommand | Class | Writes |
|------------|-------|--------|
| `premade list` | read-only (remote) | none |
| `premade search` | read-only (remote) | none |
| `premade get` | mutating (cache only) | `premade/<name>.toml` |
| `premade get all` | mutating (cache only) | `premade/<name>.toml` for missing entries |
| `premade sync` | mutating (cache only) | `premade/<name>.toml` for missing entries |
| `premade update` | mutating (cache only) | `premade/<name>.toml` |

The premade cache is outside the journaled snippet dataset, so there is
no transaction gate (`gate_mutation_on_interrupted_transactions`) and no
library-index write. `save_premade_library` does take the local-data
lock (`acquire_local_data_lock`, `src/library/manager.rs:718`).

## Tokio runtime

All five handlers take `&tokio::runtime::Runtime` and use
`runtime.block_on(...)` for the RPCs. `RUNTIME` is the global `LazyLock`
runtime in `src/main.rs:22-27`; dispatch is at `src/main.rs:739-752`.

Startup policy: `Commands::Premade { .. }` uses
`StartupRecoveryPolicy::Allow` and `StartupServices::Logging`
(`src/main.rs:960`).

## Auto-sync trigger

None. No `notify_mutation` — cached premade content is inert until a user
imports it, and the import path is what notifies.

## Error / exit mapping

`SnipResult<()>` — every `Err` becomes `error: <message>` on stderr and
exit **1** (`src/main.rs:1026-1032`).

| Situation | Result |
|-----------|--------|
| Sync disabled / no API key | exit 0, guidance on stderr |
| `SyncClient::create` failure | `runtime_error("Failed to create sync client")` → exit 1 |
| RPC failure | `runtime_error` naming the RPC → exit 1 |
| `premade get` with no name, or `get all` reached as a name | `runtime_error("Invalid usage")` → exit 1 |
| `premade get all` with per-item failures | exit 0; failures printed inline |
| `premade sync` with any per-item failure | `sync_failure(PremadePartialFailure)` → exit 1 |
| `premade update` cache read error (not `NotFound`) | `io_error("read premade cache")` → exit 1 |

There is no `CliOutcome` on these paths.

## Key Files

| Path | Role |
|------|------|
| `src/commands/premade_cmd.rs` | `require_sync_credentials` + the five handlers |
| `src/sync_commands.rs:503-584` | `run_premade_sync` — the batch download helper |
| `src/sync.rs` | `SyncClient` premade RPCs + `require_api_key` |
| `src/library/manager.rs:709-751` | `premade_exists` / `save_premade_library` (traversal + canonical containment) |
| `src/config/sync_settings.rs` | `get_sync_settings` (non-failing load) |
| `src/main.rs:209-225`, `:739-752` | Clap wiring and dispatch |

## Invariants

- **The premade cache is display/download-only.** Promoting content into a
  live library is an explicit import, never a side effect here.
- **Both `enabled` and a non-empty API key are required** before any
  request is attempted.
- **`all` never re-downloads cached entries** (bandwidth + idempotence).
- **The cache write is traversal-safe twice**: lexical rejection plus
  canonical containment.
- **Update diffs are line-set counts**, not a semantic merge.

## Gotchas

- **`premade get all` is the positional literal `all`**, while
  `premade get <name>` where `<name>` is missing is a usage error.
- **`premade get all` swallows per-item failures; `premade sync` does
  not.** The two batch paths differ in exit behavior.
- **`premade update` reports line-set differences, not a unified diff** —
  a line that moved counts as one added and one removed.
- **`get_sync_settings()` does not fail** — it falls back to defaults, so
  a corrupt `sync.toml` yields "Sync is not enabled" rather than a parse
  error.
