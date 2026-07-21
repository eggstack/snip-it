# Stable Identity Contract

## Snippet Identity

- **ID field**: UUID v4 string, stored in `id` field of `Snippet` struct (`src/library.rs:48`)
- **Generation**: `uuid::Uuid::new_v4()` — never regenerated for a given snippet
- **Deduplication on load**: Duplicate IDs within a library get a new UUID assigned at load time (`load_library` in `src/library.rs:696-707`)
- **Empty IDs**: Assigned a new UUID v4 at load time (`src/library.rs:699-700`)

### Lifecycle Rules

| Operation | ID Behavior |
|-----------|-------------|
| Edit description/command/tags/output/favorite/folders | Retains ID |
| Usage changes (use_count, last_used) | Retains ID (usage tracked in `usage.toml`, keyed by ID) |
| Move between libraries | Retains ID (globally unique) |
| Native export | Includes ID |
| Pet export (no ID field) | ID omitted (format has no field) |
| Native reimport | Existing ID is discarded; new UUID assigned (`src/commands/import_cmd.rs:135-145`) |
| Import from external source without ID | New UUID assigned |
| Same ID + identical content | Deduplicates on load (one copy kept) |
| Same ID + different content | Conflict: duplicate ID gets new UUID on load (`src/library.rs:702-703`) |
| Different ID + same content | Both kept (no content-based deduplication) |
| Restore | Retains IDs subject to collision rules (duplicates resolved at load) |
| Sync push/pull/merge | Uses same ID across devices; `ProtoSnippet` carries `id` field (`src/sync_commands.rs:160-173`) |
| Delete | Sets `deleted=true`, retains ID as tombstone (`src/commands/mod.rs:418-433`) |
| Recreate | New UUID (never reuses deleted IDs) |

### ID Assignment Points

1. **`load_library()`** — assigns UUID to empty IDs and deduplicates duplicate IDs on load (`src/library.rs:696-707`). This is the primary assignment path for all newly created snippets.
2. **`commands::import_cmd`** — explicitly assigns UUID for imported snippets, regardless of source ID (`src/commands/import_cmd.rs:145`). Existing non-empty IDs are discarded and regenerated.
3. **`doctor_cmd`** — reports planned ID regeneration for imported snippets with non-empty IDs (`src/commands/doctor_cmd.rs:264-268`). This is a diagnostic record, not an assignment path.

Note: `Snippet::new()` creates a snippet with an empty `id` field. The UUID is assigned when the library is next loaded via `load_library()`.

## Library Identity

- **Primary key**: `filename` (without `.toml` extension) in `libraries.toml` (`src/library.rs:93`)
- **Server ID**: Optional `library_id` for sync linkage (`src/library.rs:95`)
- **Server link**: Optional `server_id` for tracking server association (`src/library.rs:101`)
- **Primary flag**: `is_primary` boolean — exactly one library is primary (`src/library.rs:97`)

### Lifecycle Rules

| Operation | Identity Behavior |
|-----------|------------------|
| Library rename | Retains `library_id` if present; `filename` changes (no rename command exists — filename is immutable after creation) |
| Display name | Derived from `filename` |
| Restore collision | New library created; existing entry with same filename rejected (`src/library.rs:356-361`) |
| Primary selection | References `filename`; validated canonical name (`src/library.rs:449-466`) |
| Delete | Config entry removed, file deleted after config save for crash safety (`src/library.rs:397-446`) |
| Recreate | New entry (filename is primary key; case-insensitive duplicate check) (`src/library.rs:363-372`) |
| Sync linkage | `link_server_library` sets both `library_id` and `server_id` (`src/library.rs:480-488`) |
| Unlink | Clears `library_id` and `server_id` (`src/library.rs:491-498`) |
| Server import | `add_server_library` creates or links by normalized filename (`src/library.rs:538-577`) |

## Migration Rules

- Missing IDs get UUID v4 assigned on load (`src/library.rs:699-700`)
- Duplicate IDs get new UUID assigned on load (`src/library.rs:702-703`)
- Empty IDs get UUID v4 assigned on load (`src/library.rs:699-700`)
- No ID reuse from deleted snippets — deleted snippets retain their original ID as a tombstone (`src/commands/mod.rs:418-433`)
- Deleted snippets are excluded from TUI display but retained for sync (`src/commands/mod.rs:197`)

## Sync Identity Semantics

- Snippet IDs are the merge key across devices (`src/sync_commands.rs:769-770`)
- Last-write-wins conflict resolution uses `updated_at` timestamp (`src/sync_commands.rs:821`)
- Locally deleted snippets are never resurrected by newer server copies (`src/sync_commands.rs:803-820`)
- Server-deleted snippets are marked `deleted=true` locally (data preserved, not removed) (`src/sync_commands.rs:778-799`)
- `output` is local-only — not synced, not in `ProtoSnippet` (`src/sync_commands.rs:1141-1182`)
- `device_id` and `deleted` fields are sanitized on import (`src/commands/import_cmd.rs:119-132`)
