# Library Scope

> **Evergreen contract.** How commands resolve which library (or libraries)
> they operate on.
>
> Read this before adding a `--library` flag. `src/selector.rs` owns the
> canonical scope model; `src/library::readonly_library_sources` is the
> read-only path and **does** understand `"all"`.

---

## Two resolution paths (do not mix them)

| Path | Entry point | Understands `"all"`? | Used by |
|------|-------------|----------------------|----------|
| Selector path | `selector::LibraryScope::from_filter_arg` / `from_owned_arg` (`src/selector.rs:65-71`) | **Yes** → `LibraryScope::AllLibraries` | `snp get` (via `exact_selector`), exact-mode `run`/`clip`/`edit`, MCP |
| Read-only path | `library::readonly_library_sources` (`src/library/manager.rs:773`) | **Yes** — resolves to every configured library | `snp list` (via `commands::load_readonly_snippets`) |
| Single-library path | `commands::get_library_path` (`src/commands/mod.rs`) | **No** — `get_library_by_filename("all")` returns `library_not_found` | `snp new`, `snp edit`, `snp sync`, and the interactive selector (`snp search`, `snp select`) |

`"all"` is defined in exactly one place (`LibraryScope::from_filter_arg`), with
`readonly_library_sources` consuming the same rule, and it is covered by unit
tests in `src/selector.rs` and `src/library/manager.rs`. A command that wants
cross-library support must route through one of those two; adding `"all"`
handling to `get_library_path` would fork the rule.

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

The name is matched against library filenames (case-insensitive). If the library does not exist, the command exits with an error.

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
(`src/library/model.rs`) rejects `snp library create all`, so the keyword can
never be ambiguous between "the library named all" and "every library".

### Library ID (sync-linked libraries)

For sync-linked libraries, the **library ID** is the filename stem (e.g., `my-work` from `~/.config/snp/libraries/my-work.toml`). The ID is stable and used in sync status, pending state, and server-side operations.

---

## Resolution Rules

| Scenario | Behavior |
|----------|----------|
| No `--library` flag | Use primary library |
| `--library <name>` | Match by filename (case-insensitive), error if not found |
| `--library all` (selector or read-only path) | Union of all visible libraries |
| `--library all` (single-library path) | Error: unsupported for this command |
| Primary library not set | Error with guidance to run `snp library create` or `snp library set-primary` |

---

## Cross-Library Ambiguity

When `--library all` is used with a selector-based command:

- **Description match**: If multiple libraries contain snippets with the same description, the first match (by library sort order) is returned.
- **Command match**: If multiple libraries contain snippets with the exact same command, the first match is returned.
- **`get` with `--id`**: IDs are globally unique — no ambiguity.

---

## Machine-Output Library Identity

- `snp get --json` always includes `library` and `library_id`
  (`GetJsonOutput`, `src/commands/get_cmd.rs:69-78`), including when the scope
  came from the match itself.
- `snp list --json` emits exactly `description`, `command`, `output`, `tags`,
  `folders`, `favorite` — **no `library` field** (`src/commands/list_cmd.rs:150-157`).
- `snp list --csv` uses the same six columns; there is **no `library` column**
  (`src/commands/list_cmd.rs:171`).

Because `list` never reports a source library, it has no library identity in its
output even when `--library all` concatenates several libraries.

---

## Flag Interaction

- `snp list` warns and ignores `--library` when `--config` is set, because
  `--config` names an explicit file path (`src/commands/list_cmd.rs:58-59`).
- `--library` help text advertises `"all"` only where the selector or read-only
  path backs the command (`snp get`, `snp list`). Do not copy that wording onto a
  single-library command.

---

## Case and Canonicalization

- Library names are matched **case-insensitively** for user-facing flags.
- Library filenames are stored in lowercase with hyphens (canonical form).
- Canonicalization is stable across platforms (no case-folding differences between macOS, Linux, Windows).
- The `library_id` used in sync status and server operations is the filename stem (lowercase, hyphenated).

---

## Primary Library Default

- When no primary library is set, commands that require a library exit with guidance.
- `snp library set-primary <name>` sets the primary.
- `snp library list` shows which library is marked primary.
- The primary library persists in `~/.config/snp/libraries.toml`.
