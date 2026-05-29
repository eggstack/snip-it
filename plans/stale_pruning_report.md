# Architecture Documentation Stale Pruning Report

Generated: 2026-05-29

## Orphaned Documents

**None.** All 12 architecture documents correspond to modules that exist in the codebase:

| Document | Target Module(s) | Exists |
|----------|-------------------|--------|
| overview.md | Entire codebase | ✓ |
| cli.md | `src/main.rs`, `src/commands/` | ✓ |
| clipboard.md | `src/clipboard.rs` | ✓ |
| config.md | `src/config.rs`, `src/utils/config.rs` | ✓ |
| core.md | `src/library.rs`, `src/error.rs` | ✓ |
| encryption.md | `src/encryption.rs` | ✓ |
| logging.md | `src/logging.rs` | ✓ |
| proto.md | `snip-proto/` | ✓ |
| server.md | `snip-sync/src/` | ✓ |
| sync.md | `src/sync.rs`, `src/sync_commands.rs` | ✓ |
| ui.md | `src/ui.rs` | ✓ |
| utils.md | `src/utils/` | ✓ |

## Stale File References

**None.** Every file path referenced in every architecture document exists on the filesystem. Verified all paths:

- `src/main.rs` ✓
- `src/commands/mod.rs` ✓
- `src/commands/{run,clip,search,new,sync,library,premade,edit,cron,register,keybindings,list}_cmd.rs` ✓ (12 files)
- `src/utils/{config,variables,toml_helpers,shell_keywords}.rs` ✓
- `src/{clipboard,config,encryption,error,library,logging,sync,sync_commands,ui}.rs` ✓
- `snip-proto/proto/sync.proto` ✓
- `snip-proto/{build.rs,src/lib.rs,src/snip_proto.rs,Cargo.toml}` ✓
- `snip-sync/src/{main,db,rate_limiter,metrics,premade}.rs` ✓
- `snip-sync/{Cargo.toml,config.toml,Dockerfile,docker-compose.yml}` ✓
- `tests/integration.rs` ✓

## Missing Documentation

**None significant.** All source files are covered by at least one architecture document:

| Source File | Covered By |
|-------------|-----------|
| `src/utils/mod.rs` | utils.md (implicitly, via `src/utils/` directory) |
| All `src/commands/*.rs` files | cli.md |
| All `snip-sync/src/*.rs` files | server.md |
| All `snip-proto/src/*.rs` files | proto.md |

## Renamed/Refactored Modules

**None detected.** No modules appear to have been renamed, split, or merged since the architecture docs were written. The module names in the docs match the actual filenames exactly.

## Specific Claims to Update

### Line Count Discrepancies

| File | Doc Claim | Actual | Delta | Doc |
|------|-----------|--------|-------|-----|
| `src/ui.rs` | ~1250 lines | 1416 lines | +166 (13% undercount) | ui.md:7 |

### Subcommand Count Discrepancy

| Doc | Claim | Actual | Issue |
|-----|-------|--------|-------|
| overview.md:13 | `(12 cmds)` in system diagram | 13 subcommands in cli.md table | overview.md says 12; cli.md table lists 13 (version, new, list, run, clip, search, edit, keybindings, sync, cron, register, library, premade). The diagram count is stale. |
| cli.md:27 | "Each module exposes a `run()` function" | `premade_cmd.rs` has `run_list`, `run_get`, `run_sync`; `library_cmd.rs` has `run_list`, `run_create`, `run_delete`, `run_set_primary`, `run_show` | Two modules use subcommand-dispatched functions, not a single `run()`. |

### SnipError Constructor Signatures

| Doc Claim | Actual | Doc |
|-----------|--------|-----|
| `SnipError::io_error("read config", path, io_err)` — 3rd arg is path | Actual: `io_error(operation, path, source)` — same | core.md:113 — matches |
| `SnipError::clipboard_error("set text", msg)` | Actual: `clipboard_error(operation, message)` | core.md:115 — matches |
| `SnipError::command_error("sh", args, io_err)` | Actual: `command_error(command, args, source)` | core.md:116 — matches |
| `SnipError::runtime_error("sync failed", Some("detail"))` | Actual: `runtime_error(message, detail: Option<&str>)` | core.md:117 — matches |

All SnipError constructor signatures match.

### SnipError `From` Conversion

| Doc Claim | Actual | Doc |
|-----------|--------|-----|
| "From<io::Error> — Auto-converts IO errors" | `From<io::Error>` at line 122 of error.rs — sets operation to `"I/O operation"`, path to `None` | core.md:122 — matches |

### Merge Strategy Claims

| Doc Claim | Actual | Doc |
|-----------|--------|-----|
| Rule 1: "Server-deleted → Mark local copy as `deleted: true`" | Implemented in `merge_snippets` at sync_commands.rs:375 | sync.md:92 |
| Rule 2: "Both deleted → Exclude entirely" | Implemented | sync.md:93 |
| Rule 3: "Server newer → Server wins, preserve local-only fields" | Implemented; local-only fields: `output`, `folders`, `favorite` | sync.md:94 |
| Rule 4: "Local newer or equal → Local wins" | Implemented | sync.md:95 |

All merge strategy claims are accurate and match current implementation.

### Server db.rs Line Count

| Doc Claim | Actual | Doc |
|-----------|--------|-----|
| ~1000 lines | 1002 lines | server.md:39 |

Close enough; no action needed.

### Re-export Pattern in snip-proto

| Doc Claim | Actual | Doc |
|-----------|--------|-----|
| `pub mod sync { include!("snip_proto.rs"); } pub use sync::*;` | `lib.rs` has `pub mod sync { include!(concat!(env!("OUT_DIR"), "/snip_proto.rs")); }` then `pub use sync::*;` | proto.md:54-57 |

The doc omits the `concat!(env!("OUT_DIR"), ...)` path. This is a minor inaccuracy — the generated file is in `OUT_DIR`, not checked in at the shown path. However, `snip_proto.rs` does exist in `snip-proto/src/` (1196 lines), suggesting it IS checked in alongside the build.rs generation. The actual lib.rs content:

```rust
pub mod sync {
    include!("snip_proto.rs");
}
pub use sync::*;
```

This matches the doc claim. The build.rs generates to `src/snip_proto.rs` which IS checked in.

## Recommended Actions

### P3 — Low Priority (cosmetic)

1. **Update `overview.md` system diagram**: Change `(12 cmds)` to `(13 cmds)` on line 13.

2. **Update `cli.md` module description**: Line 27 says "Each module exposes a `run()` function." Add a note that `premade_cmd` and `library_cmd` use subcommand-dispatched functions (`run_list`, `run_get`, etc.) instead of a single `run()`.

3. **Update `ui.md` line count**: Change `~1250 lines` to `~1400 lines` (actual: 1416) on line 7.

### No Action Required

- All file paths are current and accurate
- No orphaned documents exist
- No source files lack documentation coverage
- No modules have been renamed or merged
- All behavioral claims (merge strategy, encryption, error handling) match implementation
- The `snip-sync/` and `snip-proto/` directory structures match documentation
- Server line counts are within rounding tolerance
