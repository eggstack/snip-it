# MCP (Model Context Protocol) Server

[← Back to Overview](overview.md)

## Overview

`snp mcp serve` exposes the local snippet library to AI coding agents through a
**read-only, stdio-only, non-executing** MCP server. The MCP client spawns the
binary and exchanges newline-delimited JSON-RPC 2.0 messages over stdin/stdout;
stdout carries protocol traffic only, diagnostics go to stderr, and EOF ends the
process cleanly.

It deliberately does **not** cover snippet execution (nothing under `src/mcp/`
can run a snippet), library mutation, the user-facing protocol contract
([`docs/MCP.md`](../docs/MCP.md)), the selector/search semantics it borrows
([selector.md](selector.md)), or the diagnostics report types
([diagnostics.md](diagnostics.md)).

**Sources**: `src/mcp/mod.rs` (28 lines), `src/mcp/protocol.rs` (479),
`src/mcp/tools.rs` (307), `src/mcp/client_install.rs` (269).
CLI wiring: `McpCommands` (`src/main.rs:265`), classified `SuppressReadOnly` +
`Minimal` (`src/main.rs:932`).

## Module Map

| File | Role |
|------|------|
| `src/mcp/mod.rs` | Public surface: `serve()`, `instructions()`, `install()`; re-exports `McpClient` |
| `src/mcp/protocol.rs` | JSON-RPC 2.0 newline-delimited stdio transport, handshake, `tools/list`, dispatch |
| `src/mcp/tools.rs` | The three read-only tools — thin adapters over `crate::selector` |
| `src/mcp/client_install.rs` | `McpClient` enum, official-CLI registration, manual setup blocks |

## Transport & Protocol

**File**: `src/mcp/protocol.rs`

### Framing

- One JSON-RPC 2.0 object per line on stdio. `serve()` (`src/mcp/protocol.rs:29`)
  loops on `read_bounded_line()` (`src/mcp/protocol.rs:418`) until EOF, then
  returns `Ok(())`.
- Every response is written as one JSON line terminated by `\n` and flushed
  (`write_json`, `src/mcp/protocol.rs:397`). There is no `Content-Length`
  header and no batching: this is newline-delimited JSON-RPC, not LSP-style
  framing.
- Line terminator: `trim_line()` (`src/mcp/protocol.rs:411`) strips a trailing
  `\n` and then an optional `\r`, so CRLF clients work.
- Size bound: `MAX_MESSAGE_BYTES = 1_048_576` (`src/mcp/protocol.rs:15`). A
  longer line is discarded up to the next newline and replaced by the sentinel
  `{"jsonrpc":"2.0","__oversized":true}` (`src/mcp/protocol.rs:436`), which the
  dispatcher then rejects as `-32600`. One huge line therefore cannot
  desynchronize the stream (`tests/mcp_integration.rs:293-301`).
- An empty line (or a lone `\r`) is answered `-32700` "Empty MCP message"
  (`src/mcp/protocol.rs:40-43`).

### Message Shapes

Request:

```json
{ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
  "params": { "name": "snippets_search", "arguments": { "query": "git" } } }
```

- `jsonrpc` must be the string `"2.0"`; anything else is `-32600`
  (`src/mcp/protocol.rs:72`).
- `method` must be a string; missing → `-32600` (`src/mcp/protocol.rs:81`).
- A request without `id` is a **notification**: it is handled but never
  answered (`src/mcp/protocol.rs:91`).

Success response (`write_result`, `src/mcp/protocol.rs:373`):

```json
{ "jsonrpc": "2.0", "id": 1, "result": { } }
```

Error response (`write_error`, `src/mcp/protocol.rs:380`):

```json
{ "jsonrpc": "2.0", "id": 1, "error": { "code": -32602, "message": "Invalid params" } }
```

`data` is attached only when the caller supplies it.

### Error Codes

| Code | Constant | Raised when |
|------|----------|-------------|
| `-32700` | `PARSE_ERROR` | Line is not valid JSON, or is empty |
| `-32600` | `INVALID_REQUEST` | Not an object, `jsonrpc != "2.0"`, missing `method`, missing `id` where one is required, oversized line, second `initialize` |
| `-32601` | `METHOD_NOT_FOUND` | Method is not one of the five supported methods |
| `-32602` | `INVALID_PARAMS` | Bad params for `initialize` / `tools/list` / `tools/call`, unknown tool name, invalid tool arguments |
| `-32002` | `SERVER_NOT_INITIALIZED` | A `tools/*` request arrives before `initialize` **and** `notifications/initialized` |

Constants at `src/mcp/protocol.rs:17-21`.

### Lifecycle and Capability Advertisement

- Supported methods: `initialize`, `notifications/initialized`, `ping`,
  `tools/list`, `tools/call` (`src/mcp/protocol.rs:94-179`). Anything else →
  `-32601`.
- `ping` is answered with an empty object at any time, even before
  initialization (`src/mcp/protocol.rs:101-106`).
- `initialize` (`src/mcp/protocol.rs:182`) requires `protocolVersion`, an
  object `capabilities`, and an object `clientInfo`; otherwise `-32602`. A
  second `initialize` → `-32600` "Already initialized"
  (`src/mcp/protocol.rs:112-114`).
- Negotiated revisions: `2025-11-25` (`MCP_PROTOCOL_VERSION`) plus `2025-06-18`,
  `2025-03-26`, `2024-11-05` (`src/mcp/protocol.rs:8-14`). An unsupported
  version is rejected with the supported list echoed in `data`
  (`src/mcp/protocol.rs:217-228`).
- The result advertises exactly `{"capabilities": {"tools": {}}}` — no
  `resources`, `prompts`, `sampling`, or `logging` (`src/mcp/protocol.rs:239`).
- `serverInfo` identifies `snip-it` / `CARGO_PKG_VERSION` / "Read-only local
  snippet library" (`src/mcp/protocol.rs:240-244`).
- The `instructions` string names the three tools and states that commands are
  returned as text and never executed (`src/mcp/protocol.rs:245`).
- After `initialize`, every request is refused with `-32002` until the client
  sends `notifications/initialized` (`src/mcp/protocol.rs:94-99`, `:118-129`).

### Tool Dispatch

- `tools/list` rejects non-object params and any `cursor` (pagination is
  unsupported) with `-32602` (`src/mcp/protocol.rs:136-157`).
- `tools/call` requires `params.name`; an unknown name → `-32602` "Unknown
  tool" with the name echoed in `data` (`src/mcp/protocol.rs:314-335`).
- Both success and tool-level failure are wrapped as a **result**, never as a
  JSON-RPC error:

```json
{
  "content": [ { "type": "text", "text": "<the same JSON, compact>" } ],
  "structuredContent": { },
  "isError": false
}
```

  (`src/mcp/protocol.rs:338-366`). `isError` is true when the payload carries
  an `error` key (`tools::is_error`, `src/mcp/tools.rs:208`), or when the tool
  returned `Err` — the latter is rendered as
  `{"error": "data_error", "message": "..."}`.

## Tools

**File**: `src/mcp/tools.rs` — thin JSON adapters over the canonical
`crate::selector` layer, so MCP cannot drift from `snp get` / `snp list`.
All three take an optional `library` (name or `all`); unknown names surface as
MCP `invalid_params` (`src/mcp/tools.rs:253-261`). Deleted snippets are always
filtered out (`src/mcp/tools.rs:67`, `:88`; the selector also excludes them at
`src/selector.rs:385`).

| Tool | Arguments | Behavior |
|------|-----------|----------|
| `snippets_list` | `library?`, `limit?` (default 100, max 1000) | Lists snippet metadata in library order (`tools::list`, `src/mcp/tools.rs:61`) |
| `snippets_search` | `query` (required), `library?`, `limit?`, `tags?`, `search_output?` | Fuzzy search with CLI-consistent relevance ranking (`tools::search`, `src/mcp/tools.rs:74`) |
| `snippet_get` | exactly one of `id` / `description` / `command`, plus `library?` | Single-snippet retrieval with `snp get` identity semantics (`tools::get`, `src/mcp/tools.rs:129`) |

All argument structs carry `#[serde(deny_unknown_fields)]`
(`src/mcp/tools.rs:31`, `:38`, `:53`), and every tool parses arguments through
`parse_arguments()` (`src/mcp/tools.rs:212`), which rejects a non-object
`arguments` value with `-32602`.

### Advertised Schemas

The exact schemas returned by `tools/list` (`src/mcp/protocol.rs:250-302`):

```json
{
  "name": "snippets_search",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "Required fuzzy search query." },
      "library": { "type": "string", "description": "Optional library name; use 'all' for every registered library." },
      "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 },
      "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional explicit tag filter; only snippets carrying every listed tag (case-insensitive) are searched." },
      "search_output": { "type": "boolean", "description": "Include bounded output/notes text in matching (same opt-in contract as 'snp list --search-output').", "default": false }
    },
    "required": ["query"],
    "additionalProperties": false
  }
}
```

```json
{
  "name": "snippet_get",
  "inputSchema": {
    "type": "object",
    "properties": {
      "id": { "type": "string", "description": "Exact snippet ID (case-sensitive)." },
      "description": { "type": "string", "description": "Exact description, case-insensitive; must be unique in the selected scope." },
      "command": { "type": "string", "description": "Exact command text, case-insensitive; must be unique in the selected scope." },
      "library": { "type": "string", "description": "Optional library name; use 'all' for every registered library." }
    },
    "oneOf": [
      { "required": ["id"] },
      { "required": ["description"] },
      { "required": ["command"] }
    ],
    "additionalProperties": false
  }
}
```

`snippets_list` takes only `library` and `limit`
(`src/mcp/protocol.rs:254-263`).

### `snippets_list`

- Loads every visible library through the read-only resolver, then takes the
  first `limit` entries in library order (`src/mcp/tools.rs:61-72`). There is
  **no** fuzzy filter and **no** relevance ranking — this is a plain listing.
- `limit` is bounded to 1–1000 by `bounded_limit()` (`src/mcp/tools.rs:224`); a
  missing limit defaults to 100, and `0` or `> 1000` is `-32602`.

### `snippets_search`

- Builds `SearchFields { include_tags: true, include_output:
  args.search_output.unwrap_or(false) }` (`src/mcp/tools.rs:79-82`), scores via
  `searchable_text()` + `shared_fuzzy_matcher()`
  (`src/mcp/tools.rs:99-106`), and ranks with
  `rank_snippets(..., SnippetSort::Relevance)` (`src/mcp/tools.rs:111-120`) —
  the same ranking path as the CLI.
- The optional `tags[]` argument pre-filters with `matches_required_tags()`:
  a snippet is kept only if it carries **every** listed tag, compared
  case-insensitively (`src/mcp/tools.rs:91`; `src/selector.rs:224`).
- An empty `query` matches everything (up to `limit`) with no scores
  (`src/mcp/tools.rs:95-98`).

### `snippet_get`

- Enforces **exactly one** of `id` / `description` / `command`; zero or two or
  more is `-32602` "Provide exactly one of 'id', 'description', or 'command'"
  (`src/mcp/tools.rs:131-143`). The schema's `oneOf` is the client-side mirror
  of the same rule.
- Builds a `SnippetSelector` with `ResolutionPolicy::All` and the library scope
  from `LibraryScope::from_filter_arg()` (`src/mcp/tools.rs:145-147`), then
  resolves through `resolve_selector_readonly()` (`src/mcp/tools.rs:160`).
- `ResolutionPolicy::All` yields `SelectionResult::Many` for ambiguity so MCP
  can report structured `ambiguous` with match identities — it never prompts
  and never expands variables (`src/mcp/tools.rs:157-159`).

### Result JSON

Snippet payload (`snippet_json`, `src/mcp/tools.rs:263`):

```json
{ "id": "…", "library": "work", "description": "…", "command": "…",
  "tags": [], "folders": [], "favorite": false }
```

Identity payload for ambiguity (`snippet_identity_json`, `src/mcp/tools.rs:275`):

```json
{ "id": "…", "library": "work", "description": "…" }
```

Envelope shapes:

| Outcome | Payload |
|---------|---------|
| `snippets_list` / `snippets_search` | `{ "snippets": [ <snippet payload>, … ] }` |
| `snippet_get`, exactly one match | the snippet payload directly |
| `snippet_get`, zero matches | `{ "error": "not_found", "message": "No matching snippet was found", "matches": [] }` |
| `snippet_get`, several matches | `{ "error": "ambiguous", "message": "The selector matched more than one snippet", "matches": [ <identity payload>, … ] }` |
| tool returned `Err` | `{ "error": "data_error", "message": "…" }` |

The snippet payload never carries `output`/notes text, sync metadata,
credentials, or keychain data. `output`/notes are searchable (opt-in) but are
never returned.

## Read-Only and Non-Executing Guarantees

These are enforced by construction, not by convention:

- **No execution.** There is no `std::process::Command`, no `spawn`, and no
  shell expansion anywhere in the server path. The single `Command::new` in the
  whole module tree is `src/mcp/client_install.rs:67` — the *MCP client's* own
  registration CLI, reached only from `snp mcp install`, never from
  `snp mcp serve`. A snippet's `command` is only ever returned as a string.
- **No mutation.** `src/mcp/` contains no `fs::write`, `File::create`, or
  `OpenOptions`. Snippets are read through `crate::library::load_library()`,
  which parses and returns; it never writes.
- **No recovery / no migration.** The read path uses the **side-effect-free**
  resolver: `resolve_selector_readonly()` (`src/selector.rs:645`) and
  `library::readonly_library_sources()` (`src/library/manager.rs:815`), never
  `resolve_selector()` (`src/selector.rs:703`), which calls
  `LibraryManager::ensure_library_mode()` and can migrate a legacy
  single-file checkout, create `libraries/`, copy the legacy file, write
  `libraries.toml`, and bump the generation. MCP never calls the mutating
  resolver, so an agent's read cannot trigger that recovery path.
  Concretely: `snippet_get` resolves through `resolve_selector_readonly()`
  (`src/mcp/tools.rs:160`), while `snippets_list` and `snippets_search` load
  through `load_snippets()` (`src/mcp/tools.rs:234`) → `library_sources()`
  (`src/mcp/tools.rs:253`) → `library::readonly_library_sources()`. Neither
  path reaches `resolve_selector()`.
- **No prompting.** `ResolutionPolicy::All` + `resolve_selector_readonly`
  returns `NotFound` / `One` / `Many` / `Ambiguous` as data; there is no TUI,
  no stdin prompt, and no auto-expand of variables.
- **No background work.** `snp mcp …` is classified `SuppressReadOnly` +
  `Minimal` (`src/main.rs:932`), so no auto-sync worker is spawned, no network
  connection is opened, and no recovery runs at startup.

## Search Parity

All three search surfaces share `selector::searchable_text()`
(`src/selector.rs:142`) and the `SearchFields` flags (`src/selector.rs:91`):

| Surface | Entry point | Fields |
|---------|-------------|--------|
| `snp get --query` | `SnippetSelector::resolve` → `score_fuzzy_matches(..., SearchFields::fuzzy_default())` (`src/selector.rs:441`) | description + command + tags |
| `snp list --filter` | `src/commands/list_cmd.rs:93` with `SearchFields { include_tags: true, include_output: search_output }` (`src/commands/list_cmd.rs:80`) | description + command + tags (+ output/notes with `--search-output`) |
| MCP `snippets_search` | `src/mcp/tools.rs:99` with the same two flags | description + command + tags (+ output/notes with `search_output: true`) |

- `description` and `command` always participate.
- `tags` participate by default (`include_tags: true`); `snippets_search`
  additionally supports the explicit `tags[]` must-carry-every-tag filter.
- `output`/notes participate only opt-in (`include_output`), bounded to a
  512-char `OutputPresentation::for_scoring()` summary
  (`src/selector.rs:171`) — the same contract as `snp list --search-output` and
  MCP `search_output: true`.
- `folders`, `favorite`, sync metadata, credentials, and keychain data are
  **never** searchable.

Library source resolution is shared through the canonical
`library::readonly_library_sources()` resolver (`src/mcp/tools.rs:253`):
legacy single-file mode reads as the implicit `snippets` library without
triggering the CLI migration write path, and malformed TOML fails closed
through the normal library loader (`tests/mcp_integration.rs:309-331`).

> **Adding a field changes three surfaces with no compiler error.** A new
> searchable field must go through `SearchFields`, and its default must move
> the CLI flag and the MCP `search_output` parameter together — see
> [.skills/selector-and-search-parity.md](../.skills/selector-and-search-parity.md).

## `snippet_get` Identity Contract

Mirrors `snp get` exactly (`src/mcp/tools.rs:129-206`; matching rules at
`src/selector.rs:376-436`):

- `id` — exact match, **case-sensitive** (`s.id == id.as_str()`,
  `src/selector.rs:391`).
- `description` — exact match, case-insensitive
  (`eq_ignore_ascii_case`, `src/selector.rs:422`), must be unique in scope.
- `command` — exact match, case-insensitive (`eq_ignore_ascii_case`,
  `src/selector.rs:432`), must be unique in scope.
- Exactly one of the three must be supplied (`oneOf` in the input schema,
  `src/mcp/protocol.rs:292-296`, enforced again in `tools::get`).
- Zero matches → `not_found`; multiple → `ambiguous` with identities.
- The selector's precedence is ID → description → command, so `id` wins if more
  than one is somehow supplied.

## Client Install Helpers

**File**: `src/mcp/client_install.rs` — `McpClient` (`src/mcp/client_install.rs:11`):
`claude`, `codex`, `vscode`, `cursor`, `opencode`, `zed`.

- `print_instructions()` (`src/mcp/client_install.rs:50`) prints the
  absolute-path setup block derived from `current_exe()`
  (`src/mcp/client_install.rs:103`).
- `install()` (`src/mcp/client_install.rs:56`) invokes the client's official
  noninteractive registration command **only when one exists**
  (`official_invocation`, `src/mcp/client_install.rs:108-137`):

| Client | Official invocation |
|--------|--------------------|
| Claude Code | `claude mcp add snip-it --scope user -- <exe> mcp serve` |
| Codex | `codex mcp add snip-it -- <exe> mcp serve` |
| VS Code | `code --add-mcp <json>` |
| Cursor, OpenCode, Zed | **none** — `official_invocation` returns `None` |

  The gate is exactly "the client's official CLI exposes a stable
  noninteractive registration command". For Cursor, OpenCode, and Zed it does
  not, so `install()` changes nothing and prints the manual block instead
  (OpenCode v2 `mcp.servers` JSON, Zed `context_servers`, Cursor `mcpServers`)
  — `src/mcp/client_install.rs:58-65`, `:171-215`.
- If the client CLI is absent (`ErrorKind::NotFound`), nothing is changed and
  the manual instructions are printed (`src/mcp/client_install.rs:86-93`). A
  non-zero exit from the client CLI is a hard error
  (`src/mcp/client_install.rs:78-85`).
- The child process is spawned with `stdin(Stdio::null())` and inherited
  stdout/stderr (`src/mcp/client_install.rs:70-72`).
- `shell_quote()` (`src/mcp/client_install.rs:226`) quotes the executable path
  for the printed instructions when it contains anything outside
  `[A-Za-z0-9._/-\:]`.

## CLI Surface

`McpCommands` (`src/main.rs:265`) exposes three subcommands, all classified
`SuppressReadOnly` + `Minimal` (`src/main.rs:932`) so agent reads never spawn
workers, open network connections, or create log/config state:

- `snp mcp serve` — run the stdio server until EOF (`mcp::serve()`,
  `src/mcp/mod.rs:14`, `src/main.rs:858`).
- `snp mcp instructions <client>` — print the absolute-path setup block for one
  of the six supported clients (`src/main.rs:859`).
- `snp mcp install <client>` — invoke the official noninteractive registration
  when one exists, else print manual setup without changing anything
  (`src/main.rs:860`).

## Tests

`tests/mcp_integration.rs` (331 lines) drives a real `snp mcp serve` child
process over pipes:

| Test | Protects |
|------|----------|
| `initialize_and_tools_list_follow_mcp_handshake` (`:147`) | handshake, capability advertisement, tool list |
| `list_search_get_are_read_only_and_structured` (`:178`) | tool dispatch, parameter schemas, structured results, read-only boundary |
| `malformed_and_oversized_messages_return_errors_without_crashing` (`:267`) | `-32700` / `-32602` / `-32600` error codes, oversized-line recovery |
| `malformed_library_fails_closed_as_tool_error` (`:310`) | malformed library TOML surfaces as a tool error, not a crash |

Unit tests live next to the code: `src/mcp/tools.rs:296` (limit bounds,
object-only arguments), `src/mcp/client_install.rs:243` (official invocations
use exact server arguments), and `src/mcp/protocol.rs:472` (bounded reader
discards oversized lines).

## Key Files

- `src/mcp/mod.rs` — public surface (`serve`, `instructions`, `install`)
- `src/mcp/protocol.rs` — JSON-RPC framing, handshake, `tools/list`, dispatch, error codes
- `src/mcp/tools.rs` — the three read-only tools and their argument schemas
- `src/mcp/client_install.rs` — `McpClient`, official-CLI gate, manual setup blocks
- `src/selector.rs:142` — `searchable_text`, the shared search contract
- `src/selector.rs:645` — `resolve_selector_readonly`, the side-effect-free resolver MCP must use
- `src/library/manager.rs:815` — `readonly_library_sources`, the read-only library resolver
- `src/main.rs:265` — `McpCommands`; `src/main.rs:932` — read-only service classification
- `tests/mcp_integration.rs` — end-to-end protocol contract

## Invariants

- Read-only: no execution, no mutation, no prompting, no auto-sync trigger.
- Stdio-only: stdout carries only JSON-RPC; diagnostics go to stderr.
- Search parity via `selector::searchable_text` — adding a searchable field
  must go through `SearchFields`, never a tool-local copy.
- Results never include output/notes text, sync metadata, or credentials.
- `deny_unknown_fields` on all tool argument structs
  (`src/mcp/tools.rs:31-59`); `limit` is bounded to 1–1000
  (`src/mcp/tools.rs:224`).
- Exactly three tools. Adding a fourth must keep the read-only /
  non-executing contract.

## Gotchas

- **The read-only resolver is a hard requirement, not a preference.**
  `resolve_selector()` (`src/selector.rs:703`) calls `ensure_library_mode()`,
  which migrates a legacy single-file checkout and rewrites metadata. MCP uses
  `resolve_selector_readonly()` (`src/selector.rs:645`) instead. Swapping them
  reintroduces recovery side effects into an agent-facing read path.
- **`readonly_library_sources` avoids *library* migration, not *every* write.**
  `LibraryManager::new()` (`src/library/manager.rs:43`) can still move a legacy
  macOS config directory and can write a best-effort `libraries.toml.corrupt`
  backup when the index is malformed. Neither migrates or rewrites library
  metadata; the point is that no `ensure_library_mode()` runs.
- **The advertised `oneOf` is not the enforcement.** `snippet_get` re-checks
  exactly-one-of in Rust (`src/mcp/tools.rs:131-143`); the schema alone would
  let a client bypass it.
- **Tool errors are results, not JSON-RPC errors.** A malformed library or an
  unknown library name comes back as `isError: true` with a `data_error`
  payload, so a client must inspect `isError` / `structuredContent.error`
  rather than only `error`.
- **Oversized input is answered `-32600`, not `-32700`.** The sentinel line is
  valid JSON, so it reaches the request validator
  (`src/mcp/protocol.rs:436`, `tests/mcp_integration.rs:301`).
- **Adding a searchable field changes three surfaces with no compiler error** —
  see [Search Parity](#search-parity).
- **`snp search --library all` errors by design** while `snp get --library all`,
  `snp list --library all`, and MCP's `library: "all"` work; `all` is a
  reserved library name (`src/library/model.rs:218`).

---

## Cross-References

- [selector.md](selector.md) — the resolver and search contract MCP borrows.
- [diagnostics.md](diagnostics.md) — the shared report types (MCP has none of
  its own; it returns raw JSON values).
- [cli.md](cli.md) — the CLI surface and exit codes.
- [.skills/selector-and-search-parity.md](../.skills/selector-and-search-parity.md)
  — the hard MCP boundaries (stdio-only, read-only, non-executing) and the
  search-parity obligation against `snp get --query` / `snp list --filter`.
- [`docs/MCP.md`](../docs/MCP.md) — the user-facing protocol contract.
