# library_cmd — Library Management

## Overview

`library_cmd` manages multiple snippet libraries. Libraries allow organizing snippets into separate files and choosing a "primary" library for operations.

## Entry Point

Each subcommand dispatches to a dedicated function:

```rust
pub fn run_list() -> SnipResult<()>
pub fn run_create(name: String) -> SnipResult<()>
pub fn run_delete(name: String, force: bool) -> SnipResult<()>
pub fn run_set_primary(name: String) -> SnipResult<()>
pub fn run_show(name: Option<String>) -> SnipResult<()>
```

## Subcommands

### list
```bash
snp library list
```
Lists all configured libraries with their paths and metadata.

### create
```bash
snp library create <name>
```
Creates a new empty library file at `~/.config/snp/libraries/<name>.toml`.

### delete
```bash
snp library delete <name>
```
Deletes a library file (with confirmation). If the deleted library was primary, another library is promoted.

### set-primary
```bash
snp library set-primary <name>
```
Sets the active library for all operations.

### show
```bash
snp library show
```
Shows the current primary library path and statistics.

## Library Configuration

Libraries metadata stored in `~/.config/snp/libraries.toml`:

```toml
[[library]]
name = "personal"
path = "~/.config/snp/libraries/personal.toml"
primary = true

[[library]]
name = "work"
path = "~/.config/snp/libraries/work.toml"
primary = false
```

## Migration from Single File

If `snippets.toml` exists but `libraries.toml` does not:
1. Create `libraries/` directory
2. Move `snippets.toml` to `libraries/default.toml`
3. Create `libraries.toml` with single default entry
4. Mark as primary

This happens automatically on first library operation if migration needed.

## Related

- [mod.md](mod.md) — Path resolution, library loading
- [library.md](../library.md) — LibraryManager and snippet structures
