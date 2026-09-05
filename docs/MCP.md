# Local MCP integration

`snp mcp serve` implements the MCP stdio transport. An MCP client starts the
process on demand and exchanges one newline-delimited JSON-RPC 2.0 message per
line. Standard output is protocol-only; diagnostics go to standard error.

The server supports protocol revisions `2025-11-25`, `2025-06-18`,
`2025-03-26`, and `2024-11-05`. It implements `initialize`,
`notifications/initialized`, `ping`, `tools/list`, and `tools/call`. EOF
shuts it down cleanly. Requests are bounded to 1 MiB.

## Tools

The initial tool set is intentionally read-only and contains exactly:

- `snippets_list`: list snippets from the primary library, a named library, or
  `all`. Optional `limit` is bounded to 1,000.
- `snippets_search`: fuzzy-search descriptions and command text using the
  same `SkimMatcherV2` and relevance ranking used by deterministic selection.
  `query` is required.
- `snippet_get`: retrieve one snippet by exact `id`, or by a unique exact
  `description` (case-insensitive). Ambiguous and missing matches are returned
  as structured tool errors.

Results include `id`, `library`, `description`, `command`, `tags`, `folders`,
and `favorite`. Output/notes, sync metadata, credentials, and keychain data
are not exposed. The adapter never invokes a shell or changes snippets.

Malformed TOML fails closed through the normal library loader. Legacy
single-file mode is read as the implicit `snippets` library without triggering
the CLI's migration write path. MCP reads do not trigger auto-sync.

## Client registration

The executable path below means the absolute path printed by
`snp mcp instructions <client>`.

### Claude Code

With a current Claude Code CLI, the official noninteractive command is:

```text
claude mcp add snip-it --scope user -- /absolute/path/to/snp mcp serve
```

`snp mcp install claude` invokes this command when `claude` is available.

### Codex

```text
codex mcp add snip-it -- /absolute/path/to/snp mcp serve
```

`snp mcp install codex` invokes the official command when `codex` is available.

### VS Code

The official user-profile CLI registration is:

```text
code --add-mcp '{"name":"snip-it","command":"/absolute/path/to/snp","args":["mcp","serve"]}'
```

`snp mcp install vscode` invokes `code --add-mcp` with the generated JSON.
The equivalent workspace file is `.vscode/mcp.json`; use the JSON object
printed by `snp mcp instructions vscode`.

### Cursor

Cursor does not provide a stable noninteractive CLI contract for this local
registration. Merge the printed entry into the global `~/.cursor/mcp.json` or
project `.cursor/mcp.json` file. Cursor's official MCP install deeplink flow
is also supported by Cursor, but should be reviewed and accepted by the user.

### OpenCode

OpenCode's current v2 schema stores local servers under `mcp.servers`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "servers": {
      "snip-it": {
        "type": "local",
        "command": ["/absolute/path/to/snp", "mcp", "serve"]
      }
    }
  }
}
```

OpenCode's `opencode mcp add` flow is guided/interactive and its configuration
schema has changed across generations. `snp mcp install opencode` therefore
prints the current v2 block and never writes guessed JSON.

### Zed

Use `zed: open settings file` and merge the generated entry under
`context_servers`:

```json
{
  "context_servers": {
    "snip-it": {
      "command": "/absolute/path/to/snp",
      "args": ["mcp", "serve"],
      "env": {}
    }
  }
}
```

Zed has no stable noninteractive registration command for this local server,
so `snp mcp install zed` prints the block without changing settings.
