# Library Module (`src/library/`)

[← Back to Overview](overview.md)

## Overview

The library module is the core data layer of snip-it. It defines the
`Snippet`/`Snippets` data structures, TOML persistence with deterministic
ID repair, timestamped backups, single-file → multi-library migration,
and the `LibraryManager` registry. It also owns the canonical
side-effect-free read path (`resolve_readonly_sources`,
`inspect_library_index`) shared by CLI, MCP, and diagnostics.

**Files**: `src/library/mod.rs` (re-exports), `src/library/model.rs`
(types + pure helpers), `src/library/persistence.rs` (load/save/backup),
`src/library/manager.rs` (`LibraryManager`), `src/library/tests.rs`
(unit tests).

## Key types

### `Snippet` — `src/library/model.rs:30-60`

```rust
pub struct Snippet {
    pub id: String,           // Opaque; UUID for new, legacy-<sha256> for repaired
    pub description: String,  // Human-readable name (aliases: Description, name)
    pub output: String,       // Local-only notes (aliases: Output; NOT synced)
    pub tags: Vec<String>,    // Serialized as `tag` (aliases: Tag, Tags, tags)
    pub command: String,      // Shell body; may contain <variables> (aliases: Command, cmd)
    pub folders: Vec<String>, // Local-only organization
    pub favorite: bool,       // Local-only starred flag
    pub created_at: i64,      // Unix timestamp
    pub updated_at: i64,      // Unix timestamp; save order key
    pub device_id: String,    // Originating device (sync conflict input)
    pub deleted: bool,        // Soft-delete tombstone flag
}
```

- `Snippet::new(description, command, tags) -> SnipResult<Self>`
  (`model.rs:220`) — rejects empty/whitespace command or description;
  stamps `created_at`/`updated_at` to now, `id` left empty until save/load
  normalization assigns it.
- Serde aliases keep `pet` compatibility: `[[snippets]]`/`[[Snippets]]`,
  `Description`, `Command`, `Tag` (`model.rs:15-22`, `tests.rs:20-54`).

### `Snippets` — `src/library/model.rs:14-22`

```rust
pub struct Snippets {
    pub snippets: Vec<Snippet>, // serde rename "snippets", alias "Snippets"
    pub folders: Vec<String>,   // skipped when empty
}
```

Serializes to `[[snippets]]` table array; `snp` always writes the
lowercase `snippets` form (`tests.rs:57-73`).

### `LibraryConfig` / `LibraryMeta` — `src/library/model.rs:65-103`

```rust
pub struct LibraryConfig {
    pub libraries: Vec<LibraryMeta>,
    pub generation: u64, // monotonic; bumped on every mutation
}
pub struct LibraryMeta {
    pub filename: String,           // without `.toml`
    pub library_id: String,         // server linkage (empty when unlinked)
    pub is_primary: bool,
    pub last_sync: Option<i64>,     // sync cursor; reset on relink
    pub server_id: Option<String>,  // server-side ID when linked
}
pub fn library_not_found(name: &str) -> SnipError // model.rs:155
pub fn find_orphaned_ids(active: &HashSet<String>, usage: &[String]) -> Vec<String> // model.rs:169
pub(crate) fn validate_library_name(name: &str) -> Result<(), (&'static str, &'static str)> // model.rs:179
```

Stored in `~/.config/snp/libraries.toml`. `generation` lets backup verify
coherent snapshots.

## Key functions

### `load_library(path) / save_library(path, snippets)` — `persistence.rs:158-297`

```rust
pub fn load_library(path: &Path) -> SnipResult<Snippets>
pub fn save_library(path: &Path, snippets: &Snippets) -> SnipResult<()>
pub fn save_library_internal(path: &Path, snippets: &Snippets, _guard: &LocalDataLock) -> SnipResult<()>
pub fn backup_library(path: &Path) -> SnipResult<Option<PathBuf>>
```

- **Load fails closed on malformed TOML**: best-effort copy to
  `<file>.toml.corrupt.bak`, then `SnipError::toml_error` — never
  synthesize a writable empty library (`persistence.rs:170-196`).
  Missing file or empty/whitespace file returns `Snippets::default()`.
- **ID normalization** (`normalize_snippet_ids`, `persistence.rs:36-61`):
  empty IDs get `legacy-<sha256hex>` via `deterministic_legacy_id()`;
  duplicate explicit IDs keep the first occurrence and reassign later ones
  via `deterministic_duplicate_id()`. Both branches share one
  content-fingerprint occurrence counter, so repeated loads of identical
  content yield identical IDs, but inserting/reordering identical-content
  snippets can shift provisional IDs (accepted churn after sync merges).
- **Save path**: gates on interrupted transactions, acquires the
  local-data lock, calls `backup_library()`, sorts by `updated_at`
  descending through a borrowed `SortedSnippetsView` (no payload clone),
  writes verbatim `toml::to_string_pretty` output via
  `write_private_atomic`, then invalidates the TOML cache
  (`persistence.rs:217-297`).

### `LibraryManager` CRUD — `src/library/manager.rs`

```rust
pub fn new() -> SnipResult<Self>                                  // manager.rs:41
pub fn ensure_library_mode(&mut self) -> SnipResult<()>           // manager.rs:163
pub fn migrate_from_single_file(&mut self) -> SnipResult<()>      // manager.rs:181
pub fn create_library(&mut self, filename: &str) -> SnipResult<PathBuf> // manager.rs:363
pub fn delete_library(&mut self, filename: &str) -> SnipResult<()>      // manager.rs:418
pub fn set_primary(&mut self, filename: &str) -> SnipResult<()>         // manager.rs:488
pub fn update_library_id(&mut self, filename: &str, library_id: &str) -> SnipResult<()>
pub fn link_server_library(&mut self, filename: &str, server_id: &str) -> SnipResult<()>
pub fn relink_server_library(&mut self, filename: &str, server_id: &str, last_sync: Option<i64>) -> SnipResult<()>
pub fn unlink_server_library(&mut self, filename: &str) -> SnipResult<()>
pub fn add_existing_library(&mut self, filename: &str) -> SnipResult<()>
pub fn update_last_sync(&mut self, filename: &str, timestamp: i64) -> SnipResult<()>
pub fn add_server_library(&mut self, server_name: &str, server_id: &str) -> SnipResult<PathBuf> // manager.rs:637
```

- **Migration single → multi** (`manager.rs:181-209`): copies legacy
  `~/.config/snp/snippets.toml` to `libraries/snippets.toml`, registers
  it as primary, bumps `generation`, saves config. Empty/missing legacy
  file is a no-op.
- **Create**: validates name, rejects case-insensitive duplicates and
  existing files, writes `snippets = []` starter via `write_library_file`,
  first library becomes primary (`manager.rs:363-412`).
- **Delete**: saves config *before* removing the file (crash leaves a
  recoverable orphan file, never a stale index pointer); promotes another
  library when primary is removed, preferring a server-linked one
  (`manager.rs:418-485`). Restores the pre-delete config if file removal
  fails.
- Every mutating method calls `gate_mutation()` + acquires the
  local-data lock first; `save_config()` writes `libraries.toml` via
  `write_private_atomic` + cache invalidation (`manager.rs:755-765`).

### Read-only resolution — `manager.rs:248-289`

```rust
pub struct ResolvedLibrarySource { pub name: String, pub library_id: String, pub path: PathBuf }
pub fn resolve_readonly_sources(&self, library: Option<&str>) -> SnipResult<Vec<ResolvedLibrarySource>>
pub fn readonly_library_sources(library: Option<&str>) -> SnipResult<Vec<ResolvedLibrarySource>> // manager.rs:773
```

Single canonical implementation for side-effect-free discovery: never
calls `ensure_library_mode`, never creates dirs/files or rewrites
metadata. `None` → primary (or implicit legacy `snippets` file),
`Some("all")` → every visible library, `Some(name)` → named or
`library_not_found()`. Consumers: `selector::resolve_selector_readonly()`
(`snp get`), MCP `tools.rs` list/search/get.

### Index inspection — `manager.rs:297-357`

```rust
pub fn inspect_library_index(&self) -> LibraryIndexInspection
pub enum PrimaryState { Present{name}, FileMissing{name, path}, NoPrimary{count}, NoLibraries }
pub struct LibraryIndexInspection { pub missing_files: Vec<(String, PathBuf)>, pub orphan_files: Vec<PathBuf>, pub primary: PrimaryState }
```

Existence checks + directory listing only. `doctor`, `validate`,
`repair`, `status` render this shared state into their own diagnostic
types; no generic finding DSL was introduced.

## Invariants / gotchas (from AGENTS.md)

- `gate_mutation_on_interrupted_transactions()` before every local
  mutation (save, create/delete/set-primary, relink). One journal =
  auto-rollback; multiple/incomplete = refuse, direct to `snp repair`.
- Save path does NOT post-process `toml::to_string_pretty` — tabs,
  trailing spaces, CRLF in the golden corpus must survive verbatim.
- Malformed library/`libraries.toml` fails closed (backup + error);
  missing/empty files give defaults (`manager.rs:53-84`,
  `persistence.rs:158-166`).
- `output`/`folders`/`favorite` are local-only: excluded from the sync
  fingerprint, `output` not in `ProtoSnippet` (see `output.md`).
- Library names: non-empty, ≤50 chars, no `/`, `\`, NUL, `.`/`..`, no
  `.toml` suffix (`model.rs:179-214`).
- Missing-library recovery uses atomic `<library>.sync_recovery` state
  (see `sync.md`): one normalized remote-name match only, fail on
  ambiguity, remove marker only after durable relink + retry.

## File / line refs

- Types: `src/library/model.rs:14-103` (`Snippet`, `Snippets`,
  `LibraryConfig`, `LibraryMeta`, `ResolvedLibrarySource`,
  `PrimaryState`, `LibraryIndexInspection`).
- Persistence: `src/library/persistence.rs:36-150` (ID normalization),
  `:158-202` (load), `:217-297` (save), `:303-337` (backup, 10-keep).
- Manager: `src/library/manager.rs:41-92` (`new`), `:163-209`
  (mode/migration), `:248-289` (readonly resolver), `:297-357`
  (inspection), `:363-680` (CRUD + server linkage).
- Re-exports: `src/library/mod.rs:31-40`.
