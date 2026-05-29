# Commands Module (`src/commands/mod.rs`)

## Overview

Shared helpers for all CLI commands. Provides path resolution, library loading/saving, snippet expansion, and the shared TUI snippet selection flow.

## Path Resolution

### get_config_path()

Returns `PathBuf` for files in `~/.config/snp/` (XDG-compliant).

### get_library_path()

Returns path to `snippets.toml` or active library file.

### Snippet File Locations

```
~/.config/snp/
├── snippets.toml          # Legacy single-file
└── libraries/
    └── <name>.toml        # Per-library files
```

## Library Operations

### load_snippets()

```rust
pub fn load_snippets() -> SnipResult<Snippets>
```
- Reads TOML from library path
- Returns empty `Snippets` if file doesn't exist
- Handles migration from single-file to multi-library mode

### save_snippets()

```rust
pub fn save_snippets(snippets: &Snippets) -> SnipResult<()>
```
- Writes TOML to library path
- Creates parent directories if needed
- No automatic backup (handled by callers like `new_cmd`)

### get_snippet_data()

Returns a reference to the inner `Vec<Snippet>` from `Snippets`.

## Snippet Expansion

### expand_snippet_command()

Substitutes variable placeholders in the command string:

```rust
pub fn expand_snippet_command(
    command: &str,
    variables: &[(String, Option<String>)],
) -> SnipResult<String>
```

- Syntax: `<name>` or `<name=default>`
- Escapes: `\<` → `<`, `\>` → `>`
- Returns error if required variable (no default) is missing

### strip_escape_sequences()

Converts escape sequences back to literal characters for display/execution.

## Shared TUI Selection

### run_snippet_selection()

```rust
pub fn run_snippet_selection<F>(process_snippet: F) -> SnipResult<()>
where
    F: FnOnce(&Snippet) -> SnipResult<()>,
```

Common flow for `run`, `clip`, `search` commands:
1. Load snippets
2. Open TUI with snippet list
3. User selects snippet
4. Call `process_snippet` closure with selected snippet
5. Return result

Used by:
- `run_cmd` — Executes snippet via shell
- `clip_cmd` — Copies to clipboard
- `search_cmd` — Displays snippet details

## Error Handling

All helpers return `SnipResult<T>` which is `Result<T, SnipError>`.

Common error variants:
- `SnipError::Io` — File not found, permission denied
- `SnipError::Toml` — Parse/serialize errors
- `SnipError::LibraryNotFound` — No library at path
