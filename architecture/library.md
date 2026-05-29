# Library Module (`library.rs`)

## Overview

The library module is the core data layer of snip-it. It defines the `Snippet` and `Snippets` data structures, handles TOML serialization/deserialization, and provides the `LibraryManager` for CRUD operations on snippets.

## Data Structures

### Snippet

```rust
pub struct Snippet {
    pub id: Uuid,
    pub name: String,
    pub command: String,
    pub output: Option<String>,
    pub tags: Vec<String>,
    pub folders: Vec<String>,
    pub favorite: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted: bool,
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
- `SnipError::LibraryNotFound` for missing library operations
