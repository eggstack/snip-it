# Library Scope

> **Evergreen contract.** How commands resolve which library (or libraries)
> they operate on.
>
> Read this before adding a `--library` flag. `src/selector.rs` owns the
> canonical scope model; `src/library::readonly_library_sources` is the
> read-only path and **does** understand `"all"`.

---

## Three resolution paths (do not mix them)

| Path | Entry point | Understands `"all"`? | Used by |
|------|-------------|----------------------|----------|
| Selector path | `selector::LibraryScope::from_filter_arg` / `from_owned_arg` (`src/selector.rs:65-71`) | **Yes** → `LibraryScope::AllLibraries` | `snp get` (via `exact_selector`), exact-mode `run`/`clip`/`edit`, MCP `snippet_get` |
| Read-only path | `library::readonly_library_sources` (`src/library/manager.rs:815`) → `LibraryManager::resolve_readonly_sources` (`src/library/manager.rs:261-302`) | **Yes** — resolves to every configured library | `snp list` (via `commands::load_readonly_snippets`), MCP `snippets_list` / `snippets_search` |
| Single-library path | `commands::get_library_path` (`src/commands/mod.rs:91-157`) | **No** — `get_library_by_filename("all")` returns `library_not_found` | `snp new`, `snp edit`, `snp sync`, and the interactive selector (`snp search`, `snp select`) |

There are **two independent** definitions of the keyword, and they agree:

- The selector path matches the literal `"all"` in
  `LibraryScope::from_filter_arg` (`src/selector.rs:65-71`).
- The read-only path matches the literal `"all"` on its own, in
  `LibraryManager::resolve_readonly_sources` (`src/library/manager.rs:290-301`);
  it does **not** consume `LibraryScope::from_filter_arg`.

Both are unit-tested (`src/selector.rs:1086-1104`, `src/library/tests.rs:495`).
A command that wants cross-library support must route through one of those
two; adding `"all"` handling to `get_library_path` would fork the rule a third
time.

---

## Scope Modes

### Primary Library (default)

When no `--library` flag is provided, commands operate on the **primary library** — the library configured as default in `~/.config/snp/libraries.toml`.

```
snp list                    # lists snippets in the primary library
snp new "echo hello"        # adds to the primary library
snp run                     # selects from the primary library
```

### Named Library (`--library <name>`)

Explicitly selects a specific library by name.

```
snp list --library work     # lists snippets in the "work" library
snp new "echo hello" --library personal
snp get --library work --query deploy
```

The name is matched against library filenames **exactly** (case-sensitive):
`get_library_by_filename` compares `meta.filename == name`
(`src/library/manager.rs:235-240`). If the library does not exist, the command
exits with an error.

### All Libraries (`--library all`)

Supported on the selector path and on the read-only path:

```
snp get  --library all --query deploy   # selector path
snp list --library all                  # read-only path (concatenates every library)
```

`snp search --library all` is **not** supported: the interactive selector edits
and deletes inside one library file, so it takes the single-library path and
rejects the scope with an explicit error naming the supported alternatives.
There is no `--all-libraries` flag anywhere in the CLI.

`all` is a reserved library name. `validate_library_name`
(`src/library/model.rs:218-223`) rejects `snp library create all`, so the
keyword can never be ambiguous between "the library named all" and "every
library". The check is on the exact lowercase string `all`.

### Library ID (sync-linked libraries)

A library has **two** identifiers, and they are not interchangeable:

- The **filename stem** (`my-work` from `~/.config/snp/libraries/my-work.toml`)
  is the local name. It is what `--library` matches and what `snp library list`
  prints.
- The **library ID** (`LibraryMeta.library_id`, `src/library/model.rs:80-90`)
  is the **server-side** library ID. It is assigned when the library is linked
  to a sync server and is **empty** until then. It is the value reported as
  `library_id` in `snp get --json` and used in sync status and server-side
  operations.

---

## Resolution Rules

| Scenario | Behavior |
|----------|----------|
| No `--library` flag | Use primary library |
| `--library <name>` | Match by filename exactly (case-sensitive), error if not found |
| `--library all` (selector or read-only path) | Union of all visible libraries |
| `--library all` (single-library path) | Error: `library_not_found` — the keyword is reserved, so this reads as "does not exist" |
| Primary library not set | Error `No primary library` with the guidance `Create a library with 'snp library create <name>'` (`src/selector.rs:655-660`) |

---

## Cross-Library Ambiguity

When `--library all` is used with a selector-based command, every match from
every visible library is collected and then sorted deterministically by
**library name → description → snippet ID** (`selector::finish_aggregate`,
`src/selector.rs:558-593`). The `--resolution` policy then decides what
happens:

- **`unique` (default)**: more than one match is **ambiguous**, not "the first
  one wins". `snp get` exits 5 (`AMBIGUOUS`) and prints the candidate
  identities.
- **`first`**: the first match in that sorted order is returned.
- **`all`**: every match is returned.

Applies to every selector, not just description:

- **Description match**: two libraries with the same description → ambiguous
  under the default policy.
- **Command match**: two libraries with the same exact command → ambiguous
  under the default policy.
- **`get` with `--id`**: snippet IDs are deterministic content hashes
  (`docs/IDENTITY_CONTRACT.md`), so the *same content* stored in two libraries
  yields the *same* ID — that is an ambiguous match, not a unique one. An ID
  is unique only within the resolved scope.

---

## Machine-Output Library Identity

- `snp get --json` always includes `library` and `library_id`
  (`GetJsonOutput`, `src/commands/get_cmd.rs:68-78`), including when the scope
  came from the match itself. `library` is the filename stem; `library_id` is
  the server-side library ID and is empty for a library that is not
  sync-linked.
- `snp list --json` emits exactly `description`, `command`, `output`, `tags`,
  `folders`, `favorite` — **no `library` field** (`src/commands/list_cmd.rs:155-162`).
- `snp list --csv` uses the same six columns; there is **no `library` column**
  (`src/commands/list_cmd.rs:176`).

Because `list` never reports a source library, it has no library identity in its
output even when `--library all` concatenates several libraries.

---

## Flag Interaction

- `snp list` warns and ignores `--library` when `--config` is set, because
  `--config` names an explicit file path (`src/commands/list_cmd.rs:59-61`).
- `--library` help text advertises `"all"` only where the selector or read-only
  path backs the command (`snp get`, `snp list`). `snp search` names `all` only
  to say it is rejected. Do not copy that wording onto a single-library
  command.

---

## Case and Canonicalization

- Library names are matched **exactly** (case-sensitive) against the stored
  filename, on every path — selector, read-only, and single-library alike.
  `--library Work` does not match a library stored as `work`.
- `LibraryManager::create_library` stores the name **as given**; it does not
  lowercase it or convert spaces to hyphens. Duplicate detection at creation
  time is case-insensitive, so `Work` and `work` cannot both exist
  (`src/library/manager.rs:404-414`).
- Name **creation** is validated, not normalized: `validate_library_name`
  rejects empty names, names over 50 characters, path separators, a `.toml`
  suffix, and the reserved `all`
  (`src/library/model.rs:179-225`).
- The `library_id` used in sync status and server operations is the
  **server-side** library ID, not the filename stem; it is empty until the
  library is linked to a sync server.

---

## Primary Library Default

- When no primary library is set, commands that require a library exit with guidance (`Create a library with 'snp library create <name>'`).
- `snp library set-primary <name>` sets the primary.
- `snp library list` shows which library is marked primary.
- The primary library persists in `~/.config/snp/libraries.toml`.

---

## Related

- [architecture/selector.md](../architecture/selector.md) — the scope model and the `searchable_text` contract.
- [architecture/library.md](../architecture/library.md) — library layout, migration, and the read-only source resolver.
- [MCP.md](MCP.md) — the MCP tool surface over the same resolvers.
