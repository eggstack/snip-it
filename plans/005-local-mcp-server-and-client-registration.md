# Plan 005: local MCP server and client registration

Status: planned

Depends on: Plan 002

## Objective

Expose the local snippet library to coding agents through a lightweight stdio MCP server and make registration with common clients straightforward.

The MCP process is launched on demand by the client:

```text
snp mcp serve
```

Do not run the MCP adapter at boot, bind a TCP port, or add another daemon.

## Why stdio

The target coding-agent clients already support launching local MCP commands over stdio. This provides the cleanest fit for a local snippet database:

```text
agent starts: snp mcp serve
JSON-RPC over stdin/stdout
agent exits -> child exits
```

No TLS, socket discovery, PID files, service units, or independent updater are required.

`snip-sync` remains the only long-running server in this repository.

## Dependency policy

Prefer implementing the small MCP stdio surface using dependencies already present (`serde`, `serde_json`) rather than adding a large MCP SDK/runtime solely for three read-only tools.

Before implementing manually, check the current official MCP transport/message requirements and the smallest maintained Rust SDK footprint. Use an SDK only if it clearly reduces protocol risk without materially increasing binary size or runtime complexity.

If implementing directly, keep protocol code isolated in:

```text
src/mcp/mod.rs
src/mcp/protocol.rs
src/mcp/tools.rs
src/mcp/client_install.rs   # or equivalent
```

Do not mix JSON-RPC parsing into the normal CLI command modules.

## CLI surface

Suggested structure:

```text
snp mcp serve
snp mcp instructions <client>
snp mcp install <client>
```

Candidate client enum for the first pass:

```text
claude
codex
vscode
cursor
opencode
zed
```

`instructions` is always read-only.

`install` should use an official client CLI when that client exposes a stable noninteractive MCP-add command. For clients without a stable CLI/config mutation contract, print exact instructions rather than guessing and corrupting user config.

Do not create a generic config-editor framework.

## Protocol requirements

At minimum support the stdio request sequence needed by mainstream MCP clients:

```text
initialize
notifications/initialized
ping                          if required/expected by clients
tools/list
tools/call
```

Respond with valid JSON-RPC 2.0 IDs/errors.

Protocol-version handling must be explicit:

- define the protocol version(s) this implementation supports;
- negotiate/respond according to the current official MCP initialization rules at implementation time;
- reject unsupported mandatory protocol behavior clearly;
- do not claim capabilities that are not implemented.

Stdout is protocol-only while serving. Send diagnostics/logging to stderr so a log line can never corrupt the JSON-RPC stream.

Bound input message size to a reasonable value for local MCP control messages. Reject malformed JSON/oversized requests without panicking.

EOF should shut the server down cleanly.

## Initial tool surface

Keep the first release deterministic and read-only.

Recommended tools:

### `snippets_list`

Purpose: enumerate snippet metadata without executing anything.

Arguments may include:

```text
library: optional library name
limit: bounded optional count
```

Return stable metadata such as ID, description, tags, library, and command text only when explicitly appropriate to the tool contract.

### `snippets_search`

Purpose: search snippets deterministically.

Arguments:

```text
query: required string
library: optional
limit: bounded
```

Reuse existing ranking/search primitives. Do not create an MCP-specific fuzzy algorithm that drifts from the CLI.

### `snippet_get`

Purpose: retrieve one snippet by deterministic identity.

Preferred selectors:

```text
id
or exact description where existing selector semantics can prove uniqueness
```

Reuse existing deterministic selector behavior and return an ambiguity/not-found error rather than opening a TUI.

### Optional `libraries_list`

Add only if it reduces client round trips and can reuse existing library metadata cleanly.

## Explicitly out of scope for first MCP release

Do not expose:

- arbitrary snippet execution;
- `snp run` semantics;
- shell execution;
- write/create/edit/delete tools;
- sync credential access;
- API keys;
- server administration;
- backup/restore;
- unrestricted filesystem paths.

An agent that wants to execute a returned command already has its own shell/tool approval model. Duplicating execution inside MCP would bypass or complicate that policy surface.

A later plan can add opt-in mutation tools after the read-only adapter is proven useful.

## Data access and locking

Use existing library/config loading APIs and existing local data locking semantics where needed.

The MCP server is another local reader. It must:

- fail closed on malformed TOML exactly like the CLI;
- never synthesize and save an empty library after parse failure;
- respect library metadata and deterministic resolution;
- avoid taking mutation locks for read-only operations unnecessarily;
- never trigger auto-sync solely because an MCP client reads snippets.

Keep the Tokio runtime uninitialized if all MCP tools are local/read-only and synchronous. Do not make MCP serving pull in async runtime work unless the protocol implementation actually requires it.

## Output schema

Return machine-oriented JSON objects inside MCP tool results with stable field names.

Example conceptual result:

```json
{
  "id": "...",
  "library": "work",
  "description": "Deploy service",
  "command": "...",
  "tags": ["deploy"]
}
```

Do not expose local-only metadata unless it is useful to agents and intentionally documented.

No secret/keychain fields may enter MCP output.

## Client registration behavior

### Claude Code

If the installed Claude CLI supports a stable noninteractive local MCP add command, invoke it with the absolute/current `snp` executable and arguments `mcp serve`.

Otherwise `instructions claude` prints the exact current official command.

### Codex

Prefer the official `codex mcp add` command when available. Use the exact `snp` executable path so PATH differences between interactive shell and agent launch do not break registration.

### VS Code

Prefer an official CLI MCP registration mechanism (`code --add-mcp` or the current supported equivalent) when present. Otherwise print the exact `.vscode/mcp.json`/user configuration object according to the current documented schema.

### Cursor

Use an official MCP install/deeplink/config mechanism only if its current schema is stable and testable. Otherwise print the configuration block and target file/location.

### OpenCode

OpenCode MCP configuration schemas have changed across generations. Detect the installed version/schema before modifying configuration. If that cannot be done confidently, `install opencode` should degrade to precise instructions rather than write guessed JSON.

### Zed

Print or apply the current supported context-server/MCP configuration only after verifying the current schema. Prefer Zed's own extension/command integration if a stable CLI exists.

## Mutation safety for client configs

For any client where `snp` writes a config file directly:

1. locate the official user/project config path through documented client behavior;
2. parse existing JSON/JSONC/TOML using a format-preserving approach if available;
3. add/update only the named `snip-it` MCP entry;
4. preserve unrelated configuration;
5. write atomically;
6. create a backup only if the project's existing config-edit policy already uses one;
7. refuse malformed config rather than replacing it.

Avoid direct config mutation when the client provides an official CLI that already owns this logic.

## Installer integration

Plan 002 does not automatically register MCP in every installed agent. That would be surprising and may mutate multiple unrelated configs.

After installing `snp`, the bootstrap script may print a concise next step:

```text
Agent integration: snp mcp instructions <client>
```

For fleet automation, the operator can run an explicit `snp mcp install <client>` after the binary install.

## Tests

Protocol tests should launch the real `snp mcp serve` child with an isolated config directory and exchange JSON-RPC over pipes.

Required cases:

- initialize success;
- unsupported/malformed initialize request;
- tools/list contains exactly the intended first-pass tools;
- snippets_list returns isolated fixture snippets;
- snippets_search ordering matches existing ranking semantics;
- snippet_get exact ID success;
- not-found/ambiguous errors are structured and do not launch TUI;
- malformed library fails closed;
- oversized/malformed JSON request returns protocol error/no panic;
- stderr diagnostics do not appear on stdout;
- EOF exits cleanly;
- no tool causes shell execution.

Client-install tests should exercise pure command/config rendering without requiring real user installations. Where a client CLI exists in CI/test fixtures, verify the generated invocation shape.

## Documentation

Add a concise top-level README section:

```text
Agent / MCP integration
snp mcp serve
snp mcp instructions claude
snp mcp instructions codex
...
```

Detailed per-client commands belong in `USER_GUIDE.md` or a focused `docs/MCP.md` if the section becomes large.

Do not remove or replace the existing demo GIF while editing the README.

## Acceptance criteria

1. `snp mcp serve` runs as a local stdio MCP server with protocol-only stdout.
2. Mainstream MCP clients can initialize it and list/call the supported tools.
3. The first tool set is read-only and does not execute stored commands.
4. Search/get semantics reuse existing deterministic snippet/library behavior.
5. Malformed local data fails closed exactly as normal CLI reads do.
6. MCP serving does not initialize unrelated sync/TUI runtime machinery.
7. `snp mcp instructions <client>` provides accurate current setup for Claude, Codex, VS Code, Cursor, OpenCode, and Zed.
8. `snp mcp install <client>` uses official client CLI/config mechanisms only where safe; otherwise it prints instructions rather than guessing.
9. Config mutation, where implemented, preserves unrelated user settings and is atomic.
10. README/user docs expose the MCP path and retain the existing demo GIF.