# Core Data Model

[← Back to Overview](overview.md)

## Snippet & Snippets

**File**: `src/library.rs`

### `Snippet` struct

Individual snippet with metadata:

```rust
pub struct Snippet {
    pub id: String,           // UUID v4, generated on first sync
    pub description: String,  // Human-readable name
    pub command: String,      // Shell command (may contain <variables>)
    pub output: String,       // Output file path (relative, validated)
    pub tags: Vec<String>,    // User-defined tags
    pub folders: Vec<String>, // Folder organization
    pub favorite: bool,       // Starred flag
    pub created_at: i64,      // Unix timestamp
    pub updated_at: i64,      // Unix timestamp
    pub device_id: String,    // Originating device
    pub deleted: bool,        // Soft-delete flag
}
```

### `Snippets` struct

Container for a collection:

```rust
pub struct Snippets {
    pub snippets: Vec<Snippet>,
    pub folders: Vec<String>,
}
```

### TOML Format

```toml
[[Snippets]]
Id = "uuid-here"
Description = "git commit"
Command = "git commit -m \"<msg>\""
Tag = ["git", "version-control"]
Output = ""
favorite = false
created_at = 1234567890
updated_at = 1234567890
```

Compatible with `pet` snippet manager format (supports `Description`, `Command`, `Tag` aliases).

## LibraryManager

**File**: `src/library.rs`

Manages multiple snippet libraries:

### Modes

- **Single-file mode** — Legacy, uses `~/.config/snp/snippets.toml`
- **Library mode** — Default, uses `~/.config/snp/libraries/*.toml`

### Library Configuration

Stored in `~/.config/snp/libraries.toml`:

```toml
[[libraries]]
filename = "snippets"
library_id = "server-uuid"
is_primary = true
last_sync = 1234567890
```

### Operations

- `create_library(name)` — Create new .toml file + register in config
- `delete_library(name)` — Remove file + config entry, reassign primary
- `set_primary(name)` — Mark one library as default
- `migrate_from_single_file()` — One-time migration from legacy format
- `add_server_library(name, id)` — Import library from sync server
- `load_library(path)` / `save_library(path, snippets)` — TOML I/O with error recovery
- `backup_library(path)` — Timestamped backup to `backups/` subdirectory

### Validation

Library names are validated:
- Non-empty, max 50 chars
- No slashes (`/`, `\`) or null bytes
- Prevents path traversal

## Error Handling

**File**: `src/error.rs`

```rust
pub enum SnipError {
    Io { operation, path, source },
    Toml { operation, source },
    Clipboard { operation, message },
    Command { command, args, source },
    Runtime { message, detail },
}
```

### Convenience Constructors

```rust
SnipError::io_error("read config", path, io_err)
SnipError::toml_error("serialize", toml_err)
SnipError::clipboard_error("set text", msg)
SnipError::command_error("sh", args, io_err)
SnipError::runtime_error("sync failed", Some("detail"))
```

### Conversions

- `From<io::Error>` — Auto-converts IO errors
- `SnipResult<T> = Result<T, SnipError>` — Standard result alias

## Key Files

- `src/library.rs` — Snippet, Snippets, LibraryManager, load/save/backup
- `src/error.rs` — SnipError enum, constructors, Display impl
- `src/commands/mod.rs` — `load_snippets()`, `save_snippets()` (thin wrappers)
