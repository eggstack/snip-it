# library_cmd — Library Management Subcommands

[← Back to Overview](../overview.md)

## Overview

`src/commands/library_cmd.rs` (155 lines) manages snippet libraries
through `LibraryManager`: list, create, delete, set-primary, and show.
Libraries are the unit of sync — each `<name>.toml` under
`<config>/libraries/` is one syncable collection, tracked in
`<config>/libraries.toml`.

It does **not** sync anything, and it never touches snippet *content*
except to create or delete an empty library file. Library **linkage** to a
server and `last_sync` watermarks are managed by the sync engine
(`src/sync_commands.rs`), not here.

Related: [../library.md](../library.md) (the library model),
[sync_cmd.md](sync_cmd.md) (server linkage + recovery),
[premade_cmd.md](premade_cmd.md) (the premade cache, a different
directory).

## CLI surface

`snp library` (alias `lib`) — `src/main.rs:147-151`; the subcommand enum
is `LibraryCommands` (`src/main.rs:213-233`).

| Form | Alias | Handler | Arguments |
|------|-------|---------|-----------|
| `library list` | `l` | `run_list()` (`library_cmd.rs:6`) | none |
| `library create <name>` | `c` | `run_create(name)` (`library_cmd.rs:24`) | positional `name: String` |
| `library delete <name>` | `d` | `run_delete(name, force)` (`library_cmd.rs:39`) | positional `name: String`, `-f/--force` |
| `library set-primary <name>` | `p` | `run_set_primary(name)` (`library_cmd.rs:74`) | positional `name: String` |
| `library show [name]` | `s` | `run_show(name)` (`library_cmd.rs:82`) | optional positional `name: Option<String>` |

Unlike `sync`, `register`, and `cron`, the `library` group has **no**
`*Args` struct — the clap variants are declared inline in
`src/main.rs` and destructured at dispatch (`src/main.rs:767-775`).

## Flow / steps

Every handler begins with `LibraryManager::new()`. That constructor is
**not** side-effect free — see [Fail-closed config load](#fail-closed-config-load).

### `run_list` (`library_cmd.rs:6`)

`LibraryManager::new()` → `mgr.list_libraries()` → prints
`No libraries found.` when empty, otherwise `Libraries:` followed by
`   (primary)` for the primary library and `  ` for
the rest (`:10-19`).

### `run_create` (`library_cmd.rs:24`)

`mgr.create_library(name)` → notify → `Created library '<name>' at <path>`.

`LibraryManager::create_library` (`src/library/manager.rs:376-436`):

1. `gate_mutation()` → `transaction::gate_mutation_on_interrupted_transactions`
   (`:364`, `:636-642`).
2. `validate_library_name(filename)` (`:366-367`) — see
   [Reserved names](#reserved-names).
3. `ensure_library_mode()` (`:376`) — migrates a legacy single-file
   `snippets.toml` store **before** the libraries directory exists; doing
   it later would orphan the legacy file (the comment at `:369-375`
   explains the ordering).
4. `acquire_local_data_lock()` (`:378`).
5. `init_libraries_dir()` (`:380`).
6. Reject if the file already exists (`:385-390`).
7. Reject a case-insensitive duplicate in the index (`:392-401`).
8. Write a default empty library file (`:403-410`).
9. `mutation_barrier("library-create-after-file-before-index")` (`:412`) —
   a test-only crash-injection point between the file write and the index
   update.
10. Push the `LibraryMeta`; the **first** library created is automatically
    marked primary (`:414-417`).
11. `bump_generation()` + `save_config()` (`:419-420`).

### `run_delete` (`library_cmd.rs:39`)

1. **Confirmation.** Without `--force`: a non-terminal stdin is a hard
   error — `Refusing to delete library '<name>' in non-interactive mode.
   Use --force to override.` plus `runtime_error("Non-interactive
   delete")` (`:42-51`). On a terminal, the prompt is
   `Are you sure you want to delete library '<name>'? [y/N]: `; any answer
   other than exactly `y` (lowercased) prints `Cancelled.` and returns
   `Ok(())` — exit 0 (`:52-59`).
2. `mgr.delete_library(name)` (`:61`).
3. Notify → `Deleted library '<name>'` (`:64-69`).

`LibraryManager::delete_library` (`src/library/manager.rs:442-509`):

- gate → lock (`:430-431`);
- snapshot the config for rollback (`:432`);
- look up the library; unknown name → `runtime_error("Library not
  found")` (`:433-436`);
- **remove the entry from the index and save the config FIRST**, then
  delete the file (`:445-469`). The comment at `:440-444` states the
  rationale: if the process crashes after the config save but before the
  file delete, the orphan file is recoverable; the reverse order would
  leave a stale config reference;
- if the deleted library was primary, promote another one — preferring a
  server-linked library when the deleted one was server-linked (`:447-466`);
- `mutation_barrier("library-delete-after-index-before-file")` (`:471`);
- if the file delete fails, restore the saved config and re-save it
  (`:473-493`).

### `run_set_primary` (`library_cmd.rs:74`)

`mgr.set_primary(name)` → `Set '<name>' as primary library`. **No
notification** — the primary pointer is not snippet content.

`LibraryManager::set_primary` (`src/library/manager.rs:512-533`): gate →
lock → the library must exist (else `runtime_error("Library not found")`)
→ set `is_primary` on exactly one entry → `bump_generation()` +
`save_config()`.

### `run_show` (`library_cmd.rs:82`)

With a name: prints `Library: `, `  ID: <library_id>` (or
`{not linked}` when the ID is empty), `  Primary: <bool>`, and
`  Last sync: <YYYY-MM-DD HH:MM>` when `last_sync` is set
(`:85-97`). An unknown name prints `Library '<name>' not found` and
returns `runtime_error("Library not found")` (`:98-104`).

Without a name: prints `Libraries:` and one line per library with
`(primary)` and `[linked]` markers (`:105-116`).

The `{not linked}` fallback comes from the local `StringExt::if_empty`
trait (`library_cmd.rs:120-132`).

## Reserved names

`validate_library_name` (`src/library/model.rs:179-225`) rejects:

| Rule | Reason |
|------|--------|
| Empty name | — |
| More than 50 characters | Bounded index entry |
| Path separators (`/`, `\`) | Prevents escaping the libraries directory |
| NUL bytes | — |
| `.` / `..` | Path traversal |
| A trailing `.toml` | The extension is added automatically |
| **`all`** | Reserved as the cross-library scope keyword |

The `all` rule was added in commit `1920075`:

```rust
// src/library/model.rs:213-223
// `all` is the cross-library scope keyword owned by
// `LibraryScope::from_filter_arg` (src/selector.rs). A library actually
// named `all` could never be addressed exclusively by the resolver
// surfaces, and `--library all` silently returned the wrong scope on them
// instead of an error.
if name == "all" {
    return Err((
        "Invalid library name",
        "Library name cannot be 'all' (reserved as the cross-library scope keyword). Use 'snp list --library all' to target every library.",
    ));
}
```

So `snp library create all` now fails. The `all` scope itself is a
*selector* concept (`src/selector.rs`), not a library name.

## Fail-closed config load

`LibraryManager::new()` (`src/library/manager.rs:43-94`) is the single
constructor used by all five handlers, and it does two things a casual
reader would not expect from a "read" path:

1. **macOS config-dir migration** (`:43-45`):
   `utils::config::migrate_macos_config_dir()` moves a legacy
   `~/Library/Application Support/...` config directory into the XDG
   location (`src/utils/config.rs:166-210`). A failure is a warning only.
2. **Fail-closed `libraries.toml` parse** (`:53-84`): if
   `<config>/libraries.toml` exists but does not parse, the file is
   **copied to `libraries.toml.corrupt`** (`:60-75`) and the constructor
   returns `SnipError::toml_error("parse libraries config: …")`. It never
   synthesizes a writable empty config.

A *missing* `libraries.toml` yields `LibraryConfig::default()` — that is
the only case where a default is synthesized (`:82-84`).

Consequence: `snp library list` and `snp library show` are **not**
strictly read-only. They can move the config directory (macOS) and can
create `libraries.toml.corrupt`. This is the same class of side effect
that `snp status` has (see [status_cmd.md](status_cmd.md)).

## Mutation vs read-only

| Subcommand | Class | Writes |
|------------|-------|--------|
| `library list` | read-only (modulo the constructor side effects above) | none beyond migration/backup |
| `library show` | read-only (same caveat) | none beyond migration/backup |
| `library create` | mutating | `libraries/<name>.toml`, `libraries.toml` (index + generation) |
| `library delete` | mutating | `libraries.toml` first, then `libraries/<name>.toml` |
| `library set-primary` | mutating (index only) | `libraries.toml` (primary flag + generation) |

`create` and `delete` call `gate_mutation()` through
`LibraryManager`, so both run
`gate_mutation_on_interrupted_transactions(config_dir, transaction_dir)`
(`src/library/manager.rs:666-672`). `set-primary` also calls
`gate_mutation()` (`:500`) even though it only rewrites the index.

## Tokio runtime

None. `snp library` never touches the global `RUNTIME`
(`src/main.rs:29-35`) — every handler is synchronous.

Startup policy (`src/main.rs:987-1004`):

- `library list` / `library show` → `StartupRecoveryPolicy::SuppressReadOnly`
  with `StartupServices::Minimal` (no file logging) — `src/main.rs:936-942`;
- `library create` / `library delete` / `library set-primary` →
  `StartupRecoveryPolicy::Allow` with `StartupServices::Logging` —
  `src/main.rs:998-1004`.

## Auto-sync trigger

| Subcommand | Notification |
|------------|--------------|
| `library create` | `notify_mutation(MutationKind::LibraryChange, MutationOrigin::User)` (`library_cmd.rs:29-32`) |
| `library delete` | `notify_mutation(MutationKind::LibraryChange, MutationOrigin::User)` (`library_cmd.rs:64-67`) |
| `library set-primary` | none |
| `library list` / `library show` | none |

The notification is fired **after** the mutation succeeded, and its
result is reported via `auto_sync::report_notification_result`.

## Error / exit mapping

`SnipResult<()>` — every `Err` becomes `error: <message>` on stderr and
exit **1** (`src/main.rs:1063-1069`).

| Situation | Result |
|-----------|--------|
| `library create all` | `runtime_error("Invalid library name")` → exit 1 |
| `library create` on an existing name (exact or case-insensitive) | `runtime_error("Library already exists")` → exit 1 |
| `library delete <unknown>` | `runtime_error("Library not found")` → exit 1 |
| `library delete` non-interactive without `--force` | `runtime_error("Non-interactive delete")` → exit 1 |
| `library delete` answered `n` | exit 0 (`Cancelled.`) |
| `library show <unknown>` | `runtime_error("Library not found")` → exit 1 |
| `library set-primary <unknown>` | `runtime_error("Library not found")` → exit 1 |
| Malformed `libraries.toml` | `toml_error("parse libraries config")` → exit 1, plus `libraries.toml.corrupt` |

The "not found" cases are plain `runtime_error`s, so they exit **1**, not
the documented exit code 3 (`NOT_FOUND`) — no command on this path
returns `CliOutcome::NotFound`.

## Key Files

| Path | Role |
|------|------|
| `src/commands/library_cmd.rs` | The five handlers + the `if_empty` helper |
| `src/main.rs:213-233`, `:730-738` | Clap wiring and dispatch |
| `src/library/manager.rs` | `LibraryManager` — index, files, linkage, gate, lock |
| `src/library/model.rs:179-225` | `validate_library_name` (incl. the reserved `all`) |
| `src/library/persistence.rs` | `load_library` / `save_library` (TOML fidelity) |
| `src/selector.rs` | `LibraryScope::from_filter_arg` — owns the `all` scope keyword |
| `src/utils/config.rs:166-210` | `migrate_macos_config_dir` |
| `src/auto_sync/notification.rs` | `notify_mutation` / `report_notification_result` |

## Invariants

- **All libraries live under `<config>/libraries/`** — external paths are
  unsupported.
- **Malformed `libraries.toml` fails closed**: best-effort backup plus an
  error, never a synthetic writable empty config.
- **`all` is reserved** and cannot be used as a library name.
- **Deletion is refused non-interactively without `--force`** — scripts
  must opt in explicitly.
- **The index is saved before the file is deleted** so a crash leaves a
  recoverable orphan, never a stale index entry.
- **The mutation gate runs before every index or file write.**

## Gotchas

- **`list` and `show` are not strictly read-only** — the shared
  constructor can migrate the config directory and can create
  `libraries.toml.corrupt`.
- **Library names are case-insensitively unique**, but the *file* name
  preserves the original casing (`<name>.toml`).
- **The first library created becomes primary automatically.**
- **`set-primary` bumps the generation counter** even though it changes
  no snippet content.
- **Deleting the primary library promotes a different one**, preferring a
  server-linked library when the deleted library was server-linked.
- **Missing-library recovery is not this command's job** — it is driven
  by the sync engine through the `<library>.sync_recovery` marker (see
  [sync_cmd.md](sync_cmd.md)).
