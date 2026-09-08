# Library Module (`src/library/`)

## Overview

The library module is the core data layer of snip-it. It defines the `Snippet` and `Snippets` data structures, handles TOML serialization/deserialization, and provides the `LibraryManager` for CRUD operations on snippets.

## Data Structures

### Snippet

```rust
pub struct Snippet {
    pub id: String,              // Opaque ID; UUID for new, deterministic for legacy
    pub description: String,     // Human-readable name
    pub output: String,          // Output file path
    pub tags: Vec<String>,       // User-defined tags
    pub command: String,         // Shell command (may contain <variables>)
    pub folders: Vec<String>,    // Folder organization
    pub favorite: bool,          // Starred flag
    pub created_at: i64,         // Unix timestamp
    pub updated_at: i64,         // Unix timestamp
    pub device_id: String,       // Originating device
    pub deleted: bool,           // Soft-delete flag
}
```

### Snippets

Wrapper struct that holds a list of snippets and serializes to TOML with a `[[snippets]]` table array.

### LibraryMeta & LibraryConfig

Metadata and configuration for multi-library support.

## LibraryManager

`LibraryManager` provides:
- `load_library()` — Load from TOML file
- `save_library()` — Save to TOML file with backup
- `backup_library()` — Create timestamped backup
- Migration from single-file to multi-library mode
- Premade library tracking

## Read-only source resolution (Plan 009)

Side-effect-free library discovery lives in the library layer, not in
callers:

- `ResolvedLibrarySource` — canonical `{ name, library_id, path }` for one
  visible library.
- `LibraryManager::resolve_readonly_sources()` — single implementation for
  legacy single-file handling (`None`/`"all"`/`"snippets"` → implicit
  `snippets` library), primary (`None`), named, and `all` scopes. Never
  calls `ensure_library_mode`; never creates directories/files or rewrites
  metadata.
- `readonly_library_sources()` — convenience wrapper (`new()` + resolve).
- `library_not_found()` — canonical missing-library error shared by CLI,
  MCP, and diagnostics.

Consumers: MCP `tools.rs` (`list`/`search`/`get`) and
`selector::resolve_selector_readonly()` (used by `snp get`). Mutating paths
(`resolve_selector`, `get_library_path`, `init_library_manager`) keep the
migrating behavior.

## Shared index inspection (Plan 009)

`LibraryManager::inspect_library_index()` returns a read-only
`LibraryIndexInspection` (`missing_files`, `orphan_files`, `primary:
PrimaryState`) built from existence checks and directory listing only.
`find_orphaned_ids()` is the shared pure classifier for orphaned usage
entries. `doctor`, `validate`, `repair`, and `status` render this shared
state into their own diagnostic types and keep their distinct user
semantics; no generic finding DSL or plugin framework was introduced.

## File Layout

```
src/library/
├── mod.rs          # Re-exports preserving `crate::library::*` paths
├── model.rs        # `Snippet`, `Snippets`, `LibraryConfig`, `LibraryMeta`,
│                   #   `ResolvedLibrarySource`, `PrimaryState`,
│                   #   `LibraryIndexInspection`, pure helpers
├── persistence.rs  # `load_library`, `save_library`, ID normalization, backups
├── manager.rs      # `LibraryManager`, read-only resolver
└── tests.rs        # Unit tests for the above
```

## Config File Layout

```
~/.config/snp/
├── snippets.toml          # Single-file (legacy)
├── libraries.toml        # Library metadata
└── libraries/
    └── *.toml            # Individual library files
```

## Key Behaviors

- **TOML Handling**: Uses `toml` crate for serialization. Snippets sorted by `updated_at` descending on save.
- **Backup**: Automatic backup before save using `backup_library()`
- **Migration**: Detects old `snippets.toml` and migrates to multi-library structure
- **Soft Delete**: `deleted: true` marks snippet as deleted (preserved in data, excluded from UI)

## Error Handling

- `SnipError::Io` for file operations
- `SnipError::Toml` for serialization errors
- `SnipError::Runtime` for validation errors (e.g., path traversal in library names)
