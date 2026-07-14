# Usage Metadata Module

## Purpose

Tracks local-only per-snippet usage statistics: how often and when each snippet has been used. The data is intentionally isolated from the snippet library — no command bodies are logged and no remote sync is performed.

## Module: `src/usage.rs`

### Core Types

- `UsageIndex` — Persistent index of all usage entries, serialized as TOML.
- `UsageEntry` — Single entry: snippet ID, use count, last-used timestamp.
- `UsageData` — Read-only view returned by `get_usage()` (no borrow on the index).

### Storage

- **Path**: `~/.config/snp/usage.toml`
- **Format**: TOML array-of-tables (`[[usage]]`)
- **Atomic writes**: Uses `write_private_atomic()` for crash-safe persistence
- **Permissions**: Private (0600) via the atomic write helper

### Key Methods

| Method | Behavior |
|--------|----------|
| `UsageIndex::load()` | Load from disk; returns empty index if file missing or corrupt (fail-open) |
| `UsageIndex::save()` | Atomic write to disk |
| `UsageIndex::record_use(id)` | Increment count, set timestamp to now; creates entry if new |
| `UsageIndex::get_usage(id)` | Return `UsageData` (zeroed defaults for unknown IDs) |
| `UsageIndex::prune(active_ids)` | Remove entries not in the active set (lazy cleanup) |

### Update Policy

| Action | Count update | Last-used update |
|--------|:-----------:|:----------------:|
| Successful `run` | yes | yes |
| Failed `run` | no | no |
| Cancelled `run` | no | no |
| Successful `clip` | yes | yes |
| Cancelled `clip` | no | no |
| `select` | yes | yes |
| `search`/`list`/`preview` | no | no |
| `edit`/`import`/`doctor` | no | no |

### Security Properties

- No command bodies or output metadata are stored — only the snippet UUID, count, and timestamp.
- Usage data is never synchronized to the server.
- Usage data is never included in JSON/CSV export of snippets.
- The usage file uses private permissions (0600).
- Corrupt files fail open to an empty index (no crash, no data loss).

### Identity Stability

Usage entries are keyed by snippet UUID (`id` field). When snippets are imported, renamed, or reordered, the UUID remains stable. Entries for deleted snippets are lazily pruned by `prune()`.

## Integration Points

- **Sort module** (`src/sort.rs`): Reads `UsageData` for `LastUsed` and `MostUsed` sort modes in `rank_snippets()`.
- **TUI** (`src/ui/mod.rs`): `UsageIndex` is loaded once per selection session in `run_snippet_selection()` and passed via `SnippetListParams.usage`. The `sort_filtered_indices()` function uses real usage data for `LastUsed` and `MostUsed` interactive sort modes.
- **Run command** (`src/commands/run_cmd.rs`): Calls `record_use()` after successful execution.
- **Clip command** (`src/commands/clip_cmd.rs`): Calls `record_use()` after successful clipboard copy.
- **Select command** (`src/commands/select_cmd.rs`): Calls `record_use()` after successful selection.

## Test Coverage

6 unit tests covering: load missing file, record increment, timestamp accuracy, save/load roundtrip, corrupt file fail-open, prune stale entries, and default for unknown IDs. Integration tests verify usage tracking through PTY end-to-end tests.
