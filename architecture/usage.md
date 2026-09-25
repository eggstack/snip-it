# Usage Metadata Module

[← Back to Overview](overview.md)

## Overview

Local-only per-snippet usage statistics (`src/usage.rs`, ~283 lines):
how often and when each snippet ran. Isolated from the snippet library
(no command bodies logged) and never synced — feeds `LastUsed` /
`MostUsed` sort modes and nothing else.

## Key types — `src/usage.rs:26-54`

```rust
pub struct UsageData { pub use_count: u64, pub last_used_at: Option<i64> }
pub struct UsageIndex { entries: Vec<UsageEntry> } // private field; borrow-free reads
pub struct UsageEntry { pub id: String, pub use_count: u64, pub last_used_at: Option<i64> }
```

`UsageData` is a plain copy-out struct so callers never hold a borrow on
the index. File format is a TOML array-of-tables (`[[usage]]` with
`id` / `use_count` / `last_used_at`).

## Key functions — `src/usage.rs:61-163`

```rust
pub fn UsageIndex::load() -> Self                    // fail-open; see below
pub fn UsageIndex::save(&self) -> SnipResult<()>     // local-data lock + atomic write
pub fn UsageIndex::record_use(&mut self, snippet_id: &str) // saturating +1, now() stamp
pub fn UsageIndex::get_usage(&self, snippet_id: &str) -> UsageData // zeroed default if unknown
pub fn UsageIndex::prune(&mut self, active_ids: &[String]) // retain live IDs (lazy cleanup)
pub fn UsageIndex::entries(&self) -> &[UsageEntry]
```

- **Storage**: `~/.config/snp/usage.toml`, `write_private_atomic()`
  (0600). `load_from(path)` / `save_to(path)` take explicit paths so
  tests avoid env-var races.
- **Corrupt file fails open**: warn + single sibling backup
  (`usage.toml.corrupt.bak`, overwritten on repeat) + empty index —
  unlike library load, never refuses to proceed (`:127-156`).
- **Concurrency**: read-modify-write with atomic rename — never corrupt,
  but concurrent `record_use` can lose an increment (last-writer-wins).
  Accepted: personal tool, negligible impact, no locking/DB.

### Update policy

| Action | Count | Last-used |
|--------|:-----:|:---------:|
| Successful `run` | yes | yes |
| Failed / cancelled `run` | no | no |
| Successful `clip` | yes | yes |
| Cancelled `clip` | no | no |
| `select` | yes | yes |
| `search` / `list` / preview | no | no |
| `edit` / `import` / `doctor` | no | no |

Identity key is the snippet UUID — stable across rename/reorder.
Orphan entries (deleted snippets) are pruned lazily via
`library::find_orphaned_ids()` classification shared with
`validate`/`repair`.

## Invariants / gotchas (from AGENTS.md)

- **Local-only, never synced**: not in `ProtoSnippet`, never uploaded /
  downloaded, never in JSON/CSV snippet exports, never searchable
  (selector contract excludes it).
- Only UUID + count + timestamp stored — no bodies, no output metadata.
- Sort reads usage; usage never writes sort state; default `Relevance`
  ignores usage entirely (compatibility-first).

## File / line refs

- `src/usage.rs:26-54` (types), `:56-59` (`usage_path`),
  `:61-123` (`load`/`save`/`record_use`/`get_usage`/`prune`),
  `:127-162` (fail-open load + corrupt backup + atomic save),
  `:165-283` (8 unit tests: missing file, increment, timestamp,
  round-trip, fail-open, backup, prune, unknown-ID default).
- Readers: `src/sort.rs:124` (`rank_snippets` usage slice),
  `src/ui/mod.rs` (`SnippetListParams.usage`, loaded once per session).
- Writers: `src/commands/run_cmd.rs`, `src/commands/clip_cmd.rs`
  (`record_use()` after success only).
