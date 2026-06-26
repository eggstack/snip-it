# Library Module (`library.rs`)

## Overview

The library module is the core data layer of snip-it. It defines the `Snippet` and `Snippets` data structures, handles TOML serialization/deserialization, and provides the `LibraryManager` for CRUD operations on snippets.

## Data Structures

### Snippet

```rust
pub struct Snippet {
    pub id: String,              // UUID v4, generated on first sync
    pub description: String,     // Human-readable name
    pub command: String,         // Shell command (may contain <variables>)
    pub output: String,          // Output file path
    pub tags: Vec<String>,       // User-defined tags
    pub folders: Vec<String>,    // Folder organization
    pub favorite: bool,          // Starred flag
    pub created_at: i64,         // Unix timestamp
    pub updated_at: i64,         // Unix timestamp
    pub device_id: String,       // Originating device
    pub deleted: bool,           // Soft-delete flag
}
```

### Snippets

Wrapper struct that holds a list of snippets and serializes to TOML with a `[[snippet]]` table array.

### LibraryMeta & LibraryConfig

Metadata and configuration for multi-library support.

## LibraryManager

`LibraryManager` provides:
- `load_library()` — Load from TOML file
- `save_library()` — Save to TOML file with backup
- `backup_library()` — Create timestamped backup
- Migration from single-file to multi-library mode
- Premade library tracking

## File Layout

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
