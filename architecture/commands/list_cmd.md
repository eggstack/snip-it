# list_cmd — Text-Based Snippet Listing

## Overview

`list_cmd` displays snippets in a plain text, non-interactive format. Useful for scripting and piping to other commands.

## Entry Point

```rust
pub fn run(
    filter: Option<String>,
    config: Option<PathBuf>,
    library: Option<String>,
    format: ListFormat,
    sort_opts: Option<SortOptions>,
    search_output: bool,
) -> SnipResult<()>
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
    "description": "hello",
    "command": "echo \"Hello, World!\"",
    "output": "",
    "tags": ["demo"],
    "folders": ["scripts"],
    "favorite": false
  }
]
```

### CSV (`--csv`)
```csv
description,command,output,tags,folders,favorite
hello,echo "Hello, World!",,demo,scripts,false
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

## Output-Aware Search (Release 4B)

The `--search-output` flag includes the output/notes field in fuzzy search matching.

### Behavior

- Default (flag absent): fuzzy filter matches only `description` and `command`.
- With `--search-output`: fuzzy filter also matches against `output` (bounded to 512 chars for scoring).
- Output content is sanitized for terminal display via `OutputPresentation::for_scoring()`.

### Default Display

- Empty output fields are hidden in the default (human) display format.
- Non-empty output shows a single-line summary (truncated to 80 chars).
- JSON and CSV output always include the raw `output` field.
