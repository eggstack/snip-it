# Usage Metadata Module

[← Back to Overview](overview.md)

## Overview


`src/usage.rs` (283 lines) holds local-only per-snippet usage statistics: how
often and when each snippet ran. It is deliberately isolated from the snippet
library — no command bodies, no output metadata, no credentials — and it is
never synced to a remote server. It feeds the `LastUsed` / `MostUsed` sort
modes and nothing else.

This doc covers the usage index, its on-disk format, and the update policy.
It does not cover sort ranking (see [sort.md](sort.md)), library persistence
(see [persistence.md](persistence.md)), or the sync payload
(see [sync.md](sync.md)).

## Key types — `src/usage.rs:26-54`


```rust
pub struct UsageData { pub use_count: u64, pub last_used_at: Option<i64> }
pub struct UsageIndex { entries: Vec<UsageEntry> }  // private field; borrow-free reads
pub struct UsageEntry { pub id: String, pub use_count: u64, pub last_used_at: Option<i64> }
```

| Type | Visibility | Role |
|------|-----------|------|
| `UsageData` | `pub` | Copy-out return type of `get_usage()`. A plain struct rather than a reference into the index, so callers never hold a borrow on the `UsageIndex` (`src/usage.rs:26-34`) |
| `UsageIndex` | `pub` | The persisted index. Its `entries` field is **private**; access is via `entries()` (`src/usage.rs:41-45`) |
| `UsageEntry` | `pub` | One row: snippet ID + count + timestamp (`src/usage.rs:47-54`) |

## Key functions — `src/usage.rs:61-163`


```rust
pub fn UsageIndex::load() -> Self                          // fail-open; see below
pub fn UsageIndex::save(&self) -> SnipResult<()>           // local-data lock + atomic write
pub fn UsageIndex::record_use(&mut self, snippet_id: &str) // saturating +1, now() stamp
pub fn UsageIndex::get_usage(&self, snippet_id: &str) -> UsageData  // zeroed default if unknown
pub fn UsageIndex::prune(&mut self, active_ids: &[String])          // retain live IDs
pub fn UsageIndex::entries(&self) -> &[UsageEntry]
```

### Storage

- Path: `~/.config/snp/usage.toml` — `usage_path()`
  (`src/usage.rs:57-59`, resolving `get_config_dir()` from
  `src/utils/config.rs`).
- Write: `write_private_atomic(path, &content, "usage")` (`src/usage.rs:161`)
  — atomic rename with 0600 permissions.
- `load_from(path)` / `save_to(path)` take explicit paths so unit tests
  avoid env-var races (`src/usage.rs:127`, `src/usage.rs:158`).

### Concurrency

`UsageIndex::save()` acquires the **local-data lock** before writing, so the
usage file is serialized against backup snapshot capture
(`src/usage.rs:71-76`):

```rust
pub fn save(&self) -> SnipResult<()> {
    let transaction_dir = crate::local_data::transaction_dir();
    let _lock = crate::local_data::acquire_local_data_lock(&transaction_dir)?;
    self.save_to(&usage_path())
}
```

The write is atomic (temp file + rename), so the file is never observed
half-written. However, the *read-modify-write* cycle is **not** atomic across
processes: `load()` → `record_use()` → `save()` reads outside the lock, so
two concurrent `record_use` calls can still lose an increment
(last-writer-wins). This is accepted — usage history is low-value, personal
data with no database behind it.

### Fail-open load

A missing, unreadable, or malformed `usage.toml` never blocks the CLI:

| Condition | Behavior | Source |
|---|---|---|
| File missing / unreadable | Empty index, no warning | `src/usage.rs:128-131` |
| Invalid TOML | `tracing::warn!`, copy to `usage.toml.corrupt.bak`, empty index | `src/usage.rs:132-140` |
| Corrupt backup copy fails | Warn only, still empty index | `src/usage.rs:148-156` |

This is deliberately the opposite of the library load path, which fails
closed on malformed content (never synthesizes a writable empty library).
Usage history is low-value data, so one sibling backup — overwritten on
repeat corruption — is enough.

## On-disk format


The file is a TOML array-of-tables. The table name is the serde field name
of `UsageIndex`, which is `entries`:

```toml
[[entries]]
id = "legacy-3f2a…"
use_count = 5
last_used_at = 1700000000
```

| Key | Type | Notes |
|-----|------|-------|
| `entries` | array of tables | The single top-level key; `#[serde(default)]`, so an empty file deserializes to an empty index |
| `id` | string | The snippet's deterministic ID — `legacy-<sha256 hex>`, **not** a UUID (`docs/IDENTITY_CONTRACT.md`) |
| `use_count` | integer | `u64`, incremented with `saturating_add` (`src/usage.rs:86`) |
| `last_used_at` | integer or absent | Unix epoch **seconds** (`i64`), from `chrono::Utc::now().timestamp()` (`src/usage.rs:84`) |

> The module doc comment at `src/usage.rs:10-17` matches this format: it shows
> `[[entries]]` with `id = "legacy-<sha256-hex>"`. There is no serde rename on
> the `entries` field (`src/usage.rs:41-45`), so `[[entries]]` is the only
> table name that loads — `[[usage]]` does not.

## Update policy


| Action | Count | Last-used | Source |
|--------|:-----:|:---------:|--------|
| Successful `run` | yes | yes | `src/commands/run_cmd.rs:179-184` |
| Failed / cancelled `run` | no | no | usage is written only when `ProcessResult::is_done()` |
| Successful `clip` | yes | yes | `src/commands/clip_cmd.rs:43-47` |
| Cancelled `clip` | no | no | `record_use()` is inside the successful-copy branch |
| `search` / `list` / preview | no | no | no `record_use()` call site |
| `select` | no | no | `select` prints, it does not execute |
| `edit` / `import` / `doctor` | no | no | no `record_use()` call site |

A usage-save failure is **not** fatal — it is logged at debug level and the
command still succeeds (`src/commands/run_cmd.rs:181-183`,
`src/commands/clip_cmd.rs:45-47`).

### Identity and pruning

- The identity key is the **snippet ID** — the deterministic
  `legacy-<sha256 hex>` value from `docs/IDENTITY_CONTRACT.md`. It is stable
  across description edits and reordering, and it is *not* a UUID v4.
- Orphaned entries (IDs with no live snippet) are classified by the shared
  pure helper `library::find_orphaned_ids()` (`src/library/model.rs:169-177`),
  so `validate` (warning diagnostic, `src/commands/validate_cmd.rs:461-466`)
  and `repair` (prune candidate, `src/commands/repair_cmd.rs:434-453`) agree
  on what "orphaned" means.
- `UsageIndex::prune(&active_ids)` retains only live IDs
  (`src/usage.rs:114-118`); `repair` applies it via
  `RepairAction::PruneOrphanedUsage` (`src/commands/repair_cmd.rs:667-670`).

## Readers


| Reader | Source | Purpose |
|--------|--------|---------|
| TUI selection loop | `src/commands/mod.rs:373-402` | Loads the index once per session and builds a one-shot `HashMap` for O(1) per-snippet lookup (not O(k·m)) |
| `ui::SnippetListParams.usage` | `src/ui/mod.rs:499-513` | Optional `&[UsageData]` slice handed to the TUI; the TUI also reads usage for its own ordering hints (`src/ui/mod.rs:181`) |
| `sort::rank_snippets` | `src/sort.rs:116-122` | The `usage` slice used by the `LastUsed` / `MostUsed` sort modes |
| `list` command | `src/commands/list_cmd.rs:124` | `--sort last-used` / `--sort most-used` |
| `validate` command | `src/commands/validate_cmd.rs:461-466` | Orphaned-usage-entry diagnostic |
| `repair` command | `src/commands/repair_cmd.rs:434-453`, `:667-670` | Prune orphaned usage entries |

## Writers


| Writer | Source |
|--------|--------|
| `run` command | `src/commands/run_cmd.rs:179-184` |
| `clip` command | `src/commands/clip_cmd.rs:43-47` |

## Key Files


- `src/usage.rs` — the whole module: types (`:26-54`), `usage_path` (`:57-59`), `UsageIndex` impl (`:61-163`), unit tests (`:165-283`)
- `src/library/model.rs:169-177` — `find_orphaned_ids`, the shared orphan classifier
- `src/commands/run_cmd.rs:175-184` — usage write on successful run
- `src/commands/clip_cmd.rs:40-49` — usage write on successful clip
- `src/commands/mod.rs:373-402` — session-scoped usage load and the one-shot lookup map
- `src/sort.rs:116-122` — the `usage` slice consumed by `LastUsed` / `MostUsed`
- `src/utils/config.rs:63` — `get_config_dir()`, the base for `usage.toml`
## Invariants


- **Local-only, never synced.** Usage metadata is not in `ProtoSnippet`, is
  never uploaded or downloaded, never appears in JSON/CSV snippet exports,
  and is never searchable (the selector contract excludes it).
- **Only ID + count + timestamp are stored** — no command bodies, no output
  metadata, no credentials.
- **Sort reads usage; usage never writes sort state.** The default
  `Relevance` sort ignores usage entirely (compatibility-first).
- **Writes are atomic and lock-serialized**; the read-modify-write cycle is
  not atomic across processes (accepted).
- **Corrupt usage fails open** — never blocks the CLI, unlike the library
  load path, which fails closed.
- **A usage-save failure never fails the command.**

## Gotchas


- **The on-disk table is `[[entries]]`, not `[[usage]]`.** A hand-edited
  `usage.toml` must use `[[entries]]`; there is no serde rename on the field
  (`src/usage.rs:41-45`), so a `[[usage]]` table will not load.
- **Snippet IDs are deterministic `legacy-<sha256 hex>` values, not UUIDs.**
  Do not treat the `id` field as a random v4 UUID.
- **`UsageIndex::entries` is private.** Use `entries()`; the field is not
  part of the API.
- **`UsageIndex::load()` returns `Self`, not `SnipResult<Self>`** — it cannot
  fail.
- **The `usage` module is `#[doc(hidden)] pub`** (`src/lib.rs:50-51`). It is
  reachable from the binary and integration tests, but it is **not** part of
  the supported external API — see [public_api.md](public_api.md).
- **`snp backup --include-usage` opts usage into a local backup archive**
  (`src/commands/backup_cmd.rs:26-28`, `:112-119`). That is a *local* copy,
  not a sync — but it is the one path where `usage.toml` leaves the config
  directory.
- **`last_used_at` is epoch seconds, not milliseconds.** The JSON schema
  rules (`docs/JSON_SCHEMAS.md`) use `*_unix_ms` for command output, but the
  usage file uses plain epoch seconds.

