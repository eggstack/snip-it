# Stable Identity Contract

> **Evergreen contract.** Enforced by `tests/identity_contract.rs`.
> **Note:** Line numbers are approximate and may drift as the codebase evolves.
> Verify against current source when precision is needed.
>
> **Module map:** snippet data types live in `src/library/model.rs`, load/save and
> ID normalization in `src/library/persistence.rs`, and `libraries.toml` registry
> operations in `src/library/manager.rs`. (These were one `src/library.rs` before
> the module-boundary split; there is no `src/library.rs` today.)

## Snippet Identity

- **ID field**: `id` on the `Snippet` struct (`src/library/model.rs:32`)
- **Format**: `legacy-<64 hex chars>` — a SHA-256 digest, hex-encoded
- **Generation**: **deterministic, not random.** `normalize_snippet_ids()`
  (`src/library/persistence.rs:38`) runs on every load and assigns IDs by
  hashing stable user data, so a given file always yields the same IDs.
- **Empty/missing IDs**: `deterministic_legacy_id()` (`src/library/persistence.rs:128`)
  — `SHA-256("snip-it-legacy-id-v1\0" + description + command + tags + output +
  folders + favorite + device_id + occurrence)`, prefixed `legacy-`. Field values
  are NUL-separated; the occurrence index is hashed as its little-endian bytes.
  The digest is rendered as 64 lowercase hex characters (two per digest byte).
- **Duplicate explicit IDs**: `deterministic_duplicate_id()`
  (`src/library/persistence.rs:160`) — same construction but salted with
  `"snip-it-duplicate-id-v1\0" + original_id`
- **Reserved IDs**: every ID the file already carries is reserved before any
  provisional ID is minted, so an ID that is already persisted is never handed
  out again — not to a later snippet, and not to an earlier one. Without this
  a snippet holding `legacy-<hash(content, 0)>` would not consume occurrence
  index 0, and a sibling with the same content and an empty ID would be handed
  the very same ID.
- **Occurrence counter**: the missing-ID and duplicate-ID branches deliberately
  share one content-fingerprint counter, so identical-content snippets consume
  occurrence indices from the same sequence regardless of branch. Inserting or
  reordering identical-content snippets can therefore shift the provisional IDs
  of *other* missing-ID snippets — accepted churn after a sync merge, not a bug.
- **`Snippet::new()`** leaves `id` empty (`src/library/model.rs:231`); the ID is
  assigned at the next load.

> Historical note: the load path was UUID v4 before the deterministic-ID change.
> "New UUID v4 on load" is no longer accurate anywhere in this repository. The
> **import** path still uses `uuid::Uuid::new_v4()` (`src/commands/import_cmd.rs:145`),
> which is the only remaining random-ID site.

### Lifecycle Rules

| Operation | ID Behavior |
|-----------|-------------|
| Edit description/command/tags/output/favorite/folders | Retains ID |
| Usage changes (use_count, last_used) | Retains ID (usage tracked in `usage.toml`, keyed by ID) |
| Move between libraries | Retains ID (globally unique) |
| Native export | Includes ID (the saved library TOML carries the `id` field) |
| Pet format (no `id` field) | ID omitted by the format; the import path assigns a new one |
| Native reimport | Existing ID is discarded; new UUID assigned (`src/commands/import_cmd.rs:135-145`) |
| Import from external source without ID | New UUID assigned |
| Same ID + identical content | Both snippets are kept; the second gets a deterministic replacement ID on load (`src/library/persistence.rs:160`) |
| Same ID + different content | Both snippets are kept; the second gets a deterministic replacement ID on load (`src/library/persistence.rs:160`) |
| Different ID + same content | Both kept (no content-based deduplication) |
| Restore | Retains IDs subject to collision rules (duplicates resolved at load) |
| Sync push/pull/merge | Uses same ID across devices; the protobuf `Snippet` message carries the `id` field (`snip-proto/src/snip_proto.rs:85-113`) |
| Delete | Sets `deleted=true`, retains ID as tombstone (`src/commands/mod.rs:602-617`) |
| Recreate | New ID (never reuses deleted IDs) |

### ID Assignment Points

1. **`load_library()`** — calls `normalize_snippet_ids()` to assign IDs for empty IDs and make duplicate IDs unique (`src/library/persistence.rs:199`, `:38`). This is the primary assignment path for all newly created snippets.
2. **`commands::import_cmd`** — regenerates the ID for imported snippets with `uuid::Uuid::new_v4()`, regardless of source ID (`src/commands/import_cmd.rs:145`). Existing non-empty IDs are discarded and recorded as normalizations. This is the only random-ID site left.
3. **`doctor_cmd`** — reports planned ID regeneration for imported snippets with non-empty IDs (`src/commands/doctor_cmd.rs:292-300`). This is a diagnostic record, not an assignment path.

Note: `Snippet::new()` creates a snippet with an empty `id` field. The deterministic ID is assigned when the library is next loaded via `load_library()`.

## Library Identity

- **Primary key**: `filename` (without `.toml` extension) in `libraries.toml` (`src/library/model.rs`, `LibraryMeta`)
- **Server ID**: Optional `library_id` for sync linkage (`src/library/model.rs`, `LibraryMeta`)
- **Server link**: Optional `server_id` for tracking server association (`src/library/model.rs`, `LibraryMeta`)
- **Primary flag**: `is_primary` boolean — exactly one library is primary (`src/library/model.rs`, `LibraryMeta`)

### Lifecycle Rules

| Operation | Identity Behavior |
|-----------|------------------|
| Library rename | Retains `library_id` if present; `filename` changes (no rename command exists — filename is immutable after creation) |
| Display name | Derived from `filename` |
| Restore collision | New library created; existing entry with same filename rejected (`src/library/manager.rs`) |
| Primary selection | References `filename`; validated canonical name (`src/library/manager.rs`) |
| Delete | Config entry removed, file deleted after config save for crash safety (`src/library/manager.rs`) |
| Recreate | New entry (filename is primary key; case-insensitive duplicate check) (`src/library/manager.rs`) |
| Sync linkage | `link_server_library` sets both `library_id` and `server_id` (`src/library/manager.rs:550`) |
| Unlink | Clears `library_id` and `server_id` (`src/library/manager.rs:586`) |
| Server import | `add_server_library` creates or links by normalized filename (`src/library/manager.rs:678`) |

## Migration Rules

- Missing IDs get a deterministic `legacy-<hex>` ID on load
- Duplicate IDs get a deterministic replacement on load
- Empty IDs get a deterministic `legacy-<hex>` ID on load
- No ID reuse from deleted snippets — deleted snippets retain their original ID as a tombstone (`src/commands/mod.rs:602-617`)
- Deleted snippets are excluded from TUI display but retained for sync (`src/commands/mod.rs:285-292`)

## Sync Identity Semantics

- Snippet IDs are the merge key across devices (`src/sync_commands.rs:1261-1263`)
- Last-write-wins conflict resolution uses `updated_at` timestamp, then
  `device_id`, then a SHA-256 fingerprint of the synced fields
  (`src/sync_commands.rs:1199-1224`)
- Locally deleted snippets are never resurrected by newer server copies
  (`src/sync_commands.rs:1307-1319`); a server-only tombstone is dropped because
  this device never observed the snippet (`src/sync_commands.rs:1325-1332`)
- Server-deleted snippets are marked `deleted=true` locally (data preserved, not removed) (`src/sync_commands.rs:1273-1293`)
- `output` is local-only — not synced, and absent from the protobuf `Snippet` message (`snip-proto/src/snip_proto.rs:85-113`); the merge keeps the local copy (`src/sync_commands.rs:1294-1305`)
- `device_id` and `deleted` fields are sanitized on import (`src/commands/import_cmd.rs:157-158`)

## The server's delta cursor

- **The delta cursor is server-assigned, not client-chosen.** The server
  stores the cursor position of a row in `snippets.visible_at` and filters
  rows with `visible_at >= since`; it hands the client `MAX(visible_at)` as
  the next cursor (`snip-sync/src/db.rs`). `visible_at` is clamped to the
  library's current watermark on every write, so it is monotone by
  construction.
- **`updated_at` stays the client's logical modification time.** It is stored
  verbatim and is what the server's last-write-wins rule and the client's
  merge order compare. The cursor bookkeeping never rewrites it.
- The consequence: a row pushed with an `updated_at` below the library
  maximum (clock skew on one device, or a restore from an old backup) is
  still delivered to every client, because it is delivered at its
  `visible_at` position.
- The remaining wart — the cursor is a timestamp, so the `>=` filter
  re-delivers boundary rows and second granularity still matters — is
  tracked as the proposed protocol-level follow-up in
  `plans/adrs/ADR-0001-sync-delta-cursor.md`.
