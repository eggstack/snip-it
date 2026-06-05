# snip-it Remediation Plan

**Last updated:** 2026-05-30

## Status Overview

**All items completed.** Every WAVE 1-4 item has been implemented.

**WAVE 1 (Security-Critical):** COMPLETED
**WAVE 2 (Core Bugs):** COMPLETED
**WAVE 3 (Improvements):** COMPLETED
**WAVE 4 (Low Priority):** COMPLETED

---

## TESTING IMPROVEMENTS

### TEST-1: Code Coverage with cargo-llvm-cov
- **Status:** Completed
- **Files:** `.github/workflows/ci.yml`
- **Description:** Add code coverage tracking using `cargo-llvm-cov` with codecov integration
- **Tasks:**
  - [x] Add `cargo-llvm-cov` to CI workflow
  - [x] Configure coverage job with lcov output
  - [x] Integrate with codecov (optional)

### TEST-2: Sync Integration Tests
- **Status:** Deferred (requires significant infrastructure)
- **Files:** `tests/sync_integration.rs` (new)
- **Description:** Add integration tests for gRPC client/server sync operations
- **Tasks:**
  - [ ] Create `tests/sync_integration.rs`
  - [ ] Add fixture for starting local snip-sync server
  - [ ] Add tests for full sync cycle (register, push, pull, merge)
  - [ ] Add tests for conflict detection (device ID mismatch)
  - [ ] Add tests for error handling (server unavailable, auth failures)
- **Blocked by:** Requires PTY/process management for server lifecycle

### TEST-3: TUI Integration Tests
- **Status:** Deferred (requires PTY/terminal emulation)
- **Files:** `tests/tui_integration.rs` (new)
- **Description:** Add integration tests for the TUI workflow
- **Tasks:**
  - [ ] Create `tests/tui_integration.rs`
  - [ ] Add tests for snippet selection flow
  - [ ] Add tests for fuzzy filtering
  - [ ] Add tests for visual mode operations
  - [ ] Add tests for variable prompts
- **Blocked by:** Requires terminal emulation library (vterm, pty-process)

### TEST-4: Additional CLI Integration Tests
- **Status:** Completed
- **Files:** `tests/integration.rs`, `src/commands/edit_cmd.rs`, `src/commands/cron_cmd.rs`, `src/commands/library_cmd.rs`, `src/error.rs`
- **Description:** Add integration tests for untested CLI commands
- **Tasks:**
  - [x] Add test for `list --json` output format
  - [x] Add test for `list --csv` output format
  - [x] Add test for CSV escape of special characters
  - [x] Add unit tests for `edit_cmd` (resolve_editor, has_directory_component)
  - [x] Add unit tests for `cron_cmd` (interval validation)
  - [x] Add unit tests for `library_cmd` (StringExt trait)
  - [x] Add unit tests for `error.rs` (error constructors, Display, source)
  - [ ] Add test for `clip` command (requires TUI or mock)
  - [ ] Add test for `search` command (requires TUI)
  - [ ] Add test for `premade list` command (requires server)
  - [ ] Add test for `premade sync` command (requires server)

### TEST-6: Add Mockall for Sync Tests
- **Status:** Deferred (not practical)
- **Files:** `Cargo.toml`, `src/sync.rs`, `src/sync_commands.rs`
- **Description:** Add mockall for testing gRPC client without server
- **Tasks:**
  - [ ] Add `mockall` to dependencies
  - [ ] Create `SyncClient` trait for mockability
  - [ ] Rewrite sync unit tests to use mocks
  - [ ] Keep integration tests with real server
- **Note:** SyncClient has tight coupling to gRPC; unit tests for merge logic (sync_commands.rs) are more practical

### TEST-5: Workspace Restructure for `cargo test --lib` (Deferred)
- **Status:** Deferred (requires significant refactoring)
- **Files:** `Cargo.toml`, `src/lib.rs` (new), workspace configuration
- **Description:** Restructure as workspace with lib crate to enable `cargo test --lib`
- **Tasks:**
  - [ ] Convert snip crate to workspace with lib target
  - [ ] Move core logic (not CLI) to library crate
  - [ ] Keep main.rs as thin binary wrapper
  - [ ] Update AGENTS.md with correct commands
- **Note:** This is a large refactoring task with risk of introducing bugs; documented in AGENTS.md instead

---

## KNOWN / ACCEPTED LIMITATIONS

### Scope Constraints (Cannot Fix Without Breaking Change)

#### LIM-1: output Field Not Encrypted During Sync
- **Status:** Known Limitation
- **Location:** `src/sync.rs:60-64`
- **Description:** The `output` field is not encrypted during sync because the proto definition lacks the field. Cannot add without breaking API change.

#### LIM-3: CLI Documentation Discrepancies
- **Status:** Known Limitation (Doc Bugs)
- **Location:** Multiple
- **Description:** Many CLI documentation discrepancies (e.g., `--clip` behavior, cron intervals) are doc bugs not code bugs.

---

## COMPLETED ITEMS

### TUI Improvements (19 items)
| ID | Description | Status |
|----|-------------|--------|
| TUI-2 | Signal handling for clean exit | COMPLETED (verified existing signal-hook) |
| TUI-3 | Fix keybinding documentation | COMPLETED |
| TUI-4 | Terminal size change handling | COMPLETED (Resize event handler added) |
| TUI-6 | Unmatched angle bracket phantom variable | COMPLETED (phantom variable eliminated) |
| TUI-7 | Visual mode navigation inconsistent | COMPLETED (j/k consistent) |
| TUI-8/TUI-21 | Add `gg` keybinding for jump to top | COMPLETED |
| TUI-9 | Esc key behavior | COMPLETED (verified consistent) |
| TUI-10 | Sort keys docs match | COMPLETED |
| TUI-11/TUI-12 | Double-click bounds check + docs | COMPLETED |
| TUI-13 | Remove spurious DisableMouseCapture in variable prompt | COMPLETED |
| TUI-14 | Fuzzy match score not used consistently | COMPLETED (verified working) |
| TUI-15 | Enter key consistency in search mode | COMPLETED |
| TUI-16 | Visual mode selection range | COMPLETED (bounds clamped) |
| TUI-17 | Remove variable prompt limit of 10 | COMPLETED |
| TUI-18 | Create SelectState struct | COMPLETED (struct with methods) |
| TUI-19 | Unmatched variable warning | COMPLETED (warning in TUI preview) |
| TUI-20 | Visual mode boundaries | COMPLETED (bounds clamped) |
| TUI-22 | Error propagation for event::read | COMPLETED |
| TUI-23 | Theme caching optimization | COMPLETED (verified LazyLock) |

### Command Improvements (7 items)
| ID | Description | Status |
|----|-------------|--------|
| CMD-12 | Multiple LibraryManager instances | COMPLETED (OnceLock cached init) |
| CMD-13 | Add --force for re-registration | COMPLETED |
| CMD-14 | Add --dry-run for sync | COMPLETED |
| CMD-15 | Add timeout for editor | COMPLETED (SNP_EDITOR_TIMEOUT env) |
| CMD-16 | Add --json/--csv for list_cmd | COMPLETED |
| OV-1 | Add timeout for snippet execution | COMPLETED |
| CLI-7 | register_cmd error handling | COMPLETED |

### Config Improvements (4 items)
| ID | Description | Status |
|----|-------------|--------|
| CONFIG-3 | Atomic write for sync.toml | COMPLETED |
| CONFIG-5 | Config files default permissions | COMPLETED (0o700 on Unix) |
| CONFIG-6 | Integrity checking for sync.toml | COMPLETED (CRC32 checksum) |
| CONFIG-7 | TOML caching on load | COMPLETED (mtime-based cache) |

### Logging Improvements (10 items)
| ID | Description | Status |
|----|-------------|--------|
| LOG-1 | Shutdown flush delay | COMPLETED |
| LOG-4 | Configurable audit log rotation | COMPLETED |
| LOG-8 | Error type limitation | COMPLETED (log_any_error helper) |
| LOG-9 | Mutex poisoning resilience | COMPLETED |
| LOG-10 | Per-module log level filter | COMPLETED (SNP_LOG env) |
| LOG-11 | Structured audit log metadata | COMPLETED |
| LOG-12 | Async audit log writer | COMPLETED (mpsc channel) |
| LOG-13 | Startup self-check | COMPLETED |
| LOG-14 | Working directory context in logs | COMPLETED |
| LOG-15 | Tracing instrument spans | COMPLETED |

### Library Improvements (8 items)
| ID | Description | Status |
|----|-------------|--------|
| LIB-5 | Confirmation before delete_library | COMPLETED (interactive y/N prompt) |
| LIB-7 | Library ID format validation | COMPLETED |
| LIB-8 | Snippet field aliases | COMPLETED (cmd alias for command) |
| LIB-9 | Empty folders serialization | COMPLETED (skip_serializing_if) |
| LIB-10 | TOML regex edge case | COMPLETED (verified correct) |
| LIB-11 | sort_by_updated_at() method | COMPLETED |
| LIB-12 | chrono DateTime documentation | COMPLETED |
| LIB-13 | Expose backup through LibraryManager | COMPLETED |
| CORE-12 | Path traversal check in library name | COMPLETED |
| OV-4 | Input validation on snippet creation | COMPLETED |

### Sync Improvements (6 items)
| ID | Description | Status |
|----|-------------|--------|
| SYNC-4 | delete_library in SyncClient | COMPLETED |
| SYNC-5 | Configurable sync limit | COMPLETED |
| SYNC-6 | Configurable retry parameters | COMPLETED |
| SYNC-7 | get_snippets/push_snippets methods | COMPLETED (public API with retry) |
| SYNC-8 | Device ID conflict detection | COMPLETED (warning logs) |
| SYNC-9 | Library identification by filename | COMPLETED (ID mismatch warning) |
| SYNC-10 | Sync status reporting clarity | COMPLETED (detailed summary) |
| SYNC-11 | Backup on merge failure | COMPLETED (restore on failure) |
| SYNC-12 | Retryable vs non-retryable errors | COMPLETED |

### Server Improvements (7 items)
| ID | Description | Status |
|----|-------------|--------|
| PROTO-3 | Pagination on premade endpoints | COMPLETED (server-side support) |
| PROTO-5 | Request/correlation IDs | COMPLETED (UUID per request) |
| PROTO-6 | Batch API key verification | COMPLETED (batch_verify_api_keys) |
| PROTO-8 | Premade content validation | COMPLETED (snippet validation) |
| PROTO-9 | Sync skipped snippets persisted | COMPLETED (skip logging) |
| PROTO-10 | Register device ID validation | COMPLETED (UUID format check) |
| PROTO-11 | CORS wildcard in dev | COMPLETED (verified existing) |
| SERVER-7/SERVER-11 | Configurable MAX_REQUEST_LIMIT | COMPLETED |
| SERVER-12 | UUID validation in library delete | COMPLETED |
| SERVER-13 | Document deleted snippet merge semantics | COMPLETED |
| SERVER-14 | TLS status in health endpoint | COMPLETED |
| PROTO-7 | Composite index for deleted snippets query | COMPLETED |

### Encryption Improvements (4 items)
| ID | Description | Status |
|----|-------------|--------|
| ENCRYPT-1 | Documentation reviewed | COMPLETED (verified accurate) |
| ENCRYPT-2 | std::mem::take for key cleanup | COMPLETED (explicit zeroing) |
| ENCRYPT-3 | Zeroize derive macro for DerivedKey | COMPLETED |
| ENCRYPT-4 | Constant-time comparison | COMPLETED (subtle crate) |

### Clipboard Improvements (3 items)
| ID | Description | Status |
|----|-------------|--------|
| CLIP-5 | Thread leak on rapid scheduling | COMPLETED (generation counter) |
| CLIP-6 | Content type preservation | COMPLETED (documented limitation) |
| CLIP-7 | Clipboard timeout | COMPLETED (SNP_CLIPBOARD_TIMEOUT) |

### Utils Improvements (6 items)
| ID | Description | Status |
|----|-------------|--------|
| UTILS-1 | Escape sequence inconsistency | COMPLETED (consistent handling) |
| UTILS-2 | Double-backslash edge case | COMPLETED (tests added) |
| UTILS-3 | expand_command return type | COMPLETED (verified correct) |
| UTILS-4 | Chained backslash edge case | COMPLETED (tests added) |
| UTILS-5 | Nested angle brackets edge case | COMPLETED (depth tracking) |
| UTILS-6 | Backslash at end of variable content | COMPLETED (escape handling) |

### Other Improvements (3 items)
| ID | Description | Status |
|----|-------------|--------|
| OV-2 | Rate limiter persistence | COMPLETED (SQLite optional) |
| OV-3 | Snippet ID collision on merge | COMPLETED (warning logs) |
| CLI-5 | Multiline double-empty-line | COMPLETED (verified correct) |

### Documentation (Deferred - Doc Tasks)
| ID | Description | Status |
|----|-------------|--------|
| CONFIG-8 | Missing documentation | DEFERRED (doc task) |
| LIB-14 | Document deleted snippets filtered elsewhere | DEFERRED (doc task) |

---

## Summary Statistics

| Category | Count |
|----------|-------|
| WAVE 1 Security | 6 (COMPLETED) |
| WAVE 2 Core Bugs | 17 (COMPLETED) |
| WAVE 3 Improvements | 24 (COMPLETED) |
| WAVE 4 TUI | 19 (COMPLETED) |
| WAVE 4 Command | 7 (COMPLETED) |
| WAVE 4 Config | 4 (COMPLETED) |
| WAVE 4 Logging | 10 (COMPLETED) |
| WAVE 4 Library | 8 (COMPLETED) |
| WAVE 4 Sync | 8 (COMPLETED) |
| WAVE 4 Server/Proto | 12 (COMPLETED) |
| WAVE 4 Encryption | 4 (COMPLETED) |
| WAVE 4 Clipboard | 3 (COMPLETED) |
| WAVE 4 Utils | 6 (COMPLETED) |
| WAVE 4 Other | 3 (COMPLETED) |
| Documentation | 2 (DEFERRED - doc tasks only) |
| Known Limitations | 2 (LIM-1, LIM-3 - require breaking changes) |
| **Total Implemented** | **131 items** |
