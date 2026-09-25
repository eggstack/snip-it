# MCP (Model Context Protocol) Server

[← Back to Overview](overview.md)

## Overview

`snp mcp` exposes the local snippet library to AI coding agents through a
**read-only, stdio-only, non-executing** MCP server. An MCP client spawns
`snp mcp serve` on demand and exchanges newline-delimited JSON-RPC 2.0
messages over stdin/stdout; stdout is protocol-only, diagnostics go to
stderr, and EOF shuts the server down cleanly. The adapter never invokes a
shell, never mutates snippets, never prompts, and never triggers auto-sync.

**Sources**: `src/mcp/mod.rs` (28 lines), `src/mcp/protocol.rs` (479),
`src/mcp/tools.rs` (307), `src/mcp/client_install.rs` (269).
User-facing contract: `docs/MCP.md`. CLI wiring: `McpCommands`
(`src/main.rs:238`) with `Serve` / `Instructions` / `Install`, classified
`SuppressReadOnly` + `Minimal` services (`src/main.rs:894`).

## Transport & Protocol

**File**: `src/mcp/protocol.rs`

- Framing: one JSON-RPC 2.0 object per line on stdio; `serve()` loops on
  `read_bounded_line()` (`src/mcp/protocol.rs:29,418`) until EOF.
- Bound: requests over 1 MiB (`MAX_MESSAGE_BYTES`, `src/mcp/protocol.rs:15`)
  are discarded to the next newline and answered as an oversized-message
  error, so one huge line cannot desynchronize the stream.
- Protocol revisions: `2025-11-25` (current `MCP_PROTOCOL_VERSION`) plus
  `2025-06-18`, `2025-03-26`, `2024-11-05`
  (`src/mcp/protocol.rs:8-14`). Unknown versions are rejected with the
  supported list echoed in `data`.
- Methods: `initialize`, `notifications/initialized`, `ping`, `tools/list`,
  `tools/call` (`src/mcp/protocol.rs:94-180`). Anything else is
  `METHOD_NOT_FOUND` (-32601); pre-initialization requests get
  `SERVER_NOT_INITIALIZED` (-32002); notifications (no `id`) never produce a
  response. Standard codes: -32700 parse, -32600 invalid request, -32602
  invalid params (`src/mcp/protocol.rs:17-21`).
- Capability advertisement: `initialize()` (`src/mcp/protocol.rs:182`)
  requires `protocolVersion` + object `capabilities` + `clientInfo`, echoes
  the negotiated version, advertises `"capabilities": { "tools": {} }`,
  identifies as `snip-it` / `Read-only local snippet library`, and carries a
  `instructions` string stating commands are returned as text and never
  executed (`src/mcp/protocol.rs:234-248`).
- `tools/list` rejects non-object params and any `cursor` (pagination is
  unsupported, `src/mcp/protocol.rs:136-157`); `tools/call` dispatches by
  name to `tools::{list,search,get}` and wraps both success and data-error
  outcomes as `content[]` + `structuredContent` + `isError`
  (`src/mcp/protocol.rs:304`).

## Tools

**File**: `src/mcp/tools.rs` — thin JSON adapters over the canonical
`crate::selector` layer, so MCP cannot drift from `snp get` / `snp list`.
All three take an optional `library` (name or `all`); unknown names surface
as MCP `invalid_params`. Deleted snippets are always filtered out.

| Tool | Arguments | Behavior |
|------|-----------|----------|
| `snippets_list` | `library?`, `limit?` (default 100, max 1000) | Lists snippet metadata in library order (`tools::list`, `src/mcp/tools.rs:61`) |
| `snippets_search` | `query` (required), `library?`, `limit?`, `tags?`, `search_output?` | Fuzzy search with CLI-consistent relevance ranking (`tools::search`, `src/mcp/tools.rs:74`) |
| `snippet_get` | exactly one of `id` / `description` / `command`, plus `library?` | Single-snippet retrieval with `snp get` identity semantics (`tools::get`, `src/mcp/tools.rs:129`) |

- `snippets_search` builds `SearchFields { include_tags: true,
  include_output: search_output.unwrap_or(false) }`
  (`src/mcp/tools.rs:79-82`), scores via `searchable_text()` +
  `shared_fuzzy_matcher()`, ranks with `rank_snippets(...,
  SnippetSort::Relevance)` (`src/mcp/tools.rs:99-120`), and pre-filters on
  `matches_required_tags()` when `tags` is given (case-insensitive exact
  equality per tag, `src/mcp/tools.rs:91`). An empty query matches
  everything (up to `limit`).
- `snippet_get` enforces exactly-one-of (`src/mcp/tools.rs:131-143`),
  resolves through `SnippetSelector` + `resolve_selector_readonly()` under
  `ResolutionPolicy::All` (`src/mcp/tools.rs:145-165`), and returns either
  the snippet, a structured `not_found` error, or a structured `ambiguous`
  error with match identities — never prompting or expanding variables.
- Result shape: `id`, `library`, `description`, `command`, `tags`,
  `folders`, `favorite` (`snippet_json`, `src/mcp/tools.rs:263`). Output /
  notes, sync metadata, credentials, and keychain data are never exposed.

## Search Parity

All search entry points share `selector::searchable_text()`
(`src/selector.rs:135`) and `SearchFields` (`src/selector.rs:91`):

- `description` + `command` always participate.
- `tags` participate by default (`include_tags: true`); `snippets_search`
  additionally supports the explicit `tags[]` must-carry-every-tag filter.
- `output`/notes participate only opt-in (`include_output`), bounded to a
  512-char `OutputPresentation::for_scoring()` summary — the same contract as
  `snp list --search-output` and MCP `search_output: true`.
- `folders`, `favorite`, sync metadata, credentials, and keychain data are
  never searchable.

Library source resolution is shared through the canonical
`library::readonly_library_sources()` resolver (`src/mcp/tools.rs:253`):
legacy single-file mode reads as the implicit `snippets` library without
triggering the CLI migration write path, and malformed TOML fails closed
through the normal library loader.

## `snippet_get` Identity Contract

Mirrors `snp get` exactly (`docs/MCP.md`, `src/mcp/tools.rs:129-206`):

- `id` — exact match, **case-sensitive**.
- `description` — exact match, case-insensitive, must be unique in scope.
- `command` — exact match, case-insensitive, must be unique in scope.
- Exactly one of the three must be supplied (`oneOf` in the input schema,
  `src/mcp/protocol.rs:292-296`, enforced again in `tools::get`).
- Zero matches → `not_found`; multiple → `ambiguous` with identities.

## Client Install Helpers

**File**: `src/mcp/client_install.rs` — `McpClient` (`src/mcp/client_install.rs:11`):
`claude`, `codex`, `vscode`, `cursor`, `opencode`, `zed`.

- `print_instructions()` prints the absolute-path setup block derived from
  `current_exe()` (`src/mcp/client_install.rs:50,103`).
- `install()` (`src/mcp/client_install.rs:56`) invokes the client's official
  noninteractive command only when one exists
  (`official_invocation`, `src/mcp/client_install.rs:108`): Claude Code
  (`claude mcp add … --scope user -- <exe> mcp serve`), Codex
  (`codex mcp add …`), VS Code (`code --add-mcp <json>`). Cursor, OpenCode,
  and Zed have no stable noninteractive contract, so `install` changes
  nothing and prints the manual block instead (OpenCode v2
  `mcp.servers` JSON, Zed `context_servers`, Cursor `mcpServers`).
- If the client CLI is absent (`NotFound`), nothing is changed and the
  manual instructions are printed (`src/mcp/client_install.rs:86-93`).

## CLI Surface

`McpCommands` (`src/main.rs:238`) exposes three subcommands, all classified
`SuppressReadOnly` + `Minimal` so agent reads never spawn workers, open
network connections, or create log/config state:

- `snp mcp serve` — run the stdio server until EOF (`mcp::serve()`,
  `src/mcp/mod.rs:14`, `src/main.rs:820`).
- `snp mcp instructions <client>` — print the absolute-path setup block
  for one of the six supported clients (`src/main.rs:821`).
- `snp mcp install <client>` — invoke the official noninteractive
  registration when one exists, else print manual setup without changing
  anything (`src/main.rs:822`).

## Invariants

- Read-only: no execution, no mutation, no prompting, no auto-sync trigger.
- Stdio-only: stdout carries only JSON-RPC; diagnostics go to stderr.
- Search parity via `selector::searchable_text` — adding a searchable field
  must go through `SearchFields`, never a tool-local copy.
- Results never include output/notes text, sync metadata, or credentials.
- `deny_unknown_fields` on all tool argument structs
  (`src/mcp/tools.rs:31-59`); `limit` is bounded to 1–1000
  (`src/mcp/tools.rs:224`).
