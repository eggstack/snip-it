# Library Scope

> **Evergreen contract.** How commands resolve which library (or libraries)
> they operate on.
>
> Read this before adding a `--library` flag. `src/selector.rs` owns the
> canonical scope model; `src/commands/mod.rs::get_library_path` is the
> single-library path and does **not** understand `"all"`.

---

## Two resolution paths (do not mix them)

| Path | Entry point | Understands `"all"`? | Used by |
|------|-------------|----------------------|----------|
| Selector path | `selector::LibraryScope::from_filter_arg` / `from_owned_arg` (`src/selector.rs:61-68`) | **Yes** → `LibraryScope::AllLibraries` | `snp get` (via `exact_selector`), any selector-based resolution |
| Single-library path | `commands::get_library_path` (`src/commands/mod.rs:91`) | **No** — `get_library_by_filename("all")` returns `library_not_found` | `snp list`, `snp new`, `snp edit` |

`"all"` is defined in exactly one place (`LibraryScope::from_filter_arg`) and is
covered by unit tests in `src/selector.rs`. A command that wants cross-library
support must route through the selector; adding `"all"` handling to the
single-library path would fork the rule.

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

Supported **only on the selector path**, i.e. `snp get`:

```
snp get --library all --query deploy
```

`snp list --library all` is **not** supported and fails with a
`library_not_found` error for `"all"` — `list` uses the single-library path
(`src/commands/list_cmd.rs:64`). There is no `--all-libraries` flag anywhere in
the CLI.

### Library ID (sync-linked libraries)

For sync-linked libraries, the **library ID** is the filename stem (e.g., `my-work` from `~/.config/snp/libraries/my-work.toml`). The ID is stable and used in sync status, pending state, and server-side operations.

---

## Resolution Rules

| Scenario | Behavior |
|----------|----------|
| No `--library` flag | Use primary library |
| `--library <name>` | Match by filename (case-insensitive), error if not found |
| `--library all` (selector path only) | Union of all visible libraries |
| `--library all` (single-library path) | Error: `library_not_found` |
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

Because `list` is single-library by construction, it has no source-library
identity to report.

---

## Flag Interaction

- `snp list` warns and ignores `--library` when `--config` is set, because
  `--config` names an explicit file path (`src/commands/list_cmd.rs:58-59`).
- `--library` help text advertises `"all"` only where the selector path backs
  the command (currently `snp get`). Do not copy that wording onto a
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
