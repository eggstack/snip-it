# Library Scope

> Phase 08A — Workstream I
> How commands resolve which library (or libraries) they operate on.

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
```

The name is matched against library filenames (case-insensitive). If the library does not exist, the command exits with an error.

### All Libraries (`--library all`)

Operates across every library simultaneously. Only supported by `list` and `get`.

```
snp list --library all      # lists snippets from all libraries
snp get --library all       # searches all libraries for a snippet
```

### Library ID (sync-linked libraries)

For sync-linked libraries, the **library ID** is the filename stem (e.g., `my-work` from `~/.config/snp/libraries/my-work.toml`). The ID is stable and used in sync status, pending state, and server-side operations.

---

## Resolution Rules

| Scenario | Behavior |
|----------|----------|
| No `--library` flag | Use primary library |
| `--library <name>` | Match by filename (case-insensitive), error if not found |
| `--library all` | Union of all libraries (only `list` and `get`) |
| Primary library not set | Error with guidance to run `snp library create` or `snp library set-primary` |

---

## Cross-Library Ambiguity

When `--library all` is used:

- **Description match**: If multiple libraries contain snippets with the same description, the first match (by library sort order) is returned.
- **Command match**: If multiple libraries contain snippets with the exact same command, the first match is returned.
- **`get` with `--id`**: IDs are globally unique — no ambiguity.
- **`list` output**: Each item includes a `library` field identifying its source.

---

## Machine-Output Library Identity

JSON and CSV output include library identity when operating across libraries:

- `list --json --library all`: Each item includes a `library` field.
- `list --csv --library all`: CSV includes a `library` column.
- `get --json`: Always includes `library` and `library_id` fields.

---

## Help Text Convention

- Default help text assumes primary library: `"List snippets in the default library"`.
- `--library` help: `"Library name or 'all' for all libraries"`.
- `--all-libraries` flag (where available): Explicit opt-in for cross-library operations.

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
