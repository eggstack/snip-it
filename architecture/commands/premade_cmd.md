# premade_cmd — Browse/Download Premade Libraries

[← Back to Overview](../overview.md)

## Overview

`src/commands/premade_cmd.rs` (269 lines) is the client for the
server's premade-library catalog (`ListPremadeLibraries`,
`GetPremadeLibrary`, `SearchPremadeLibraries` RPCs plus the
`run_premade_sync` batch helper). Downloads land in the local premade
cache (`~/.config/snp/premade/`), never in live snippet libraries.

## CLI surface

`snp premade` (alias `p`), `main.rs:210-225,738-751`:

| Form | Handler | Notes |
|------|---------|-------|
| `premade list` (alias `l`) | `run_list(rt)` (`:7`) | `filename: description` lines |
| `premade get <name>` / `get all` | `run_get(name, all, rt)` (`:44`) | `all` skips cached; bare/missing name → usage error |
| `premade sync` (alias `s`) | `run_sync(rt)` (`:145`) | `run_premade_sync` batch download of missing |
| `premade search <query>` (alias `se`) | `run_search` (`:158`) | `filename: desc (N snippets) [tags]` |
| `premade update <name>` (alias `u`) | `run_update` (`:205`) | Diff + re-download, up-to-date short-circuit |

Every handler requires the global Tokio runtime and a configured
sync (`enabled` + client creation).

## Flow / steps

1. `get_sync_settings`; `!enabled` → stderr hint + `Ok` (exit 0,
   consistent with `sync run`).
2. `runtime.block_on(SyncClient::create(settings))`; failure →
   `runtime_error("Failed to create sync client")`.
3. Per command: `list_premade_libraries` / `get_premade_library(name)` /
   `search_premade_libraries(query)` / `run_premade_sync`, each with a
   specific `runtime_error` wrap naming the RPC.
4. `run_get(all)`: enumerate catalog, skip `premade_exists` entries,
   download + `save_premade_library`, per-item `+ name → path` /
   `✗ name: err` lines; empty delta → `All premade libraries already
   downloaded.` Single-get validates the name (`all` routes to batch;
   missing/`all`-as-name prints usage and errors).
5. `run_update`: read cached file (missing → empty baseline;
   `NotFound` tolerated, other I/O errors propagate), fetch remote,
   byte-compare → `already up to date` or `+added / -removed` line
   counts, then `save_premade_library` + path line.

## Mutation vs read-only

Cache-writing, snippet-safe: writes only `premade/*.toml` (plus reads
of `sync.toml` via settings). No transaction gate, no local-data lock
(the premade cache is outside the journaled dataset), no library
index changes.

## Auto-sync trigger

None. No `notify_mutation` — cached premade content is inert until a
user imports it (import then notifies `Import/Import`).

## Error / exit mapping

`SnipResult<()>`: disabled sync is exit-0 guidance; client/RPC/save
failures are `runtime_error` with the operation named (exit 1).
Single-get usage violations error explicitly. Batch mode never fails
fast — per-item failures print inline and the command still succeeds.

## Key invariants

- Premade cache is display/download-only; promoting content into a
  live library is an explicit import, never a side effect here.
- `all` never re-downloads cached entries (bandwidth + idempotence).
- Update diffs are line-set counts (added/removed), not a semantic
  merge — review before importing.
- Server catalog RPCs are read-only; `run_premade_sync` downloads
  missing entries only.

## File / line references

- Handlers: `src/commands/premade_cmd.rs:7,44,145,158,205`
- Cache: `LibraryManager::{premade_exists,save_premade_library,get_premade_dir}`
- RPCs: `snip-proto/proto/sync.proto`; dispatch: `src/main.rs:738-751`
