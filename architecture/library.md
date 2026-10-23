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

**What this document does not cover.** The *atomic write primitive* and the
*durability classes* used by the save path are in
[persistence.md](persistence.md) and [utils/atomic.md](utils/atomic.md). The
*mutation gate*, *transaction journal*, and *lock hierarchy* are in
[persistence.md](persistence.md). The *search-selector contract* that reads
these fields is in [selector.md](selector.md). The *sync protocol* that
consumes the synced subset of the fields is in [sync.md](sync.md). The
*cross-library scope keyword* (`all`) is in [selector.md](selector.md) and
[../docs/LIBRARY_SCOPE.md](../docs/LIBRARY_SCOPE.md).

---

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
  (`model.rs:231`) — rejects empty/whitespace command or description;
  stamps `created_at`/`updated_at` to now, `id` left empty until save/load
  normalization assigns it.
- Serde aliases keep `pet` compatibility: `[[snippets]]`/`[[Snippets]]`,
  `Description`, `Command`, `Tag` (`model.rs:15-22, 31-59`,
  `tests.rs:49-84`).

### `Snippets` — `src/library/model.rs:15-22`

```rust
pub struct Snippets {
    pub snippets: Vec<Snippet>, // serde rename "snippets", alias "Snippets"
    pub folders: Vec<String>,   // skipped when empty
}
```

Serializes to `[[snippets]]` table array; `snp` always writes the
lowercase `snippets` form (`tests.rs:86-103`).

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
```

Stored in `~/.config/snp/libraries.toml`. `generation` lets backup verify
coherent snapshots.

### Read-only resolution types — `src/library/model.rs:105-148`

```rust
pub struct ResolvedLibrarySource {
    pub name: String,       // canonical library name (filename without `.toml`)
    pub library_id: String, // server-side library ID when linked, otherwise empty
    pub path: PathBuf,      // canonical path to the library TOML file
}

pub enum PrimaryState {
    Present { name: String },
    FileMissing { name: String, path: PathBuf },
    NoPrimary { count: usize },
    NoLibraries,            // default
}

pub struct LibraryIndexInspection {
    pub missing_files: Vec<(String, PathBuf)>,
    pub orphan_files: Vec<PathBuf>,
    pub primary: PrimaryState,
}
```

`ResolvedLibrarySource` is constructed **without** migration, directory
creation, or metadata rewrites (`model.rs:105-111`). `PrimaryState` and
`LibraryIndexInspection` are a shared read-only view: `doctor`, `validate`,
`repair`, and `status` each render it into their own diagnostic types
(`model.rs:135-139`).

### Pure helpers — `src/library/model.rs:150-225`

```rust
pub fn library_not_found(name: &str) -> SnipError                                   // model.rs:154
pub fn find_orphaned_ids(active: &HashSet<String>, usage: &[String]) -> Vec<String>   // model.rs:169
pub(crate) fn validate_library_name(name: &str) -> Result<(), (&'static str, &'static str)> // model.rs:179
```

`library_not_found` is the canonical missing-library error, used by every
read-only resolution path so CLI, MCP, and diagnostics cannot drift
(`model.rs:150-153`).

`find_orphaned_ids` is shared by `validate` (warning diagnostics) and
`repair` (prune candidates); empty usage IDs are ignored
(`model.rs:163-177`).

`validate_library_name` rejects, in order: empty names; names longer than
50 characters; names containing `/`, `\`, or NUL; `.` and `..`; names ending
in `.toml`; and the reserved name `all` (`model.rs:179-225`).

> **`all` is reserved.** `all` is the cross-library scope keyword owned by
> `LibraryScope::from_filter_arg` (`src/selector.rs`). A library actually
> named `all` could never be addressed exclusively by the resolver surfaces,
> and `--library all` silently returned the wrong scope on them instead of an
> error. `snp library create all` is therefore rejected
> (`model.rs:213-223`).

---

## TOML persistence

### `load_library(path) / save_library(path, snippets)` — `persistence.rs:199-338`

```rust
pub fn load_library(path: &Path) -> SnipResult<Snippets>
pub fn save_library(path: &Path, snippets: &Snippets) -> SnipResult<()>
pub fn save_library_internal(path: &Path, snippets: &Snippets, _guard: &LocalDataLock) -> SnipResult<()>
pub fn backup_library(path: &Path) -> SnipResult<Option<PathBuf>>
```

- **Load fails closed on malformed TOML**: best-effort copy to
  `<file>.toml.corrupt.bak`, then `SnipError::toml_error` — never
  synthesize a writable empty library (`persistence.rs:211-237`).
  Missing file or empty/whitespace file returns `Snippets::default()`
  (`persistence.rs:200-207`). A missing file and an empty file are
  different cases and must stay different.
- **Save path**: gates on interrupted transactions, acquires the
  local-data lock, calls `backup_library()`, sorts by `updated_at`
  descending through a borrowed `SortedSnippetsView` (no payload clone),
  writes verbatim `toml::to_string_pretty` output via
  `write_private_atomic`, then invalidates the TOML cache
  (`persistence.rs:258-338`).

### The save path never post-processes TOML

`save_library` writes the output of `toml::to_string_pretty` **verbatim**
(`persistence.rs:249-257`). There is exactly one serialization-then-write
site, and nothing may run between them.

Any post-processing pass (re-indenting, key sorting, trimming, "fixing"
quotes) silently corrupts any user file containing tabs, trailing
whitespace, or CRLF — the user gets no error, they get mangled snippets
later.

The specific historical trap: `quote_strings_containing_backslashes` once
ran after serialization. That older implementation could not distinguish a
TOML triple-quoted multi-line string from an ordinary double-quoted string,
and its single-quoted output preserved escape sequences like `\t` as a
literal two-character pair. The helper survives only for callers that
hand-write TOML — **it must never be applied to serialized output**
(`src/utils/toml_helpers.rs:12-20, 181-193`).

### Golden-corpus rules

`golden_corpus()` (`tests/support/mod.rs:157`) holds 24 edge-case commands:
tabs (internal, in a Makefile recipe, adjacent to quotes, before trailing
whitespace), trailing spaces (single and multiple), CRLF and mixed
newlines, no/one/many trailing newlines, backslashes (Windows paths,
tab+backslash), escaped angle brackets, nested quotes, shell operators and
substitution, unicode, leading spaces, a multiline script, blank lines, and
variables.

`tests/local_contracts.rs:217` (`test_golden_corpus_preserves_exact_text`)
drives every case through the real binary and asserts the command survives
create → save → load byte-for-byte. It also asserts the **exact snippet
count** matches the corpus size, so a silently dropped snippet fails the
test. When you change anything in the save/load path, add a corpus case that
would have caught your bug.

### Gate and lock derivation

`save_library` derives its gate and local-data-lock paths from the **config
directory**, not from the target library path:

```rust
let sync_state_dir = crate::config::derive_sync_state_dir();  // persistence.rs:264
let transaction_dir = crate::local_data::transaction_dir();   // persistence.rs:265
crate::transaction::gate_mutation_on_interrupted_transactions(
    &sync_state_dir,
    &transaction_dir,
)?;                                                            // persistence.rs:266-269
let _local_lock = crate::local_data::acquire_local_data_lock(&transaction_dir)?; // persistence.rs:274
```

Deriving these from the target library path meant a unit test that saved to
a temp file still created `.transaction/` and a lock file *inside the
temp directory*, while a test that saved to a real library path wrote to
the developer's real `~/.config/snp/` — mutating `audit.log`, `usage.toml`,
and `.transaction/`, and contending on one process-wide lock file.

`ScopedConfigDir` (`src/utils/config.rs:35-57`) is the test-side fix: a
`#[cfg(test)]`-only, thread-local override of `get_config_dir()` that
restores the previous value on drop, including on assertion panic. Because
it is thread-local, parallel unit tests do not race on a process-global
`XDG_CONFIG_HOME` mutation.

`save_library_internal` (`persistence.rs:292-304`) skips the gate and the
lock. It is valid only for callers that already hold the local-data lock and
have already passed the gate — restore uses it so the global gate does not
see restore's *own* journal as interrupted and roll it back mid-flight.

### Schema-version write path

The persisted library format carries **no schema version**. The dead
versioning framework (`src/migration.rs`) was removed; introducing a schema
version is a schema change that requires a migration and a version bump.

---

## Deterministic ID normalization

`normalize_snippet_ids` (`persistence.rs:38-102`) runs on every load and has
exactly two branches:

| Branch | Condition | Action |
|--------|-----------|--------|
| Missing ID | `snippet.id.is_empty()` | `deterministic_legacy_id(snippet, occurrence)` |
| Duplicate ID | `seen_ids.contains(&snippet.id)` | `deterministic_duplicate_id(&snippet.id, snippet, occurrence)` |

The first occurrence of a duplicated explicit ID keeps the original ID; only
later duplicates are reassigned.

Both branches **share one content-fingerprint occurrence counter**, so
identical-content snippets consume occurrence indices from the same
sequence regardless of which branch processes them. This keeps IDs
deterministic for a fixed file order; inserting or reordering
identical-content snippets can shift provisional IDs of unrelated
missing-ID snippets (accepted churn after sync merges)
(`persistence.rs:32-37`).

### The digest preimage

`deterministic_legacy_id` (`persistence.rs:128-157`) hashes:

```
"snip-it-legacy-id-v1\0" + description + "\0" + command + "\0"
+ (tag + "\0")* + output + "\0" + (folder + "\0")*
+ favorite-as-one-byte + "\0" + device_id + "\0" + occurrence.to_le_bytes()
```

`deterministic_duplicate_id` (`persistence.rs:160-190`) hashes the same
fields prefixed with `"snip-it-duplicate-id-v1\0"` and the original ID.

Both render as `legacy-<64 lowercase hex chars>`. The full lifecycle rules
are in [../docs/IDENTITY_CONTRACT.md](../docs/IDENTITY_CONTRACT.md).

> **Note the inclusion of `folders`, `favorite`, and `device_id`.** These
> are local-only or sync-metadata fields, but they participate in the
> digest, so the same snippet content in two libraries with different
> folder assignments gets different deterministic IDs.

### The content fingerprint

`snippet_content_key` (`persistence.rs:105-125`) builds the occurrence
counter key from the same field set — description, command, tags, output,
folders, `favorite` as `"1"`/`"0"`, and `device_id` — joined by NUL.

---

## `LibraryManager`

`src/library/manager.rs`

```rust
pub fn new() -> SnipResult<Self>                                  // manager.rs:43
pub fn ensure_library_mode(&mut self) -> SnipResult<()>           // manager.rs:165
pub fn migrate_from_single_file(&mut self) -> SnipResult<()>      // manager.rs:183
pub fn create_library(&mut self, filename: &str) -> SnipResult<PathBuf> // manager.rs:376
pub fn delete_library(&mut self, filename: &str) -> SnipResult<()>      // manager.rs:442
pub fn set_primary(&mut self, filename: &str) -> SnipResult<()>         // manager.rs:512
pub fn update_library_id(&mut self, filename: &str, library_id: &str) -> SnipResult<()>
pub fn link_server_library(&mut self, filename: &str, server_id: &str) -> SnipResult<()>
pub fn relink_server_library(&mut self, filename: &str, server_id: &str, last_sync: Option<i64>) -> SnipResult<()>
pub fn unlink_server_library(&mut self, filename: &str) -> SnipResult<()>
pub fn add_existing_library(&mut self, filename: &str) -> SnipResult<()>
pub fn update_last_sync(&mut self, filename: &str, timestamp: i64) -> SnipResult<()>
pub fn add_server_library(&mut self, server_name: &str, server_id: &str) -> SnipResult<PathBuf> // manager.rs:678
```

### `ensure_library_mode()` — the MUTATING path

```rust
pub fn ensure_library_mode(&mut self) -> SnipResult<()> {
    if self.is_single_file_mode() {
        self.migrate_from_single_file()?;
    }
    Ok(())
}
```

`is_single_file_mode()` is `!self.libraries_dir.exists()`
(`manager.rs:155-157`). `ensure_library_mode` therefore **mutates**:
`migrate_from_single_file` (`manager.rs:183-222`) creates `libraries/`,
copies the legacy `~/.config/snp/snippets.toml` to
`libraries/snippets.toml`, registers it as primary, bumps `generation`, and
saves `libraries.toml`. The original legacy file is left behind as a second,
now-inert copy.

**Ordering matters.** The existence of `libraries/` is what flips the install
into library mode, and in library mode the legacy file is invisible to every
read path. So the migration reads the legacy content **first** and returns
without creating the directory when the file is empty (an empty legacy file
carries no snippets — the install stays in single-file mode and the file
remains the source of truth). When the content is non-empty, the directory is
created, the library file is written, the library is registered, and the
config is saved; if any of that fails, the directory is removed again so the
legacy file is not orphaned. The registration is skipped when a `snippets`
entry already exists — a second entry would make `libraries.toml` ambiguous
and duplicate every snippet in `--library all`.

Any command that resolves a library path through `ensure_library_mode` —
directly or transitively — can therefore migrate a checkout as a side
effect of a read.

### The canonical read-only resolver

```rust
pub fn resolve_readonly_sources(
    &self,
    library: Option<&str>,
) -> SnipResult<Vec<ResolvedLibrarySource>>            // manager.rs:261-302

pub fn readonly_library_sources(
    library: Option<&str>,
) -> SnipResult<Vec<ResolvedLibrarySource>>             // manager.rs:815-818
```

`readonly_library_sources` is the process-level entry point: it constructs a
`LibraryManager` and delegates to `resolve_readonly_sources`. It performs no
migration and creates no files or directories (`manager.rs:810-818`).

`resolve_readonly_sources` never calls `ensure_library_mode`, never creates
directories or files, and never rewrites metadata
(`manager.rs:250-256`). Scope semantics:

| `library` argument | Result |
|--------------------|--------|
| `None` | the primary library — or, in legacy single-file mode, the implicit `snippets` library |
| `Some("all")` | every visible library |
| `Some("snippets")` (legacy mode) | the legacy single-file library |
| `Some(name)` | the named library, or `library_not_found(name)` |

In legacy single-file mode the resolver represents the legacy file as an
implicit `snippets` library so normal CLI resolution keeps working
(`manager.rs:265-282`).

Consumers: `commands::load_readonly_snippets` (`src/commands/mod.rs:169-189`,
used by `snp list`), `selector::resolve_selector_readonly()` (`snp get`), MCP
`tools.rs` list/search/get.

> **`snp list` was migrated by a read.** Before the fix, `snp list` resolved
> through `get_library_path`, which calls the *mutating*
> `ensure_library_mode()` (`src/commands/mod.rs:91-157`). On a legacy
> single-file checkout a plain `snp list` created `libraries/`, copied the
> legacy file (leaving the original behind as a second, now-inert copy),
> wrote `libraries.toml`, and bumped the generation. `list` now resolves via
> `crate::commands::load_readonly_snippets(library.as_deref())`
> (`src/commands/list_cmd.rs:65-77`), which routes `"all"` through
> `LibraryScope::from_filter_arg` so `--library all` means the same thing
> here as on `snp get`, exact-mode `run`/`clip`/`edit`, and MCP
> (`src/commands/mod.rs:159-168`).
>
> `snp search` rejects `--library all` outright, because the interactive
> selector edits and deletes within a single library file
> (`src/commands/search_cmd.rs:32-44`).

### Index inspection

```rust
pub fn inspect_library_index(&self) -> LibraryIndexInspection  // manager.rs:310
```

Existence checks + directory listing only. `doctor`, `validate`, `repair`,
`status` render this shared state into their own diagnostic types; no
generic finding DSL was introduced (`manager.rs:304-310`).

### CRUD and linkage

- **Migration single → multi** (`manager.rs:183-222`): copies legacy
  `~/.config/snp/snippets.toml` to `libraries/snippets.toml`, registers
  it as primary, bumps `generation`, saves config. Empty/missing legacy
  file is a no-op.
- **Create**: validates name, rejects case-insensitive duplicates and
  existing files, writes `snippets = []` starter via `write_library_file`,
  first library becomes primary (`manager.rs:376-436`).
- **Delete**: saves config *before* removing the file (crash leaves a
  recoverable orphan file, never a stale index pointer); promotes another
  library when primary is removed, preferring a server-linked one
  (`manager.rs:442-509`). Restores the pre-delete config if file removal
  fails.
- Every mutating method calls `gate_mutation()` + acquires the
  local-data lock first; `save_config()` writes `libraries.toml` via
  `write_private_atomic` + cache invalidation (`manager.rs:797-807`).
- `gate_mutation()` runs *before* any in-memory mutation, so a refused call
  leaves the manager clean for a retry after `snp repair`
  (`manager.rs:666-672`).

### Config loading — fail closed

`LibraryManager::new()` (`manager.rs:43-94`) parses `libraries.toml`. A
**malformed** file is copied to `libraries.toml.corrupt` on a best-effort
basis and then returns `SnipError::toml_error` — it never synthesizes a
writable empty config (`manager.rs:58-83`). A **missing** file yields
`LibraryConfig::default()` (`manager.rs:84-86`).

`LibraryManager::with_config_dir` (`manager.rs:100-142`) is the `#[cfg(test)]`
counterpart of `new()` that roots the manager at an explicit directory
without mutating process-wide environment variables.

---

## Local-only vs synced field split

| Field | Synced? | Notes |
|-------|---------|-------|
| `id` | Yes | Primary identity; the same ID across devices |
| `description` | Yes | Searchable |
| `command` | Yes | Searchable |
| `tags` | Yes | Searchable by default |
| `created_at` | Yes | Unix timestamp |
| `updated_at` | Yes | Unix timestamp; conflict-resolution input |
| `device_id` | Yes | Conflict-resolution input; not searchable |
| `deleted` | Yes | Soft-delete tombstone; deletion beats live content |
| `output` | **No** | Local-only notes. Not in `ProtoSnippet`; not searchable |
| `folders` | **No** | Local-only organization. Not searchable |
| `favorite` | **No** | Local-only starred flag. Not searchable |

`output` / `folders` / `favorite` are excluded from the sync fingerprint.
`output` is not part of `ProtoSnippet` at all, and `snp edit --output`
requires `--filter` (see [output.md](output.md)).

The searchable subset is governed by `selector::searchable_text` — see
[selector.md](selector.md) and
[../.skills/selector-and-search-parity.md](../.skills/selector-and-search-parity.md).
Adding a field to the searchable set changes three surfaces with no
compiler error.

---

## Agent skill

See [`.skills/persistence-and-toml-fidelity.md`](../.skills/persistence-and-toml-fidelity.md)
for the golden command corpus, the "never post-process serialized TOML" rule,
durability classes, and fail-closed parsing, and
[`.skills/selector-and-search-parity.md`](../.skills/selector-and-search-parity.md)
for the searchable-field contract. Identity rules are in
[`docs/IDENTITY_CONTRACT.md`](../docs/IDENTITY_CONTRACT.md) and the
cross-library scope keyword in
[`docs/LIBRARY_SCOPE.md`](../docs/LIBRARY_SCOPE.md).

---

## Key Files

| File | Role |
|------|------|
| `src/library/mod.rs` | Module root; re-exports `model`, `persistence`, `manager` so the pre-split `crate::library::*` paths keep working (`mod.rs:31-40`) |
| `src/library/model.rs` | `Snippet`, `Snippets`, `LibraryConfig`, `LibraryMeta`, `ResolvedLibrarySource`, `PrimaryState`, `LibraryIndexInspection`, and the pure helpers `library_not_found` / `find_orphaned_ids` / `validate_library_name` |
| `src/library/persistence.rs` | `load_library`, `save_library`, `save_library_internal`, `backup_library`, `normalize_snippet_ids`, `deterministic_legacy_id`, `deterministic_duplicate_id` |
| `src/library/manager.rs` | `LibraryManager` registry/primary behavior, `ensure_library_mode`, and the canonical read-only resolver `resolve_readonly_sources` / `readonly_library_sources` |
| `src/library/tests.rs` | 97 in-crate unit tests: round-trip, `pet` compatibility, ID normalization, name validation |
| `src/utils/atomic.rs` | `write_private_atomic` / `atomic_replace` — the write primitives the save path uses |
| `src/utils/config.rs` | `get_config_dir`, `derive_sync_state_dir`, and the `#[cfg(test)]` `ScopedConfigDir` override |
| `src/utils/toml_helpers.rs` | Escape helpers — for hand-written TOML only, never for serialized output |
| `src/transaction.rs` | The mutation gate and the transaction lock the save path depends on |
| `src/local_data.rs` | `LocalDataLock` and `transaction_dir()` — the lock the save path takes |
| `src/commands/mod.rs` | `get_library_path` (mutating) and `load_readonly_snippets` (read-only) |
| `src/commands/list_cmd.rs` | `snp list` — resolves through the read-only path |
| `src/commands/search_cmd.rs` | `snp search` — rejects `--library all` |
| `src/selector.rs` | `LibraryScope::from_filter_arg`, `searchable_text`, `resolve_selector_readonly` |
| `tests/local_contracts.rs` | Golden-corpus byte fidelity and count preservation |
| `tests/support/mod.rs` | `golden_corpus()` — the 24 edge-case commands |

---

## Invariants

- `gate_mutation_on_interrupted_transactions()` before every local
  mutation (save, create/delete/set-primary, relink). One journal =
  auto-rollback; multiple/incomplete = refuse, direct to `snp repair`.
- Lock order is `LocalDataLock` → `TransactionLock` → writes; the gate runs
  before either lock. See [persistence.md](persistence.md).
- Save path does NOT post-process `toml::to_string_pretty` — tabs,
  trailing spaces, CRLF in the golden corpus must survive verbatim.
- Malformed library/`libraries.toml` fails closed (backup + error);
  missing/empty files give defaults (`manager.rs:55-86`,
  `persistence.rs:199-207`).
- `output`/`folders`/`favorite` are local-only: excluded from the sync
  fingerprint, `output` not in `ProtoSnippet` (see [output.md](output.md)).
- Library names: non-empty, ≤50 chars, no `/`, `\`, NUL, `.`/`..`, no
  `.toml` suffix, and not the reserved name `all`
  (`model.rs:179-225`).
- Read-only surfaces resolve through `readonly_library_sources` /
  `resolve_selector_readonly` — never through `ensure_library_mode()` or
  `resolve_selector`.
- Missing-library recovery uses atomic `<library>.sync_recovery` state
  (see [sync.md](sync.md)): one normalized remote-name match only, fail on
  ambiguity, remove marker only after durable relink + retry.

---

## Gotchas

- **`ensure_library_mode()` mutates.** It is not a "check" — it creates
  `libraries/`, copies the legacy file, writes `libraries.toml`, and bumps
  the generation. A read-only path that routes through it turns a plain
  `snp list` into a migration. Use `readonly_library_sources` instead.
- **`resolve_selector` mutates; `resolve_selector_readonly` does not.** The
  former runs `ensure_library_mode()` and can migrate/rewrite metadata.
  Picking the wrong one reintroduces recovery side effects — see
  [selector.md](selector.md).
- **`all` is a reserved library name.** `snp library create all` is
  rejected because `--library all` is the cross-library scope keyword.
- **Never post-process serialized TOML.** See the golden-corpus rules
  above.
- **The save path derives its gate and lock paths from the config
  directory**, not the library path. Keep it that way, or unit tests write
  to the developer's real `~/.config/snp/`.
- **`save_library_internal` skips the gate and lock.** Only call it when
  you already hold both.
- **Deterministic IDs are not UUIDs.** They are `legacy-<sha256 hex>` over
  content *plus* `folders`/`favorite`/`device_id` *plus* an occurrence
  index. Copying a snippet between libraries can change its ID.
- **The occurrence counter is shared between the missing-ID and duplicate-ID
  branches.** Changing one branch without the other shifts IDs.
- **A missing file and an empty file are different cases.** Missing →
  defaults; empty → defaults; malformed → fail closed. Never collapse
  malformed into empty.
