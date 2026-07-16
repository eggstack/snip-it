# Architecture Review Plan

> **Historical Document**: This captured the review process used in 2026-05-29. The review
> is complete. New commands added since then (`select_cmd`, `shell_cmd`, `doctor_cmd`,
> `import_cmd`, `pet_analysis`) do not have architecture docs yet.

This document outlines a systematic review process for all architecture documentation in the `architecture/` directory (excluding this file). For each module, subagents will verify claims against the actual codebase and produce improvement plans in the `plans/` directory.

## Modules to Review

| # | Module | Architecture File(s) | Source Code Location |
|---|--------|---------------------|---------------------|
| 1 | CLI Entry Point | `cli.md` | `src/main.rs` |
| 2 | Clipboard | `clipboard.md` | `src/clipboard.rs` |
| 3 | Commands | `commands/*.md` | `src/commands/` |
| 4 | Config | `config.md` | `src/config.rs`, `src/utils/config.rs` |
| 5 | Core | `core.md` | `src/library.rs`, `src/error.rs` |
| 6 | Encryption | `encryption.md` | `src/encryption.rs` |
| 7 | Library | `library.md` | `src/library.rs` |
| 8 | Logging | `logging.md` | `src/logging.rs` |
| 9 | Overview | `overview.md` | All files |
| 10 | Proto | `proto.md` | `snip-proto/` |
| 11 | Server | `server.md` | `snip-sync/src/` |
| 12 | Sync | `sync.md` | `src/sync.rs`, `src/sync_commands.rs` |
| 13 | TUI | `tui.md` | `src/ui/mod.rs`, `src/ui/theme.rs`, `src/ui/highlight.rs`, `src/ui/variables.rs` |
| 14 | UI | `ui.md` | `src/ui/` |
| 15 | Utils | `utils.md`, `utils/variables.md` | `src/utils/` |

## Review Process

For each module, a subagent will:
1. Read the architecture document(s)
2. Identify all claims, design decisions, and specifications
3. Locate and read the corresponding source code
4. Verify each claim against the implementation
5. Interrogate the code for:
   - Bugs or edge cases not addressed in documentation
   - Potential improvements not documented
   - Discrepancies between documented behavior and actual implementation
   - Missing error handling
   - Security concerns
   - Performance considerations
6. Write an improvement plan to `plans/<module_name>.md`

## Subagent Tasks

### 1. CLI Review
- **File**: `architecture/cli.md`
- **Source**: `src/main.rs`, command dispatch logic
- **Plan output**: `plans/cli.md`

### 2. Clipboard Review
- **File**: `architecture/clipboard.md`
- **Source**: `src/clipboard.rs`
- **Plan output**: `plans/clipboard.md`

### 3. Commands Review
- **Files**: `architecture/commands/*.md` (13 files)
- **Source**: `src/commands/` (13 modules)
- **Plan output**: `plans/commands.md`

### 4. Config Review
- **File**: `architecture/config.md`
- **Source**: `src/config.rs`, `src/utils/config.rs`
- **Plan output**: `plans/config.md`

### 5. Core Review
- **File**: `architecture/core.md`
- **Source**: `src/library.rs`, `src/error.rs`
- **Plan output**: `plans/core.md`

### 6. Encryption Review
- **File**: `architecture/encryption.md`
- **Source**: `src/encryption.rs`
- **Plan output**: `plans/encryption.md`

### 7. Library Review
- **File**: `architecture/library.md`
- **Source**: `src/library.rs`
- **Plan output**: `plans/library.md`

### 8. Logging Review
- **File**: `architecture/logging.md`
- **Source**: `src/logging.rs`
- **Plan output**: `plans/logging.md`

### 9. Overview Review
- **File**: `architecture/overview.md`
- **Source**: All source files
- **Plan output**: `plans/overview.md`

### 10. Proto Review
- **File**: `architecture/proto.md`
- **Source**: `snip-proto/`
- **Plan output**: `plans/proto.md`

### 11. Server Review
- **File**: `architecture/server.md`
- **Source**: `snip-sync/src/`
- **Plan output**: `plans/server.md`

### 12. Sync Review
- **File**: `architecture/sync.md`
- **Source**: `src/sync.rs`, `src/sync_commands.rs`
- **Plan output**: `plans/sync.md`

### 13. TUI Review
- **File**: `architecture/tui.md`
- **Source**: `src/ui/mod.rs`, `src/ui/theme.rs`, `src/ui/highlight.rs`, `src/ui/variables.rs`
- **Plan output**: `plans/tui.md`

### 14. UI Review
- **File**: `architecture/ui.md`
- **Source**: `src/ui/`
- **Plan output**: `plans/ui.md`

### 15. Utils Review
- **Files**: `architecture/utils.md`, `architecture/utils/variables.md`
- **Source**: `src/utils/`
- **Plan output**: `plans/utils.md`

## Stale Item Detection

After all reviews complete, a final check will:
1. Compare `architecture/` contents against actual source code structure
2. Identify any documentation for modules that no longer exist
3. Flag missing documentation for new modules discovered in source
4. List outdated files that reference deprecated functionality

## Execution Order

Subagents should be launched in batches of 3-4 for parallel processing:

**Batch 1**: CLI, Clipboard, Config
**Batch 2**: Core, Encryption, Library
**Batch 3**: Logging, Overview, Proto
**Batch 4**: Server, Sync, Utils
**Batch 5**: Commands, TUI, UI (larger modules)

## Review Plan Metadata

- **Created**: 2026-05-29
- **Total modules**: 15
- **Estimated subagents**: 15

## Stale Item Detection Results

After reviewing all architecture documentation against the actual source code structure, the following items were identified:

### Files in Architecture Not Backed by Source

| Architecture File | Status | Notes |
|-----------------|--------|-------|
| `commands/mod.md` | Valid | References `src/commands/mod.rs` which exists |
| All other files | Valid | Each architecture file has corresponding source |

### Source Files Without Architecture Documentation

| Source File/Directory | Notes |
|----------------------|-------|
| `src/utils/toml_helpers.rs` | No dedicated doc (referenced in utils.md) |
| `src/utils/shell_keywords.rs` | No dedicated doc (referenced in utils.md) |

### Architecture Files to Prune (Outdated or Obsolete)

None identified at this time. All architecture documents correspond to existing modules.

### Verification Complete

All 17 entries in the `architecture/` directory map to existing source code locations. No stale documentation files require removal.

---
*Review status: INCOMPLETE - Implementation phase in progress*

*Review completed: 2026-05-29*