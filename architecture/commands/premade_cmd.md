# premade_cmd — Premade Library Access

## Overview

`premade_cmd` accesses community-curated snippet libraries from the snip-sync server. Premade libraries provide ready-to-use snippets for common tasks.

## Entry Point

Each subcommand dispatches to a dedicated function:

```rust
pub fn run_list(runtime: &tokio::runtime::Runtime) -> SnipResult<()>
pub fn run_get(name: Option<String>, all: bool, runtime: &tokio::runtime::Runtime) -> SnipResult<()>
pub fn run_sync(runtime: &tokio::runtime::Runtime) -> SnipResult<()>
pub fn run_search(query: String, runtime: &tokio::runtime::Runtime) -> SnipResult<()>
pub fn run_update(name: String, runtime: &tokio::runtime::Runtime) -> SnipResult<()>
```

## Subcommands

### list
```bash
snp premade list
```
Lists all available premade libraries on the server:
- Name
- Description
- Snippet count
- Tags

### get
```bash
snp premade get <library-id>
```
Downloads a specific premade library (or all with `snp premade get all`):
1. Fetch library definition from server
2. Save to `~/.config/snp/premade/<name>.toml`
3. Merge snippets into local library (optional)

### search
```bash
snp premade search <query>
```
Searches premade libraries on the server by query string:
- Name, description, tags matched against query
- Displays snippet count and tags for each match

### sync
```bash
snp premade sync
```
Downloads all missing premade libraries from the server.

### update
```bash
snp premade update <name>
```
Re-downloads a specific premade library and shows the diff:
- Compares old and new content line-by-line
- Reports lines added/removed
- Skips if already up to date

## Premade Library Source

Server-side premade libraries are defined in `snip-sync/src/premade.rs`:
- Scans a `premade-libraries/` directory on the server
- Provides metadata via `ListPremadeLibraries` RPC
- Clients download via `GetPremadeLibrary` RPC

## Local Storage

Downloaded premade libraries stored at:
```
~/.config/snp/premade/
├── git.toml          # Git commands
├── docker.toml      # Docker commands
├── kubernetes.toml  # K8s commands
└── ...
```

## Integration with Local Libraries

Premade snippets can be:
- **Viewed only** — Keep separate from local snippets
- **Merged** — Import into primary library
- **Updated** — Re-sync with server to get new versions

## Related

- [sync.md](../sync.md) — Premade library RPC protocol
- [library_cmd.md](library_cmd.md) — Library management
