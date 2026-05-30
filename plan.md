# snip-it Remediation Plan

**Last updated:** 2026-05-30

## Status Overview

All WAVE 1, 2, and 3 items have been completed. WAVE 4 items are deferred.

**WAVE 1 (Security-Critical):** COMPLETED
**WAVE 2 (Core Bugs):** COMPLETED
**WAVE 3 (Improvements):** COMPLETED
**WAVE 4 (Low Priority):** DEFERRED

---

## KNOWN / ACCEPTED LIMITATIONS

### Scope Constraints (Cannot Fix Without Breaking Change)

#### LIM-1: output Field Not Encrypted During Sync
- **Status:** Known Limitation
- **Location:** `src/sync.rs:60-64`
- **Description:** The `output` field is not encrypted during sync because the proto definition lacks the field. Cannot add without breaking API change.

#### LIM-2: \< Escape Inconsistency in variables.rs
- **Status:** Known Limitation (Documented)
- **Location:** `src/utils/variables.rs`
- **Description:** `\<` escape sequence inconsistency between parse and expand is a documented known edge case.

#### LIM-3: CLI Documentation Discrepancies
- **Status:** Known Limitation (Doc Bugs)
- **Location:** Multiple
- **Description:** Many CLI documentation discrepancies (e.g., `--clip` behavior, cron intervals) are doc bugs not code bugs.

---

## DEFERRED ITEMS (WAVE 4)

These items are low priority and deferred for future work.

### TUI Improvements
| ID | Description | Files |
|----|-------------|-------|
| TUI-2 | Add signal handling for clean exit | `src/ui/mod.rs` |
| TUI-3 | Fix keybinding documentation or code | `src/ui/mod.rs:831,838,843-846` |
| TUI-4 | Terminal size change handling | `src/ui/mod.rs` |
| TUI-6 | Unmatched angle bracket creates phantom variable | `src/utils/highlight.rs:39-49` |
| TUI-7 | Visual mode navigation inconsistent | `src/ui/mod.rs:831,838` |
| TUI-8 | `gg` keybinding missing | `src/ui/mod.rs:827` |
| TUI-9 | Esc key inconsistent behavior | `src/ui/mod.rs:867-869` |
| TUI-10 | Sort keys don't match documentation | `src/ui/mod.rs:843-846` |
| TUI-11 | Double-click execution not documented | `src/ui/mod.rs:591-593` |
| TUI-12 | Double-click detection logic flaw | `src/ui/mod.rs:581-604` |
| TUI-13 | Spurious DisableMouseCapture in variable prompt | `src/ui/variables.rs:140-143` |
| TUI-14 | Fuzzy match score not used consistently | `src/ui/mod.rs:283-311` |
| TUI-15 | Enter key behavior inconsistency in search mode | `src/ui/mod.rs:742-746` |
| TUI-16 | Visual mode selection range includes visual start | `src/ui/mod.rs:646-686` |
| TUI-17 | Variable prompt limit of 10 | `src/ui/variables.rs:65` |
| TUI-18 | Create SelectState struct | `src/ui/mod.rs` |
| TUI-19 | Unmatched variable warning | `src/ui/mod.rs` |
| TUI-20 | Visual mode boundaries | `src/ui/mod.rs` |
| TUI-21 | Add `gg` as alternative to `Ctrl+g` | `src/ui/mod.rs` |
| TUI-22 | Error propagation for event::poll and event::read | `src/ui/mod.rs:559` |
| TUI-23 | Theme caching optimization | `src/ui/theme.rs:64-66` |

### Command Improvements
| ID | Description | Files |
|----|-------------|-------|
| CMD-12 | Multiple LibraryManager instantiations per command | Throughout commands |
| CMD-13 | Add re-registration support | `src/commands/register_cmd.rs` |
| CMD-14 | Add --dry-run for sync | `src/commands/sync_cmd.rs` |
| CMD-15 | Add timeout for editor | `src/commands/edit_cmd.rs` |
| CMD-16 | Consider --json/--csv for list_cmd | `src/commands/list_cmd.rs` |

### Config Improvements
| ID | Description | Files |
|----|-------------|-------|
| CONFIG-3 | No atomic write for sync.toml | `src/config.rs:166` |
| CONFIG-8 | Missing documentation | Documentation |

### Logging Improvements
| ID | Description | Files |
|----|-------------|-------|
| LOG-1 | shutdown_logging may lose buffered logs | `src/logging.rs:101-106` |
| LOG-4 | Add rotation policy configuration | `src/logging.rs` |
| LOG-8 | log_config_operation error type limitation | `src/logging.rs:172` |
| LOG-9 | LOG_GUARD mutex poisoning | `src/logging.rs:26-27,102,86` |
| LOG-10 | Add log level filter per module | `src/logging.rs` |
| LOG-11 | Add structured metadata to audit log | `src/logging.rs` |
| LOG-12 | Add async audit log writer | `src/logging.rs` |
| LOG-13 | Add startup self-check | `src/logging.rs` |
| LOG-14 | Add structured error context to log_command_execution | `src/logging.rs` |
| LOG-15 | Implement tracing::instrument for function spans | `src/logging.rs` |

### Library Improvements
| ID | Description | Files |
|----|-------------|-------|
| LIB-5 | Add confirmation before delete_library() | `src/library.rs` |
| LIB-7 | No validation of library_id format | `src/library.rs` |
| LIB-8 | Snippet fields renamed on deserialization | `src/library.rs` |
| LIB-9 | Empty folders array serializes inconsistently | `src/library.rs` |
| LIB-10 | TOML regex edge case with escaped quotes | `src/utils/toml_helpers.rs:14-15` |
| LIB-11 | Add sort_by_updated_at() method | `src/library.rs` |
| LIB-12 | Consider using chrono DateTime instead of i64 | `src/library.rs` |
| LIB-13 | Expose library backup through LibraryManager | `src/library.rs` |
| LIB-14 | Document that deleted snippets filtered elsewhere | `src/library.rs` |
| CORE-12 | Library name validation missing path traversal check | `src/library.rs:98-121` |

### Sync Improvements
| ID | Description | Files |
|----|-------------|-------|
| SYNC-4 | Missing `delete_library` in SyncClient | `src/sync.rs` |
| SYNC-5 | Hardcoded sync limit | `src/sync.rs` |
| SYNC-6 | Configurable retry parameters | `src/sync.rs` |
| SYNC-7 | Missing get_snippets and push_snippets methods | `src/sync.rs` |
| SYNC-8 | Device ID conflict detection | `src/sync.rs` |
| SYNC-9 | Library identification by filename | `src/sync_commands.rs:187` |
| SYNC-10 | Sync status reporting clarity | `src/sync_commands.rs` |
| SYNC-11 | Backup on merge failure | `src/library.rs` |
| SYNC-12 | Retryable vs non-retryable error classification | `src/sync.rs` |

### Server Improvements
| ID | Description | Files |
|----|-------------|-------|
| SERVER-7 | Make MAX_REQUEST_LIMIT configurable | `snip-sync/src/main.rs:35` |
| SERVER-9 | Graceful shutdown for rate limiter cleanup task | `snip-sync/src/rate_limiter.rs` |
| SERVER-11 | Hardcoded MAX_REQUEST_LIMIT magic number | `snip-sync/src/main.rs:35` |
| SERVER-12 | Add SQL injection defense in library delete | `snip-sync/src/main.rs` |
| SERVER-13 | Document deleted snippet merge semantics | `snip-sync/src/main.rs`, `src/sync_commands.rs` |
| SERVER-14 | Consider TLS warning in health/ready endpoint | `snip-sync/src/main.rs` |

### Proto Improvements
| ID | Description | Files |
|----|-------------|-------|
| PROTO-3 | Missing pagination on premade library endpoints | `snip-sync/src/main.rs:755-782` |
| PROTO-4 | Health check returns hardcoded `healthy: true` | `snip-sync/src/main.rs:343-352` (FIXED) |
| PROTO-5 | Add request ID / correlation ID | `proto/sync.proto` |
| PROTO-6 | Batch API key verification | `snip-sync/src/db.rs:216-234` |
| PROTO-7 | Missing index for deleted snippets query | `snip-sync/src/db.rs:372-437` |
| PROTO-8 | Premade library content not validated | `snip-sync/src/main.rs:784-821` |
| PROTO-9 | Sync skipped snippets not persisted | `snip-sync/src/main.rs:578-608` |
| PROTO-10 | Register endpoint device ID not validated | `snip-sync/src/main.rs:389` |
| PROTO-11 | CORS wildcard in development | `snip-sync/src/main.rs:912-914` |

### Encryption Improvements
| ID | Description | Files |
|----|-------------|-------|
| ENCRYPT-1 | Documentation outdated (memory cost) | Documentation |
| ENCRYPT-2 | Could use `std::mem::take` for explicit key cleanup | `src/encryption.rs` |
| ENCRYPT-3 | Missing ZeroizeDerive for DerivedKey | `src/encryption.rs` |
| ENCRYPT-4 | Constant-time comparison for salt/nonce extraction | `src/encryption.rs` |

### Clipboard Improvements
| ID | Description | Files |
|----|-------------|-------|
| CLIP-5 | Thread leak on rapid scheduling | `src/clipboard.rs` |
| CLIP-6 | No clipboard content type preservation | `src/clipboard.rs` (Known Limitation) |
| CLIP-7 | Clipboard operations have no timeout | `src/clipboard.rs` |

### Config Improvements (Additional)
| ID | Description | Files |
|----|-------------|-------|
| CONFIG-5 | Config files use default permissions | `src/config.rs`, `src/utils/config.rs` (Known Limitation) |
| CONFIG-6 | No integrity checking for sync.toml | `src/config.rs` (Known Limitation) |
| CONFIG-7 | TOML parsing on every load | `src/config.rs` (Known Limitation) |

### Utils Improvements
| ID | Description | Files |
|----|-------------|-------|
| UTILS-1 | Escape sequence handling inconsistency | `src/utils/variables.rs` |
| UTILS-2 | Double-backslash before angle bracket loses character | `src/utils/variables.rs:111` |
| UTILS-3 | expand_command return type mismatch | `src/utils/variables.rs` (Documentation Issue) |
| UTILS-4 | Chained backslash escape edge case | `src/utils/variables.rs` |
| UTILS-5 | Nested angle brackets edge case | `src/utils/variables.rs` |
| UTILS-6 | Backslash at end of variable content | `src/utils/variables.rs` |

### Other Improvements
| ID | Description | Files |
|----|-------------|-------|
| OV-1 | Add command timeout for snippet execution | `src/commands/run_cmd.rs:116-134` |
| OV-2 | Rate limiter should support persistence | `snip-sync/src/rate_limiter.rs` |
| OV-3 | Snippet ID collision on merge | `src/sync_commands.rs:394-475` |
| OV-4 | Missing input validation on snippet creation | `src/commands/new_cmd.rs` |

---

## CLI Bugs (Remaining TODO)
| ID | Description | Files |
|----|-------------|-------|
| CLI-5 | Multiline input reads until double-empty-line | `src/commands/new_cmd.rs:8-25` |
| CLI-7 | `register_cmd` inconsistent error handling | `src/commands/register_cmd.rs:51-54` |

---

## Summary Statistics

| Category | Count |
|----------|-------|
| WAVE 1 Security | 6 (COMPLETED) |
| WAVE 2 Core Bugs | 17 (COMPLETED) |
| WAVE 3 Improvements | 24 (COMPLETED) |
| WAVE 4 Low Priority | 85+ (DEFERRED) |
| Known Limitations | 6 |
| **Total Remaining** | ~90 items (deferred or known limitation) |