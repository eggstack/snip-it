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
pub fn load_snippets(config: &Option<PathBuf>) -> SnipResult<Snippets>
```
- Reads TOML from library path
- Returns empty `Snippets` if file doesn't exist
- Handles migration from single-file to multi-library mode

### save_snippets()

```rust
pub fn save_snippets(snippets: &Snippets, config: &Option<PathBuf>) -> SnipResult<()>
```
- Writes TOML to library path
- Creates parent directories if needed
- Uses atomic write (temp file + rename) and creates a backup before saving

### get_snippet_data()

Extracts parallel arrays of descriptions, commands, tags, folders, and favorites for TUI display, along with a mapping from filtered indices to original snippet indices. Deleted snippets are filtered out.

## Snippet Expansion

### expand_snippet_command()

Expands a snippet command, prompting for variables if present:

```rust
pub fn expand_snippet_command(snippet: &Snippet) -> SnipResult<ExpandedCommand>
```

- If no variables found, strips escape sequences and returns `Expanded`
- If variables found, prompts user via TUI dialog
- Returns `Cancel`, `Skip`, or `Expanded(String)`
- Variable syntax: `<name>` or `<name=default>`
- Escapes: `\<` → `<`, `\>` → `>`

### strip_escape_sequences()

Defined in `crate::utils::variables`. Converts escape sequences back to literal characters for display/execution.

## Shared TUI Selection

### run_snippet_selection()

```rust
pub fn run_snippet_selection<F>(
    filter: Option<String>,
    library: Option<String>,
    do_sync: bool,
    allow_delete: bool,
    sort_opts: Option<SortOptions>,
    runtime: Option<&tokio::runtime::Runtime>,
    mut process_fn: F,
) -> SnipResult<SelectionOutcome>
where
    F: FnMut(&Snippet, Option<String>) -> SnipResult<ProcessResult>,
```

Common flow for `run`, `clip`, `select`, `search` commands:
1. Load library and snippets
2. Open TUI with snippet list
3. User selects snippet (or deletes if allowed)
4. Call `process_fn` closure with selected snippet and copy flag
5. Optionally run post-selection sync (when `do_sync` is true)
6. Return `SelectionOutcome`

The `runtime` parameter must be `Some(&RUNTIME)` when `do_sync` is true, `None` otherwise.

Used by:
- `run_cmd` — Executes snippet via shell
- `clip_cmd` — Copies to clipboard
- `select_cmd` — Returns snippet for programmatic use
- `search_cmd` — Displays snippet details

## Error Handling

All helpers return `SnipResult<T>` which is `Result<T, SnipError>`.

Common error variants:
- `SnipError::Io` — File not found, permission denied
- `SnipError::Toml` — Parse/serialize errors
- `SnipError::LibraryNotFound` — No library at path
