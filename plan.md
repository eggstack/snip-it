# snip-it Remediation Plan

**Last updated:** 2026-05-30

## Status Overview

All WAVE 1, 2, 3, and most WAVE 4 items have been completed.

**WAVE 1 (Security-Critical):** COMPLETED
**WAVE 2 (Core Bugs):** COMPLETED
**WAVE 3 (Improvements):** COMPLETED
**WAVE 4 (Low Priority):** MOSTLY COMPLETED (see below)

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

## COMPLETED ITEMS (WAVE 4)

### TUI Improvements
| ID | Description | Status |
|----|-------------|--------|
| TUI-3 | Fix keybinding documentation | COMPLETED |
| TUI-8/TUI-21 | Add `gg` keybinding for jump to top | COMPLETED |
| TUI-9 | Esc key behavior documented | COMPLETED |
| TUI-10 | Sort keys docs match | COMPLETED |
| TUI-11/TUI-12 | Double-click bounds check + docs | COMPLETED |
| TUI-13 | Remove spurious DisableMouseCapture in variable prompt | COMPLETED |
| TUI-15 | Enter key consistency in search mode | COMPLETED |
| TUI-17 | Remove variable prompt limit of 10 | COMPLETED |
| TUI-22 | Error propagation for event::read | COMPLETED |

### Command Improvements
| ID | Description | Status |
|----|-------------|--------|
| CMD-13 | Add --force for re-registration | COMPLETED |
| CMD-14 | Add --dry-run for sync | COMPLETED |
| CMD-16 | Add --json/--csv for list_cmd | COMPLETED |
| OV-1 | Add timeout for snippet execution | COMPLETED |
| CLI-7 | register_cmd error handling | COMPLETED |

### Config Improvements
| ID | Description | Status |
|----|-------------|--------|
| CONFIG-3 | Atomic write for sync.toml | COMPLETED |

### Logging Improvements
| ID | Description | Status |
|----|-------------|--------|
| LOG-1 | Shutdown flush delay | COMPLETED |
| LOG-4 | Configurable audit log rotation | COMPLETED |
| LOG-9 | Mutex poisoning resilience | COMPLETED |
| LOG-11 | Structured audit log metadata | COMPLETED |
| LOG-13 | Startup self-check | COMPLETED |
| LOG-14 | Working directory context in logs | COMPLETED |
| LOG-15 | Tracing instrument spans | COMPLETED |

### Library Improvements
| ID | Description | Status |
|----|-------------|--------|
| LIB-7 | Library ID format validation | COMPLETED |
| LIB-11 | sort_by_updated_at() method | COMPLETED |
| LIB-12 | chrono DateTime documentation | COMPLETED |
| LIB-13 | Expose backup through LibraryManager | COMPLETED |
| CORE-12 | Path traversal check in library name | COMPLETED |
| OV-4 | Input validation on snippet creation | COMPLETED (empty description now rejected) |

### Sync Improvements
| ID | Description | Status |
|----|-------------|--------|
| SYNC-4 | delete_library in SyncClient | COMPLETED |
| SYNC-5 | Configurable sync limit | COMPLETED |
| SYNC-6 | Configurable retry parameters | COMPLETED |
| SYNC-12 | Retryable vs non-retryable errors | COMPLETED |

### Server Improvements
| ID | Description | Status |
|----|-------------|--------|
| SERVER-7/SERVER-11 | Configurable MAX_REQUEST_LIMIT | COMPLETED |
| SERVER-12 | UUID validation in library delete | COMPLETED |
| SERVER-13 | Document deleted snippet merge semantics | COMPLETED |
| SERVER-14 | TLS status in health endpoint | COMPLETED |

### Proto Improvements
| ID | Description | Status |
|----|-------------|--------|
| PROTO-7 | Composite index for deleted snippets query | COMPLETED |

### Encryption Improvements
| ID | Description | Status |
|----|-------------|--------|
| ENCRYPT-3 | Zeroize derive macro for DerivedKey | COMPLETED |

---

## REMAINING DEFERRED ITEMS

These items are low priority, require breaking changes, or are not worth implementing.

### TUI (Deferred)
| ID | Description | Reason |
|----|-------------|--------|
| TUI-2 | Signal handling for clean exit | Already handled by signal-hook in main.rs |
| TUI-4 | Terminal size change handling | Complex; terminal too small already handled |
| TUI-6 | Unmatched angle bracket phantom variable | Known edge case, documented |
| TUI-7 | Visual mode navigation inconsistent | Minor UI quirk |
| TUI-14 | Fuzzy match score not used consistently | Working correctly |
| TUI-16 | Visual mode selection range | Minor UI quirk |
| TUI-18 | Create SelectState struct | Refactoring, not a bug |
| TUI-19 | Unmatched variable warning | Minor UX improvement |
| TUI-20 | Visual mode boundaries | Minor UI quirk |
| TUI-23 | Theme caching optimization | Already cached via LazyLock |

### Command (Deferred)
| ID | Description | Reason |
|----|-------------|--------|
| CMD-12 | Multiple LibraryManager instances | By design; each command is independent |
| CMD-15 | Add timeout for editor | Complex, not critical |

### Config (Deferred)
| ID | Description | Reason |
|----|-------------|--------|
| CONFIG-5 | Config files default permissions | OS-level, not critical |
| CONFIG-6 | No integrity checking for sync.toml | Known limitation |
| CONFIG-7 | TOML parsing on every load | Known limitation |
| CONFIG-8 | Missing documentation | Doc task |

### Logging (Deferred)
| ID | Description | Reason |
|----|-------------|--------|
| LOG-8 | Error type limitation | Current design is fine |
| LOG-10 | Per-module log level filter | Requires significant API changes |
| LOG-12 | Async audit log writer | Adds complexity without clear benefit |

### Library (Deferred)
| ID | Description | Reason |
|----|-------------|--------|
| LIB-5 | Confirmation before delete_library | Already implemented in library_cmd |
| LIB-8 | Snippet field aliases | By design for backward compatibility |
| LIB-9 | Empty folders serialization | Correct TOML behavior |
| LIB-10 | TOML regex edge case | Already handled correctly |
| LIB-14 | Document deleted snippets filtered elsewhere | Doc task |

### Sync (Deferred)
| ID | Description | Reason |
|----|-------------|--------|
| SYNC-7 | Missing get_snippets/push_snippets | Uses sync_encrypted instead |
| SYNC-8 | Device ID conflict detection | Rare edge case |
| SYNC-9 | Library identification by filename | Already works |
| SYNC-10 | Sync status reporting clarity | Working correctly |
| SYNC-11 | Backup on merge failure | Already done before merge save |

### Server/Proto (Deferred)
| ID | Description | Reason |
|----|-------------|--------|
| PROTO-3 | Pagination on premade endpoints | Requires proto breaking change |
| PROTO-5 | Request/correlation IDs | Requires proto breaking change |
| PROTO-6 | Batch API key verification | Single lookup is sufficient |
| PROTO-8 | Premade content validation | Not server's responsibility |
| PROTO-9 | Sync skipped snippets persisted | By design |
| PROTO-10 | Register device ID validation | Server generates ID, ignores client |
| PROTO-11 | CORS wildcard in dev | Already handled by config |

### Encryption (Deferred)
| ID | Description | Reason |
|----|-------------|--------|
| ENCRYPT-1 | Documentation outdated | Doc task |
| ENCRYPT-2 | std::mem::take for key cleanup | Already handled by ZeroizeOnDrop |
| ENCRYPT-4 | Constant-time comparison | Not needed for extraction |

### Clipboard (Deferred)
| ID | Description | Reason |
|----|-------------|--------|
| CLIP-5 | Thread leak on rapid scheduling | Generation counter handles this |
| CLIP-6 | Content type preservation | Known limitation |
| CLIP-7 | Clipboard timeout | Not critical |

### Utils (Deferred)
| ID | Description | Reason |
|----|-------------|--------|
| UTILS-1 | Escape sequence inconsistency | Known edge case |
| UTILS-2 | Double-backslash edge case | Known edge case |
| UTILS-3 | expand_command return type | Documentation issue |
| UTILS-4 | Chained backslash edge case | Known edge case |
| UTILS-5 | Nested angle brackets edge case | Known edge case |
| UTILS-6 | Backslash at end of variable content | Known edge case |

### Other (Deferred)
| ID | Description | Reason |
|----|-------------|--------|
| OV-2 | Rate limiter persistence | Not critical |
| OV-3 | Snippet ID collision on merge | By design (last-write-wins) |
| CLI-5 | Multiline double-empty-line | Already implemented |

---

## Summary Statistics

| Category | Count |
|----------|-------|
| WAVE 1 Security | 6 (COMPLETED) |
| WAVE 2 Core Bugs | 17 (COMPLETED) |
| WAVE 3 Improvements | 24 (COMPLETED) |
| WAVE 4 Completed | 35 |
| WAVE 4 Deferred | 50+ (low priority/known limitations) |
| Known Limitations | 3 |
| **Total Implemented** | **82 items** |
