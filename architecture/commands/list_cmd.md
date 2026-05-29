# list_cmd — Text-Based Snippet Listing

## Overview

`list_cmd` displays snippets in a plain text, non-interactive format. Useful for scripting and piping to other commands.

## Entry Point

```rust
pub fn run(matches: &ArgMatches) -> SnipResult<()>
```

## Flow

1. Load snippets from library
2. Apply optional filters (tag, folder, search term)
3. Print to stdout in specified format
4. Exit

## Output Formats

### Default (plain text)
```
 Name          Command                     Tags      Folder
─────────────────────────────────────────────────────────────
 hello         echo "Hello, World!"       demo      scripts
 fortunes      fortune | cowsay           fun      scripts
```

### JSON (`--json`)
```json
[
  {
    "id": "...",
    "name": "hello",
    "command": "echo \"Hello, World!\"",
    "tags": ["demo"],
    "folders": ["scripts"]
  }
]
```

### CSV (`--csv`)
```csv
name,command,tags,folders
hello,echo "Hello, World!",demo,scripts
```

## Filters

- `--tag <tag>` — Filter by tag
- `--folder <folder>` — Filter by folder
- `--search <term>` — Fuzzy search on name/command
- `--sort <field>` — Sort by name, date, or usage

## Use Cases

- Integration with external tools (jq, fzf)
- CI/CD pipeline inspection
- Quick lookup without TUI

## Related

- [mod.md](mod.md) — Shared helpers
- [search_cmd.md](search_cmd.md) — TUI interactive search
- [tui.md](../tui.md) — TUI architecture
