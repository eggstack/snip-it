# Architecture Review Plan

**Date:** 2026-05-29
**Scope:** All architecture documents in `architecture/` directory
**Output:** Improvement plans per module in `plans/` directory

---

## Overview

This plan coordinates a systematic review of the snip-it architecture. Each architecture document describes a discrete module. The review process will:

1. Verify claims in each document against actual code
2. Interrogate the code for bugs, design issues, and improvement opportunities
3. Write per-module improvement plans into `plans/`
4. Prune stale architecture documents and outdated information

---

## Module Review Assignments

Each module below is assigned to a subagent. The subagent must:

1. **Read the architecture document** in `architecture/<module>.md`
2. **Trace claims to code** — verify that file paths, function names, structs, and behaviors described in the document match the actual implementation
3. **Interrogate the code** — look for:
   - Bugs (logic errors, edge cases, error handling gaps)
   - Design issues (tight coupling, unclear responsibilities, dead code)
   - Missing or inconsistent documentation
   - Security concerns (especially in encryption, sync, server modules)
   - Performance issues
   - Test coverage gaps
4. **Write an improvement plan** to `plans/<module>_review.md` containing:
   - Summary of discrepancies found between document and code
   - Bugs identified with code locations
   - Design improvement opportunities (no direct code changes)
   - Priority ranking (critical/high/medium/low)

### Module List

| # | Module | Doc | Key Source Files |
|---|--------|-----|-----------------|
| 1 | `overview` | `architecture/overview.md` | `src/main.rs`, project root |
| 2 | `cli` | `architecture/cli.md` | `src/main.rs`, `src/commands/` |
| 3 | `clipboard` | `architecture/clipboard.md` | `src/clipboard.rs` |
| 4 | `config` | `architecture/config.md` | `src/config.rs` |
| 5 | `core` | `architecture/core.md` | `src/library.rs`, `src/error.rs` |
| 6 | `encryption` | `architecture/encryption.md` | `src/encryption.rs` |
| 7 | `logging` | `architecture/logging.md` | `src/logging.rs` |
| 8 | `proto` | `architecture/proto.md` | `snip-proto/` |
| 9 | `server` | `architecture/server.md` | `snip-sync/src/` |
| 10 | `sync` | `architecture/sync.md` | `src/sync.rs`, `src/sync_commands.rs` |
| 11 | `ui` | `architecture/ui.md` | `src/ui.rs` |
| 12 | `utils` | `architecture/utils.md` | `src/utils/` |

### Execution Order

Reviews are independent and may run in parallel. Recommended grouping by dependency:

- **Phase 1 (Foundation):** `overview`, `core`, `config`, `utils`
- **Phase 2 (Features):** `cli`, `clipboard`, `encryption`, `logging`
- **Phase 3 (Infrastructure):** `proto`, `server`, `sync`, `ui`

---

## Stale Item Pruning

After all module reviews complete:

1. **Scan `architecture/` for orphaned documents** — files that describe modules no longer in the codebase
2. **Compare doc paths against `src/` structure** — any file paths in architecture docs that no longer exist indicate stale claims
3. **Check for renamed/refactored modules** — if a module was split or merged, determine if the old doc should be updated or removed
4. **Prune action:** Update or delete stale documents, noting changes in `plans/stale_pruning_report.md`

---

## Output Structure

```
plans/
├── overview_review.md
├── cli_review.md
├── clipboard_review.md
├── config_review.md
├── core_review.md
├── encryption_review.md
├── logging_review.md
├── proto_review.md
├── server_review.md
├── sync_review.md
├── ui_review.md
├── utils_review.md
└── stale_pruning_report.md
```

---

## Instructions for Subagents

Each subagent receives a module name and must:

```bash
# Read the architecture document
cat architecture/<module>.md

# Find all source files referenced
grep -r "src/" architecture/<module>.md

# Cross-reference with actual source
# Verify every claim, trace every path
# Look for bugs, dead code, security issues
# Write findings to plans/<module>_review.md
```

**Do NOT propose code changes.** Document findings as observations and improvement opportunities only.

---

## Completion Criteria

- [x] All 12 module reviews written to `plans/`
- [x] Stale pruning report written to `plans/stale_pruning_report.md`
- [x] All `plans/*.md` files committed

## Completion Status

**Completed:** 2026-05-29

All 13 output files (12 reviews + 1 stale pruning report) have been generated and committed. Each review verifies architecture document claims against actual source code and identifies bugs, design issues, security concerns, and performance issues with priority rankings.

### Output Files

| File | Module | Key Findings |
|------|--------|--------------|
| `plans/overview_review.md` | overview | Argon2 memory cost 64 KiB (OWASP min: 19 MiB), rate limiting gaps on 2 endpoints, CORS warning misleading |
| `plans/cli_review.md` | cli | Sync fall-through bug (critical), `_config` flag silently ignored in 4 commands, 0 tests in 12 of 13 modules |
| `plans/clipboard_review.md` | clipboard | Auto-clear race condition, `copypasta` compiled without default features |
| `plans/config_review.md` | config | Migration silently loses data, API key in plaintext, dead test |
| `plans/core_review.md` | core | `set_primary()` no-ops on missing filename, duplicate metadata on repeated imports, zero timestamps in `Snippet::new()` |
| `plans/encryption_review.md` | encryption | Argon2 64 KiB (P0), `hash_password` API misuse, no parameter versioning |
| `plans/logging_review.md` | logging | `config.level` dead field, shutdown logs after guard drop, audit log no file locking |
| `plans/proto_review.md` | proto | Missing `cargo:rerun-if-changed`, generated code drift risk, no proto versioning |
| `plans/server_review.md` | server | CORS blocks instead of allows, registration rate limit bypassable, no TLS, Argon2 64 KiB |
| `plans/sync_review.md` | sync | Encryption failures cause permanent snippet loss, Argon2 per-snippet redundant, API key for both auth+encryption |
| `plans/ui_review.md` | ui | HashSet linear scan instead of `contains()`, 1416-line monolith, `Mutex<Theme>` unnecessary for `Copy` type |
| `plans/utils_review.md` | utils | Unmatched `<` silently drops character, angle-bracket parsing duplicated 3x, `once_cell` vs `LazyLock` inconsistency |
| `plans/stale_pruning_report.md` | all | No orphaned docs, no stale references, 3 minor claim updates needed |
