# Utilities

[← Back to Overview](overview.md)

## Module Index

**Directory**: `src/utils/`

| Module | File | Purpose |
|--------|------|---------|
| `config` | `config.rs` | Config directory paths, macOS migration |
| `variables` | `variables.rs` | Variable parsing and expansion |
| `toml_helpers` | `toml_helpers.rs` | TOML escape sequence handling |
| `shell_keywords` | `shell_keywords.rs` | Shell keyword set for syntax highlighting |

## Variables

**File**: `src/utils/variables.rs` (277 lines)

### Syntax

- `<name>` — Variable with no default (user must provide)
- `<name=default>` — Variable with default value
- `\<` and `\>` — Literal angle brackets (escape sequences)

### Functions

| Function | Description |
|----------|-------------|
| `parse_variables(command)` | Extract `Variable` structs from command |
| `extract_variables_for_display(command)` | Format variables for TUI display |
| `expand_command(command, values)` | Replace `<var>` with user-provided values |
| `strip_escape_sequences(command)` | Convert `\<` → `<`, `\>` → `>` |

### Parsing

`extract_variable_tokens()` walks the command character by character:
- Tracks backslash state for escape handling
- Skips escaped `\<` sequences
- Extracts content between `<` and `>`
- Splits on `=` for default values

### Expansion

`expand_command()` replaces variables while preserving:
- Escaped angle brackets (become literal `<`/`>`)
- Trailing backslashes
- Multiple uses of the same variable (tracked by index)

### Tests

19 tests covering: simple vars, defaults, multiple vars, no vars, escaped brackets, escaped backslashes, mixed escapes, trailing backslash, escaped backslash before bracket.

## TOML Helpers

**File**: `src/utils/toml_helpers.rs` (180 lines)

### Problem

TOML double-quoted strings interpret `\<` as an escape sequence, which fails because `\<` is not a valid TOML escape. Snippet commands frequently contain `\<` (for variables) and `\>`.

### Solution

Two complementary functions:

| Function | Purpose |
|----------|---------|
| `fix_invalid_toml_escapes(toml_str)` | **On load**: Convert affected double-quoted strings to single-quoted |
| `quote_strings_containing_backslashes(toml_str)` | **On save**: Convert backslash-containing strings to single-quoted |

### Strategy

For each double-quoted string in the TOML:
1. Check if it contains `\<` or `\>` (on load) or any `\` (on save)
2. If no single quotes in content → convert to single-quoted string
3. If single quotes present → escape backslash with `\\` in double quotes

Uses regex `r#""([^"\\]*(?:\\.[^"\\]*)*)""#` to find double-quoted strings.

## Shell Keywords

**File**: `src/utils/shell_keywords.rs` (199 lines)

A `LazyLock<HashSet<&str>>` containing ~190 common CLI tool names for syntax highlighting:

- **Version control**: git, svn, hg
- **Containers**: docker, kubectl, helm, podman
- **Package managers**: npm, yarn, cargo, pip, brew
- **Cloud**: aws, gcloud, az, terraform
- **Core utils**: ls, grep, sed, awk, find, curl, ssh
- **Process**: ps, kill, top, systemctl
- **Editors**: vim, nano, emacs, code

## Config Paths

**File**: `src/utils/config.rs`

| Function | Returns |
|----------|---------|
| `get_config_dir()` | `~/.config/snp` (or `$XDG_CONFIG_HOME/snp`) |
| `get_config_path(filename)` | `get_config_dir().join(filename)` |
| `get_snippets_path()` | `get_config_path("snippets.toml")` |
| `get_sync_config_path()` | `get_config_path("sync.toml")` |
| `get_legacy_macos_config_dir()` | Old macOS path if it exists |
| `migrate_macos_config_dir()` | Move files from old to new path |
