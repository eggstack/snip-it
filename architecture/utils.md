# Utilities

[← Back to Overview](overview.md)

## Module Index

**Directory**: `src/utils/` (re-exported via `src/utils/mod.rs`)

| Module | File | Lines | Purpose | Detail |
|--------|------|-------|---------|--------|
| `atomic` | `atomic.rs` | 699 | Durability-aware atomic writes | [utils/atomic.md](utils/atomic.md) |
| `config` | `config.rs` | 231 | Config directory paths, macOS migration | [utils/config.md](utils/config.md) |
| `process` | `process.rs` | 166 | Process-liveness checks, owned-lock-file removal | [utils/process.md](utils/process.md) |
| `redact` | `redact.rs` | 72 | Secret redaction for display/log paths | [utils/redact.md](utils/redact.md) |
| `shell_keywords` | `shell_keywords.rs` | 198 | Shell keyword set for syntax highlighting | [utils/shell_keywords.md](utils/shell_keywords.md) |
| `tempfile_guard` | `tempfile_guard.rs` | 38 | RAII temporary-file cleanup | [utils/tempfile_guard.md](utils/tempfile_guard.md) |
| `toml_helpers` | `toml_helpers.rs` | 456 | TOML escape sequence handling | [utils/toml_helpers.md](utils/toml_helpers.md) |
| `variables` | `variables.rs` | 1919 | Variable parsing and expansion | [utils/variables.md](utils/variables.md) |
| `mod` | `mod.rs` | 23 | Module declarations + variable helper re-exports | — |

## Variables

**File**: `src/utils/variables.rs` (1919 lines — includes Pet-choice support, diagnostics, and ~100 unit tests)

Full reference: [utils/variables.md](utils/variables.md).

### Syntax

- `<name>` — Variable with no default (user must provide)
- `<name=default>` — Variable with default value
- `<name=|_opt1_||_opt2_||>` — Pet-compatible multiple choice (first choice is the default)
- `\<` and `\>` — Literal angle brackets (escape sequences)

### Functions

| Function | Description |
|----------|-------------|
| `parse_variables(command)` | Extract `Variable` structs from command |
| `parse_variables_diagnostics(command)` | Variables plus `VariableDiagnostic` warnings (`choice.*`, `var.duplicate`) |
| `extract_variables_for_display(command)` | Format variables for TUI display |
| `expand_command(command, values)` | Replace `<var>` with user-provided values |
| `strip_escape_sequences(command)` | Convert `\<` → `<`, `\>` → `>`, `\\` → `\` |
| `has_unmatched_angle_bracket(command)` | Detect unclosed `<` (nesting- and escape-aware) |
| `VariableAssignments::parse_arg / from_pairs` | Parse `--var key=value` CLI assignments |

### Parsing

`extract_variable_tokens()` walks the command character by character:
- Tracks backslash state for escape handling
- Skips escaped `\<` sequences
- Extracts content between `<` and `>` (nesting-aware via depth counter)
- Splits on `=` for default values; detects `|_..._||` choice syntax

### Expansion

`expand_command()` replaces variables while preserving:
- Escaped angle brackets (become literal `<`/`>`)
- Trailing backslashes
- Multiple uses of the same variable (tracked by per-name usage index)

## TOML Helpers

**File**: `src/utils/toml_helpers.rs` (456 lines — ~210 lines of scanner plus tests)

Full reference: [utils/toml_helpers.md](utils/toml_helpers.md).

### Problem

TOML double-quoted strings interpret `\<` as an escape sequence, which fails because `\<` is not a valid TOML escape. Snippet commands frequently contain `\<` (for variables) and `\>`.

### Solution

Two complementary functions sharing one hand-written `fix_toml_strings` scanner:

| Function | Purpose |
|----------|---------|
| `fix_invalid_toml_escapes(toml_str)` | **On load**: rewrite affected single-line basic strings as single-quoted |
| `quote_strings_containing_backslashes(toml_str)` | **For hand-written TOML**: rewrite any backslash-containing string as single-quoted |

### Strategy

For each single-line double-quoted string in the TOML:
1. Check if it contains `\<` or `\>` (on load) or any `\` (hand-written save path)
2. If no single quotes in content → convert to single-quoted string
3. If single quotes present → escape backslash with `\\` in double quotes

Triple-quoted multi-line regions are passed through verbatim. Never run on `toml::to_string_pretty` output.

## Shell Keywords

**File**: `src/utils/shell_keywords.rs` (198 lines)

Full reference: [utils/shell_keywords.md](utils/shell_keywords.md).

A `SHELL_KEYWORDS: &[&str]` list of 190 common CLI tool names plus a `LazyLock<HashSet<&str>>` (`SHELL_KEYWORDS_SET`) for O(1) lookup, used by the TUI syntax highlighter:

- **Version control**: git, svn, hg
- **Containers**: docker, kubectl, helm, podman
- **Package managers**: npm, yarn, cargo, pip, brew
- **Cloud**: aws, gcloud, az, terraform
- **Core utils**: ls, grep, sed, awk, find, curl, ssh
- **Process**: ps, kill, top, systemctl
- **Editors**: vim, nano, emacs, code

## Config Paths

**File**: `src/utils/config.rs` (231 lines)

Full reference: [utils/config.md](utils/config.md).

| Function | Returns |
|----------|---------|
| `get_config_dir()` | `~/.config/snp` (or `$XDG_CONFIG_HOME/snp`) |
| `ensure_config_dir()` | Same, creating with `0o700` on Unix |
| `get_config_path(filename)` | `get_config_dir().join(filename)` |
| `get_snippets_path()` | `get_config_path("snippets.toml")` |
| `get_sync_config_path()` | `get_config_path("sync.toml")` |
| `derive_sync_state_dir()` | Parent of the sync config path |
| `get_legacy_macos_config_dir()` | Old macOS path if it exists |
| `migrate_macos_config_dir()` | Move files from old to new path |

## Atomic Writes, Temp Files, Process Liveness, Redaction

- **Atomic writes** (`atomic.rs`): `write_private_atomic` for simple TOML saves, `atomic_replace` with `Durability` classes for the rest — see [utils/atomic.md](utils/atomic.md).
- **Temp-file cleanup** (`tempfile_guard.rs`): RAII guard consumed by `persist()` after rename — see [utils/tempfile_guard.md](utils/tempfile_guard.md).
- **Process checks** (`process.rs`): shared `is_process_alive` (`kill(pid, 0)`, ESRCH-dead / EPERM-alive) and owned-lock-file removal used by the transaction, local-data, and execution locks — see [utils/process.md](utils/process.md).
- **Secret redaction** (`redact.rs`): best-effort `redact_secrets` for logs/errors/status; backups use a separate `redact_sync_config` — see [utils/redact.md](utils/redact.md).
