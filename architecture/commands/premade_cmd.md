# premade_cmd — Premade Library Access

## Overview

`premade_cmd` accesses community-curated snippet libraries from the snip-sync server. Premade libraries provide ready-to-use snippets for common tasks.

## Entry Point

```rust
pub fn run(matches: &ArgMatches) -> SnipResult<()>
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
Downloads a specific premade library:
1. Fetch library definition from server
2. Save to `~/.config/snp/premade/<name>.toml`
3. Merge snippets into local library (optional)

### sync
```bash
snp premade sync
```
Updates all downloaded premade libraries with latest versions from server.

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
