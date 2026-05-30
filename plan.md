# snip-it Consolidated Remediation Plan

**Last updated:** 2026-05-29

## Implementation Waves

The items in each wave can be implemented in parallel by separate agents. Dependencies within a wave are minimal.

### WAVE 1: Security-Critical (6 items - independent)
| ID | Description | Files |
|----|-------------|-------|
| SEC-1 | Output path validation uses string comparison, not canonicalization | `src/commands/run_cmd.rs:10-46` |
| SEC-2 | Editor path resolution respects directory components | `src/commands/edit_cmd.rs:39-130` |
| SEC-3 | TLS server name verification not performed | `src/sync.rs:299-316` |
| SEC-5 | Shell execution uses user-controlled $SHELL | `src/commands/run_cmd.rs:48-50,116-119` |
| SEC-6 | API key masked but still in memory | `src/commands/register_cmd.rs:57-62` |
| CLI-1 | TOCTOU race between output path validation and file creation | `src/commands/run_cmd.rs:92-93` |

**Implementation notes:** SEC-1, SEC-2, SEC-5, SEC-6, CLI-1 are all in `run_cmd.rs` and `edit_cmd.rs` - same agent can fix multiple. SEC-3 is in `sync.rs`.

### WAVE 2: Core Bugs (17 items - independent)
| ID | Description | Files |
|----|-------------|-------|
| CORE-1 | Atomic config saves missing | `src/library.rs` |
| CORE-2 | `deleted` flag not filtered in TUI | `src/ui/mod.rs` |
| CORE-3 | Silent migration on library access | `src/commands/mod.rs:59-63` |
| CLI-3 | TUI exit always triggers sync | `src/commands/mod.rs:252-260` |
| CORE-5 | Empty snippet commands not validated | `src/library.rs` |
| CORE-6 | `save_config` errors silently swallowed | `src/library.rs:456-468` |
| CORE-7 | Snippet ID uniqueness not enforced | `src/library.rs` |
| CORE-8 | LibraryManager doesn't track unsaved changes | `src/library.rs` |
| CORE-9 | `get_library_path` discards errors | `src/commands/mod.rs:56-84` |
| CORE-10 | Primary library selection ignores server origin | `src/library.rs:338-339` |
| CORE-11 | No encryption key validation on load | `src/library.rs` |
| CLIP-1 | Race condition in `schedule_clipboard_clear` | `src/clipboard.rs:30-39` |
| CLIP-2 | Visual mode clipboard copy missing audit log | `src/ui/mod.rs:675` |
| CLIP-3 | UI clipboard operations suppress errors | `src/ui/mod.rs:619,675,817` |
| CONFIG-1 | Keychain migration silent failure | `src/config.rs:188-199` |
| CONFIG-2 | Keychain unavailable returns empty key | `src/config.rs:73` |
| CONFIG-4 | Race condition in macOS migration | `src/utils/config.rs:68-84` |

**Implementation notes:** CORE-* items in `src/library.rs` can be done together. CLI-3, CORE-3, CORE-9 are in `src/commands/mod.rs`. CLIP-* items in UI and clipboard can be done together.

### WAVE 3: Improvements (24 items - some parallelization)
| ID | Description | Files | Sub-wave |
|----|-------------|-------|----------|
| SEC-7 | Add TLS/HTTPS enforcement for production | `snip-sync/src/main.rs` | Security |
| SEC-8 | Make default server URL HTTPS | `src/config.rs:125-129` | Security |
| SEC-9 | Keychain failure should not silently fall back to plaintext | `src/config.rs:48-56` | Security |
| CMD-3 | run_cmd --clip copies command, not output | `src/commands/run_cmd.rs:81-90` | Commands |
| CMD-10 | sync_cmd::run doesn't propagate sync errors | `src/commands/sync_cmd.rs:185-192` | Commands |
| CMD-11 | premade_cmd::run_sync ignores return value | `src/commands/premade_cmd.rs:144-153` | Commands |
| LIB-1 | No sorting on save | `src/library.rs:511-525` | Library |
| LIB-2 | backup_library() not called automatically | `src/library.rs` | Library |
| LIB-3 | Case-sensitive library name duplicates | `src/library.rs` | Library |
| LIB-4 | Add library name case-insensitivity check | `src/library.rs` | Library |
| LIB-6 | Support `name` -> `description` migration | `src/library.rs` | Library |
| LOG-2 | Audit log unbounded growth | `src/logging.rs:217-263` | Logging |
| LOG-3 | Audit log failure is invisible | `src/logging.rs:223-232` | Logging |
| LOG-5 | Add `log_sync_operation` function | `src/logging.rs` | Logging |
| LOG-6 | Audit log contains snippet content | `src/logging.rs:247-254` | Logging |
| LOG-7 | Log directory permissions | `src/logging.rs:55` | Logging |
| SERVER-3 | Missing input validation on `api_key` in Register | `snip-sync/src/main.rs:389` | Server |
| SERVER-4 | Race condition in rate limiter cleanup task | `snip-sync/src/rate_limiter.rs:17-27` | Server |
| SERVER-5 | No limits on `local_snippets` array in Sync | `snip-sync/src/main.rs:580` | Server |
| SERVER-6 | Premade file content not sanitized before serving | `snip-sync/src/premade.rs:208` | Server |
| SERVER-8 | Add batch size limit for Sync local_snippets | `snip-sync/src/main.rs:580` | Server |
| PROTO-1 | Premade filename sanitization too restrictive | `snip-sync/src/main.rs:798-806` | Server |
| PROTO-2 | Rate limiting bypass on Register endpoint | `snip-sync/src/main.rs:360-387` | Server |
| TUI-1 | Visual line mode (`V`) bug | `src/ui/mod.rs:633-638` | UI |

**Implementation notes:** Each sub-wave (Security, Commands, Library, Logging, Server, UI) can be done independently.

### WAVE 4: Low Priority Polish (40+ items - fully parallel)
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
| CMD-12 | Multiple LibraryManager instantiations per command | Throughout commands |
| CMD-13 | Add re-registration support | `src/commands/register_cmd.rs` |
| CMD-14 | Add --dry-run for sync | `src/commands/sync_cmd.rs` |
| CMD-15 | Add timeout for editor | `src/commands/edit_cmd.rs` |
| CMD-16 | Consider --json/--csv for list_cmd | `src/commands/list_cmd.rs` |
| CONFIG-3 | No atomic write for sync.toml | `src/config.rs:166` |
| CONFIG-8 | Missing documentation | Documentation |
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
| LIB-5 | Add confirmation before delete_library() | `src/library.rs` |
| LIB-7 | No validation of library_id format | `src/library.rs` |
| LIB-8 | Snippet fields renamed on deserialization | `src/library.rs` |
| LIB-9 | Empty folders array serializes inconsistently | `src/library.rs` |
| LIB-10 | TOML regex edge case with escaped quotes | `src/utils/toml_helpers.rs:14-15` |
| LIB-11 | Add sort_by_updated_at() method | `src/library.rs` |
| LIB-12 | Consider using chrono DateTime instead of i64 | `src/library.rs` |
| LIB-13 | Expose library backup through LibraryManager | `src/library.rs` |
| LIB-14 | Document that deleted snippets filtered elsewhere | `src/library.rs` |
| SYNC-4 | Missing `delete_library` in SyncClient | `src/sync.rs` |
| SYNC-5 | Hardcoded sync limit | `src/sync.rs` |
| SYNC-6 | Configurable retry parameters | `src/sync.rs` |
| SYNC-7 | Missing get_snippets and push_snippets methods | `src/sync.rs` |
| SYNC-8 | Device ID conflict detection | `src/sync.rs` |
| SYNC-9 | Library identification by filename | `src/sync_commands.rs:187` |
| SYNC-10 | Sync status reporting clarity | `src/sync_commands.rs` |
| SYNC-11 | Backup on merge failure | `src/library.rs` |
| SYNC-12 | Retryable vs non-retryable error classification | `src/sync.rs` |
| OV-1 | Add command timeout for snippet execution | `src/commands/run_cmd.rs:116-134` |
| OV-2 | Rate limiter should support persistence | `snip-sync/src/rate_limiter.rs` |
| OV-3 | Snippet ID collision on merge | `src/sync_commands.rs:394-475` |
| OV-4 | Missing input validation on snippet creation | `src/commands/new_cmd.rs` |

---

### Security Issues

#### SEC-1: Output Path Validation Uses String Comparison, Not Canonicalization
- **Severity:** High (Security)
- **Status:** TODO
- **Location:** `src/commands/run_cmd.rs:10-46`
- **Description:** `validate_output_path` checks for `..` components and absolute paths via string/representation checks, not actual path resolution. A symlink attack could bypass these checks.
- **Fix:** Use `std::fs::canonicalize` to resolve the actual path and verify it stays within the working directory.
- **Dependencies:** None
- **Wave:** 1

#### SEC-2: Editor Path Resolution Respects Directory Components
- **Severity:** High (Security)
- **Status:** TODO
- **Location:** `src/commands/edit_cmd.rs:39-130`
- **Description:** Relative paths with directory components (e.g., `./script.sh`) are resolved against CWD. An attacker could place a malicious `./vim` in a directory and wait for user to edit from there. Also: If user sets `EDITOR` to relative path with directory components, code resolves relative to CWD.
- **Dependencies:** None
- **Wave:** 1

#### SEC-3: TLS Server Name Verification Not Performed
- **Severity:** High (Security)
- **Status:** TODO
- **Location:** `src/sync.rs:299-316`
- **Description:** `create_tls_channel()` does not verify server certificate hostnames. `with_enabled_roots()` without `server_name` verification allows MITM attacks.
- **Fix:** Add `server_name` verification to `ClientTlsConfig`.
- **Dependencies:** None
- **Wave:** 1

#### SEC-4: API Key Transmitted in Plaintext
- **Severity:** High (Security)
- **Status:** Known Limitation
- **Location:** `src/sync.rs`
- **Description:** API keys sent in request bodies over unencrypted connections. Server warns about TLS; production should use reverse proxy with TLS.
- **Note:** This is a documented requirement for production deployment.
- **Dependencies:** TLS configuration
- **Wave:** 1 (documented risk)

#### SEC-5: Shell Execution Uses User-Controlled $SHELL
- **Severity:** Medium (Security)
- **Status:** TODO
- **Location:** `src/commands/run_cmd.rs:48-50,116-119`
- **Description:** `get_shell()` reads from `$SHELL` env var. An attacker with control over the environment could execute arbitrary commands.
- **Dependencies:** None
- **Wave:** 1

#### SEC-6: API Key Masked but Still in Memory
- **Severity:** Low (Security)
- **Status:** TODO
- **Location:** `src/commands/register_cmd.rs:57-62`
- **Description:** API key is masked before printing but full key remains in memory. Someone with access to process memory could recover it.
- **Dependencies:** None
- **Wave:** 1

---

### Critical Bugs

#### CLI-1: TOCTOU Race in Output Path Validation
- **Severity:** High (Security)
- **Status:** TODO
- **Location:** `src/commands/run_cmd.rs:97-100`
- **Description:** Between `validate_output_path()` and `fs::File::create()`, the path could be modified (symlink attack, path traversal via symlink). The validation check and the actual file creation are not atomic.
- **Dependencies:** None
- **Wave:** 1

#### CORE-1: Atomic Config Saves Missing
- **Severity:** High
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** `save_config()` writes directly to `libraries.toml` without temp file or atomic rename. On crash/power failure, config could be corrupted.
- **Dependencies:** None
- **Wave:** 2

#### CORE-2: `deleted` Flag Not Filtered in TUI
- **Severity:** High
- **Status:** TODO
- **Location:** `src/ui/mod.rs`
- **Description:** `select_snippet_inner()` and `get_snippet_data()` don't filter out `deleted: true` snippets. Users may see soft-deleted snippets in TUI.
- **Dependencies:** None
- **Wave:** 2

#### CORE-3: Silent Migration on Library Access
- **Severity:** High
- **Status:** TODO
- **Location:** `src/commands/mod.rs:59-63`
- **Description:** `get_library_path()` and `init_library_manager()` silently migrate from single-file to library mode when user just wants to list libraries or view a snippet. No confirmation prompt, one-way migration.
- **Dependencies:** None
- **Wave:** 2

#### CLI-3: TUI Exit Always Triggers Sync
- **Severity:** High
- **Status:** TODO
- **Location:** `src/commands/mod.rs:252-260`, `src/commands/clip_cmd.rs`, `src/commands/search_cmd.rs`
- **Description:** `run_snippet_selection` ALWAYS calls `run_default_sync(runtime)` when `do_sync` is true, regardless of HOW the user exits (Cancel, Done, or early break). `clip` command passes `sync: true` but never checks sync result.
- **Dependencies:** None
- **Wave:** 2

#### CORE-4: Command Field Not Sanitized
- **Severity:** High (documented risk)
- **Status:** Known Limitation
- **Location:** `src/commands/run_cmd.rs`
- **Description:** Commands are executed via shell without sanitization beyond variable expansion.
- **Dependencies:** None
- **Wave:** 1 (documented risk)

---

## MEDIUM PRIORITY

### Bugs

#### CORE-5: Empty Snippet Commands Not Validated
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** `Snippet::new()` and `create_library()` don't validate that command is non-empty. Empty commands could be saved and cause issues downstream.
- **Dependencies:** None
- **Wave:** 2

#### CORE-6: `save_config` Errors Silently Swallowed
- **Status:** TODO
- **Location:** `src/library.rs:456-468`
- **Description:** `update_library_id()` and `update_last_sync()` swallow errors - they always return `Ok(())` even if `save_config` failed.
- **Dependencies:** None
- **Wave:** 2

#### CORE-7: Snippet ID Uniqueness Not Enforced
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** Multiple snippets can have the same `id` since no uniqueness check in `load_library()` or `save_library()`. Could cause sync issues.
- **Dependencies:** None
- **Wave:** 2

#### CORE-8: LibraryManager Doesn't Track Unsaved Changes
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** If `save_config()` fails, in-memory state is out of sync. No rollback mechanism.
- **Dependencies:** None
- **Wave:** 2

#### CORE-9: `get_library_path` Discards Errors
- **Status:** TODO
- **Location:** `src/commands/mod.rs:56-84`
- **Description:** If `ensure_library_mode()` fails during `get_library_path`, code continues anyway and may return path to wrong file.
- **Dependencies:** None
- **Wave:** 2

#### CORE-10: Primary Library Selection Ignores Server Origin
- **Status:** TODO
- **Location:** `src/library.rs:338-339`
- **Description:** When deleting primary library, code promotes first remaining library without considering if it was synced from server vs local-only.
- **Dependencies:** None
- **Wave:** 2

#### CORE-11: No Encryption Key Validation on Load
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** `load_library()` doesn't validate file structure intact after encryption. If decrypt fails partially, no recovery.
- **Dependencies:** None
- **Wave:** 2

#### CLI-5: Multiline Input Reads Until Double-Empty-Line
- **Status:** TODO
- **Location:** `src/commands/new_cmd.rs:8-25`
- **Description:** `read_multiline_command()` reads until TWO consecutive empty lines. Cannot create snippets with internal empty lines.
- **Dependencies:** None
- **Wave:** 2

#### CLI-6: `sync_cmd::run` Silently Ignores Failures
- **Status:** TODO
- **Location:** `src/commands/sync_cmd.rs:185-192`
- **Description:** `run_sync()` result is not checked or propagated.
- **Dependencies:** None
- **Wave:** 3

#### CLI-7: `register_cmd` Inconsistent Error Handling
- **Status:** TODO
- **Location:** `src/commands/register_cmd.rs:51-54`
- **Description:** If `save_sync_settings` fails after successful registration, user has valid API key/device_id that won't be saved.
- **Dependencies:** None
- **Wave:** 2

#### CLI-8: `_config` Parameter Unused in TUI Commands
- **Status:** Documented Limitation
- **Location:** `src/commands/run_cmd.rs:141`
- **Description:** `_config` parameter accepted but never used. Commands using `run_snippet_selection` cannot override config path.
- **Dependencies:** None
- **Wave:** 4

#### CLI-9: API Key in Memory After Registration
- **Status:** Known Limitation
- **Location:** `src/commands/register_cmd.rs:57-62`
- **Description:** API key printed to stdout (masked) but stored in `SyncSettings` in memory. No attempt to clear after use.
- **Dependencies:** None
- **Wave:** 2

#### CLIP-1: Race Condition in `schedule_clipboard_clear`
- **Status:** TODO
- **Location:** `src/clipboard.rs:30-39`
- **Description:** Generation counter logic is fragile - if second call's thread spawns after first thread reads but before it checks, both may attempt clear. Comparison `gen == CLIPBOARD_GENERATION.load()` not atomic with spawn.
- **Dependencies:** None
- **Wave:** 2

#### CLIP-2: Visual Mode Clipboard Copy Missing Audit Log
- **Status:** TODO
- **Location:** `src/ui/mod.rs:675`
- **Description:** When copying multiple snippets via visual mode (`V` then `y`), does NOT call `audit_log("copy", snippet)`. Only logged at debug level.
- **Dependencies:** None
- **Wave:** 2

#### CLIP-3: UI Clipboard Operations Suppress Errors
- **Status:** TODO
- **Location:** `src/ui/mod.rs:619, 675, 817`
- **Description:** All three UI clipboard operations use `let _ = clipboard::copy_to_clipboard_auto()`. Failures are completely silent.
- **Dependencies:** None
- **Wave:** 2

#### CLIP-4: Visual Mode Copy Copies Description Not Command
- **Status:** **FIXED** (verified per AGENTS.md)
- **Location:** `src/ui/mod.rs:672`
- **Description:** Visual mode `y` copies descriptions, not commands, inconsistent with single-select behavior.
- **Fix Applied:** Line 672 now uses `commands[*idx]` (not `descriptions[*idx]`). The `y` key copies the actual command to clipboard.
- **Dependencies:** None
- **Wave:** 2 (completed)

#### CONFIG-1: Keychain Migration Silent Failure
- **Status:** TODO
- **Location:** `src/config.rs:188-199`
- **Description:** When migrating plaintext API key to keychain, failures are logged but not propagated. API key may remain in plaintext if keychain unavailable.
- **Dependencies:** None
- **Wave:** 2

#### CONFIG-2: Keychain Unavailable Returns Empty Key
- **Status:** TODO
- **Location:** `src/config.rs:73`
- **Description:** When keychain unavailable and `KEYCHAIN_MARKER` encountered, deserialization returns `Ok(String::new())`. User sees no indication API key failed to load.
- **Dependencies:** None
- **Wave:** 2

#### CONFIG-3: No Atomic Write for sync.toml
- **Status:** TODO
- **Location:** `src/config.rs:166`
- **Description:** `save_sync_settings()` writes directly without atomic rename. Process crash mid-write could corrupt file.
- **Dependencies:** None
- **Wave:** 4

#### CONFIG-4: Race Condition in macOS Migration
- **Status:** TODO
- **Location:** `src/utils/config.rs:68-84`
- **Description:** Multiple processes could race during migration. `dst.exists()` check is TOCTOU.
- **Dependencies:** None
- **Wave:** 2

#### SERVER-3: Missing Input Validation on `api_key` Field in Register
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs:390-392`
- **Description:** `register` RPC completely ignores the `api_key` field from `RegisterRequest`. A new API key is generated with `uuid::Uuid::new_v4()` regardless of what was passed. No length/format validation.
- **Dependencies:** None
- **Wave:** 2

#### SERVER-4: Race Condition in Rate Limiter Cleanup Task
- **Status:** TODO
- **Location:** `snip-sync/src/rate_limiter.rs:17-27`
- **Description:** Cleanup task holds lock across `await` point. If task panics, lock is poisoned.
- **Fix:** Use a separate channel to trigger shutdown.
- **Dependencies:** None
- **Wave:** 2

#### SERVER-5: No Limits on `local_snippets` Array in Sync
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs:580`
- **Description:** `SyncRequest.local_snippets` has no size limit. Client could send millions of snippets causing memory exhaustion.
- **Dependencies:** None
- **Wave:** 2

#### SERVER-6: Premade File Content Not Sanitized Before Serving
- **Status:** TODO
- **Location:** `snip-sync/src/premade.rs:199`
- **Description:** `get()` returns raw file content via `fs::read_to_string(&canonical_path)` without running `fix_invalid_toml_escapes()`.
- **Dependencies:** None
- **Wave:** 2

#### SERVER-7: Make MAX_REQUEST_LIMIT Configurable
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs:35`
- **Description:** `MAX_REQUEST_LIMIT = 1000` is hardcoded and not documented.
- **Dependencies:** None
- **Wave:** 4

#### SERVER-8: Add Batch Size Limit for Sync local_snippets
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs:580`
- **Description:** Validate `req.local_snippets.len()` against reasonable limit.
- **Dependencies:** None
- **Wave:** 2

#### SERVER-9: Graceful Shutdown for Rate Limiter Cleanup Task
- **Status:** TODO
- **Location:** `snip-sync/src/rate_limiter.rs`
- **Description:** Use shutdown signal channel instead of running forever.
- **Dependencies:** None
- **Wave:** 4

#### PROTO-1: Premade Library Filename Sanitization Too Restrictive
- **Status:** **FIXED**
- **Location:** `snip-sync/src/main.rs:798-806`
- **Description:** Filters out dots (`.`), which are valid in filenames. `devops.tools` becomes `devopstools`.
- **Fix:** Allow dots, add path traversal protection instead (checking for `..`, `/`, `\`).
- **Dependencies:** None
- **Wave:** 3

#### PROTO-2: Rate Limiting Bypass on Register Endpoint
- **Status:** **FIXED**
- **Location:** `snip-sync/src/main.rs:360-387`
- **Description:** `register` uses `x-forwarded-for` header for rate limiting without validation. Clients can spoof.
- **Fix:** Only trust x-forwarded-for from trusted proxies (configurable via `TRUSTED_PROXIES` env var or `rate_limit.trusted_proxies` config).
- **Dependencies:** None
- **Wave:** 3

#### PROTO-3: Missing Pagination on Premade Library Endpoints
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs:755-782`
- **Description:** `ListPremadeLibraries` returns all libraries with no pagination.
- **Dependencies:** None
- **Wave:** 3

#### PROTO-10: Register Endpoint Device ID Not Validated
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs:389`
- **Description:** `RegisterRequest.device_id` is ignored entirely.
- **Dependencies:** None
- **Wave:** 3

#### PROTO-11: CORS Wildcard in Development
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs:912-914`
- **Description:** `CORS_ALLOW_ALL=true` allows any origin.
- **Dependencies:** None
- **Wave:** 4

#### LIB-1: No Sorting on Save
- **Status:** **FIXED**
- **Location:** `src/library.rs:524-547`
- **Description:** `save_library()` now sorts snippets by `updated_at` descending before saving.
- **Dependencies:** None
- **Wave:** 3
- **Fix Applied:** `sorted.snippets.sort_by_key(|b| std::cmp::Reverse(b.updated_at));`

#### LIB-2: backup_library() Not Called Automatically
- **Status:** **FIXED**
- **Location:** `src/library.rs`
- **Description:** `backup_library()` is now invoked automatically before saving.
- **Dependencies:** None
- **Wave:** 3
- **Fix Applied:** `backup_library(path)` called at start of `save_library()`.

#### LIB-3: Case-Sensitive Library Name Duplicates
- **Status:** **FIXED**
- **Location:** `src/library.rs`
- **Description:** `create_library("MyLib")` then `create_library("mylib")` now returns error on case-insensitive filesystems.
- **Dependencies:** None
- **Wave:** 3
- **Fix Applied:** Added case-insensitive duplicate check in `create_library()`.

#### LIB-4: Add Library Name Case-Insensitivity Check
- **Status:** **FIXED** (combined with LIB-3)
- **Location:** `src/library.rs`
- **Description:** Check for case-insensitive duplicates on case-insensitive filesystems.
- **Dependencies:** None
- **Wave:** 3
- **Fix Applied:** Same as LIB-3.

#### LIB-5: Add Confirmation Before delete_library()
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** `delete_library()` immediately removes file without confirmation.
- **Dependencies:** None
- **Wave:** 4

#### LIB-6: Support `name` -> `description` Migration
- **Status:** **FIXED**
- **Location:** `src/library.rs`
- **Description:** Legacy data with `name` field now deserializes correctly via alias.
- **Dependencies:** None
- **Wave:** 3
- **Fix Applied:** Added `#[serde(alias = "name")]` to `description` field.

#### LOG-1: shutdown_logging May Lose Buffered Logs
- **Status:** TODO
- **Location:** `src/logging.rs:101-106`
- **Description:** `shutdown_logging` never called on signal termination. Buffered logs lost.
- **Dependencies:** None
- **Wave:** 2

#### LOG-2: Audit Log Unbounded Growth
- **Status:** **DONE**
- **Location:** `src/logging.rs:289-325`
- **Description:** `audit.log` grows indefinitely. No rotation or retention policy.
- **Fix:** Added `rotate_audit_log_if_needed()` function that rotates log when it exceeds 10MB and deletes rotated files older than 30 days.
- **Dependencies:** None
- **Wave:** 3

#### LOG-3: Audit Log Failure Is Invisible
- **Status:** **DONE**
- **Location:** `src/logging.rs:238-278`
- **Description:** Errors silently swallowed and logged at debug level only.
- **Fix:** Audit log errors now logged at `warn` level for path failures and `error` level for write failures.
- **Dependencies:** None
- **Wave:** 3

#### LOG-4: Add Rotation Policy Configuration
- **Status:** TODO
- **Location:** `src/logging.rs`
- **Description:** Allow configuring max log file size or retention days, not just daily rotation.
- **Dependencies:** None
- **Wave:** 4

#### LOG-5: Add `log_sync_operation` Function
- **Status:** **DONE**
- **Location:** `src/logging.rs:327-374`
- **Description:** No equivalent for sync operations (connect, merge, conflict resolution).
- **Fix:** Added `SyncOperationType` enum and `log_sync_operation()` function that logs sync operations with appropriate levels.
- **Dependencies:** None
- **Wave:** 3

#### LOG-6: Audit Log Contains Snippet Content
- **Status:** **DONE**
- **Location:** `src/logging.rs:255-261`
- **Description:** Audit log records description, command, and output. Sensitive data written to plain text.
- **Fix:** Removed command and output from audit log. Now only logs snippet ID, description, and operation type.
- **Dependencies:** None
- **Wave:** 3

#### LOG-7: Log Directory Permissions
- **Status:** **DONE**
- **Location:** `src/logging.rs:61-66`
- **Description:** Default umask creates directories with 755 permissions. Others can read logs.
- **Fix:** Set explicit permissions (700) for log directory on Unix.
- **Dependencies:** None
- **Wave:** 3
- **Dependencies:** None
- **Wave:** 3

#### LOG-8: log_config_operation Error Type Limitation
- **Status:** TODO
- **Location:** `src/logging.rs:172`
- **Description:** Function signature uses `&str` error type which only accepts `'static` lifetimes.
- **Dependencies:** None
- **Wave:** 4

#### LOG-9: LOG_GUARD Mutex Poisoning
- **Status:** TODO
- **Location:** `src/logging.rs:26-27,102,86`
- **Description:** `lock().unwrap()` will panic if mutex is poisoned.
- **Dependencies:** None
- **Wave:** 4

#### SYNC-4: Missing `delete_library` in SyncClient
- **Status:** TODO
- **Location:** `src/sync.rs`
- **Description:** Documented `delete_library()` method not implemented in `SyncClient`.
- **Dependencies:** None
- **Wave:** 3

#### SYNC-5: Hardcoded Sync Limit
- **Status:** TODO
- **Location:** `src/sync.rs`
- **Description:** `sync_encrypted()` hardcodes `limit: 1000`. Consider making configurable.
- **Dependencies:** None
- **Wave:** 4

#### OV-1: Add Command Timeout for Snippet Execution
- **Status:** TODO
- **Location:** `src/commands/run_cmd.rs:116-134`
- **Description:** Shell commands have no timeout. Long-running commands could block indefinitely.
- **Dependencies:** None
- **Wave:** 3

---

### Improvements

#### SEC-7: Add TLS/HTTPS Enforcement for Production
- **Status:** **FIXED**
- **Location:** `snip-sync/src/main.rs:829-831`
- **Severity:** High
- **Description:** Server warns "TLS is not enabled" but doesn't enforce. Client should also validate TLS certificates.
- **Fix:** Server now requires `TLS_ENABLED=true` or fails to start. Set `SNIP_SYNC_ALLOW_HTTP=true` to allow plaintext for development.
- **Dependencies:** None
- **Wave:** 3

#### SEC-8: Make Default Server URL HTTPS
- **Status:** **FIXED**
- **Location:** `src/config.rs:125-129`
- **Severity:** Medium
- **Description:** Default `http://localhost:50051` is dangerous. Should default to HTTPS.
- **Fix:** Changed default URL from `http://localhost:50051` to `https://localhost:50051`.
- **Dependencies:** None
- **Wave:** 3

#### SEC-9: Keychain Failure Should Not Silently Fall Back to Plaintext
- **Status:** **FIXED**
- **Location:** `src/config.rs:48-56`
- **Severity:** Medium
- **Description:** When keychain storage fails, API key is stored in plaintext. Should fail explicitly or require confirmation.
- **Fix:** Refuses to store plaintext API key unless `SNP_ALLOW_PLAINTEXT_API_KEY=true` is explicitly set.
- **Dependencies:** None
- **Wave:** 3

#### CMD-3: run_cmd --clip Copies Command, Not Output
- **Status:** Done
- **Location:** `src/commands/run_cmd.rs:81-90`
- **Description:** `--clip` copies the expanded command, not the output. User expectation mismatch.
- **Dependencies:** None
- **Wave:** 3

#### CMD-10: sync_cmd::run Doesn't Propagate Sync Errors
- **Status:** Done
- **Location:** `src/commands/sync_cmd.rs:185-192`
- **Description:** `run_sync()` errors silently ignored. Always returns `Ok(())`.
- **Dependencies:** None
- **Wave:** 3

#### CMD-11: premade_cmd::run_sync Ignores Return Value
- **Status:** Done
- **Location:** `src/commands/premade_cmd.rs:144-153`
- **Description:** `run_sync()` always returns `Ok(())` regardless of actual sync result.
- **Dependencies:** None
- **Wave:** 3

#### TUI-1: Visual Line Mode (`V`) Bug
- **Status:** **FIXED**
- **Location:** `src/ui/mod.rs:633-638`
- **Description:** When pressing `V`, `visual_end` is set but `selected` stays at current position. Confusing visual state.
- **Fix:** Set `selected = visual_end` when `V` is pressed.
- **Dependencies:** None
- **Wave:** 3

#### UTILS-1: Escape Sequence Handling Inconsistency
- **Status:** TODO
- **Location:** `src/utils/variables.rs`
- **Description:** `extract_variable_tokens` and `expand_command` handle `\<` differently:
  - `expand_command`: `\<` → `<`
  - `extract_variable_tokens`: drops the `<` entirely
- **Impact:** `extract_variable_tokens("<host> and \<website>")` returns only `["host"]`. When expanded, output is `"host and <website>"` — but `<website>` was silently dropped from token extraction.
- **Dependencies:** None
- **Wave:** 2

#### UTILS-2: Double-Backslash Before Angle Bracket Loses Character
- **Status:** TODO
- **Location:** `src/utils/variables.rs:111`
- **Description:** Input `\\<foo>` (literal backslash then variable) produces `\foo>` instead of expected `\<foo>`.
- **Dependencies:** None (high priority fix recommended)
- **Wave:** 3

#### ENCRYPT-1: Documentation Outdated (Memory Cost)
- **Status:** TODO (doc update only)
- **Location:** Documentation only
- **Description:** Architecture doc shows `ARGON2_MEMORY_COST_KIB: 1 << 6` (64 KiB) but actual is `1 << 14` (16 MiB) - OWASP minimum. Documentation is outdated (code is improved).
- **Dependencies:** None
- **Wave:** 4

#### SERVER-10: Add Health Check for Database Connectivity
- **Status:** **FIXED** (verified per AGENTS.md)
- **Location:** `snip-sync/src/main.rs:343-352`
- **Description:** Health RPC now verifies database connectivity via `db.ping()`.
- **Fix Applied:** Done.

#### PROTO-4: Health Check Returns Hardcoded `healthy: true`
- **Status:** **FIXED** (verified per AGENTS.md)
- **Location:** `snip-sync/src/main.rs:343-352`
- **Description:** Health check always returns true without checking dependencies.
- **Fix Applied:** Done.

---

## LOW PRIORITY

### Bugs

#### TUI-6: Unmatched Angle Bracket Creates Phantom Variable
- **Status:** Known Limitation (Documented)
- **Location:** `src/utils/highlight.rs:39-49`, `src/ui/mod.rs`
- **Description:** When parsing `<text` without closing `>`, parser silently drops `<` and treats everything after as literal. No warning shown.
- **Note:** Documented in AGENTS.md as known edge case.
- **Wave:** 4

#### TUI-7: Visual Mode Navigation Inconsistent
- **Status:** TODO
- **Location:** `src/ui/mod.rs:831,838`
- **Description:** `h`/`l` arrow keys move selection in ALL modes, not just visual mode.
- **Dependencies:** TUI-3
- **Wave:** 4

#### TUI-8: `gg` Keybinding Missing
- **Status:** TODO
- **Location:** `src/ui/mod.rs:827`
- **Description:** Documented `gg` (vim-style jump to top) is actually `Ctrl+g`.
- **Dependencies:** TUI-3
- **Wave:** 4

#### TUI-9: Esc Key Inconsistent Behavior
- **Status:** TODO
- **Location:** `src/ui/mod.rs:867-869`
- **Description:** In normal mode, `Esc` does nothing. Comment says "Esc no longer quits - use q instead".
- **Dependencies:** TUI-3
- **Wave:** 4

#### TUI-10: Sort Keys Don't Match Documentation
- **Status:** TODO
- **Location:** `src/ui/mod.rs:843-846`
- **Description:** `n` sorts Newest not Name, `o` sorts Oldest not Date, `a` sorts AlphaAsc not Usage.
- **Dependencies:** TUI-3
- **Wave:** 4

#### TUI-11: Double-Click Execution Not Documented
- **Status:** TODO
- **Location:** `src/ui/mod.rs:591-593`
- **Description:** Double-click executes snippet but not mentioned in keybindings documentation.
- **Dependencies:** None
- **Wave:** 4

#### TUI-12: Double-Click Detection Logic Flaw
- **Status:** TODO
- **Location:** `src/ui/mod.rs:581-604`
- **Description:** If user clicks Row A, then quickly clicks Row B, `last_click_row` and `last_click_time` are NOT reset properly.
- **Dependencies:** None
- **Wave:** 4

#### TUI-13: Spurious DisableMouseCapture in Variable Prompt
- **Status:** TODO
- **Location:** `src/ui/variables.rs:140-143`
- **Description:** Mouse capture disabled on quit but was never enabled in `prompt_variables_inner()`.
- **Dependencies:** None
- **Wave:** 4

#### TUI-14: Fuzzy Match Score Not Used Consistently
- **Status:** TODO
- **Location:** `src/ui/mod.rs:283-311`
- **Description:** When explicit sort modes active, fuzzy scores discarded. `n` (newest) might rank poor match above perfect match.
- **Dependencies:** None
- **Wave:** 4

#### TUI-15: Enter Key Behavior Inconsistency in Search Mode
- **Status:** TODO
- **Location:** `src/ui/mod.rs:742-746`
- **Description:** `Enter` does nothing in search mode but selects in non-search mode. Undocumented.
- **Dependencies:** None
- **Wave:** 4

#### TUI-16: Visual Mode Selection Range Includes Visual Start
- **Status:** TODO
- **Location:** `src/ui/mod.rs:646-686`
- **Description:** Vim yanks exclude start position, current impl includes `end - start + 1` items.
- **Dependencies:** None
- **Wave:** 4

#### TUI-17: Variable Prompt Limit of 10
- **Status:** TODO
- **Location:** `src/ui/variables.rs:65`
- **Description:** Only 10 variables displayed at once. 11+ variable snippets have hidden variables.
- **Dependencies:** None
- **Wave:** 4

#### UTILS-3: expand_command Return Type Mismatch
- **Status:** Documentation Issue
- **Location:** `src/utils/variables.rs`
- **Description:** Documented as `SnipResult<String>` (can error on missing vars). Actual returns `String`, falls back to variable name.
- **Note:** This is reasonable behavior - documentation is wrong.
- **Dependencies:** None
- **Wave:** 4

#### UTILS-4: Chained Backslash Escape Edge Case
- **Status:** TODO
- **Location:** `src/utils/variables.rs`
- **Description:** `\\\` — three backslashes loses one backslash.
- **Dependencies:** None
- **Wave:** 4

#### UTILS-5: Nested Angle Brackets Edge Case
- **Status:** TODO
- **Location:** `src/utils/variables.rs`
- **Description:** `echo <<foo>` — second `<` silently dropped, output is `echo foo`.
- **Dependencies:** None
- **Wave:** 4

#### UTILS-6: Backslash at End of Variable Content
- **Status:** TODO
- **Location:** `src/utils/variables.rs`
- **Description:** `<foo\>` — backslash is lost, output is `<foo>` (var named foo).
- **Dependencies:** None
- **Wave:** 4

#### SERVER-11: Hardcoded MAX_REQUEST_LIMIT Magic Number
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs:35`
- **Description:** `MAX_REQUEST_LIMIT = 1000` not documented, not configurable.
- **Dependencies:** SERVER-7
- **Wave:** 4

#### LIB-7: No Validation of library_id Format
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** `update_library_id()` accepts any string without UUID validation.
- **Dependencies:** None
- **Wave:** 4

#### LIB-8: Snippet Fields Renamed on Deserialization
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** `#[serde(rename = ...)]` means `Description` and `description` both accepted, but no `name` alias for `description`.
- **Dependencies:** LIB-6
- **Wave:** 4

#### LIB-9: Empty Folders Array Serializes Inconsistently
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** Empty `Vec<String>` may serialize as `folders = []` or be omitted.
- **Dependencies:** None
- **Wave:** 4

#### LIB-10: TOML Regex Edge Case with Escaped Quotes
- **Status:** TODO
- **Location:** `src/utils/toml_helpers.rs:14-15`
- **Description:** Regex match boundaries may be incorrect for strings with `\"`.
- **Dependencies:** None
- **Wave:** 4

#### CORE-12: Library Name Validation Missing Path Traversal Check
- **Status:** TODO
- **Location:** `src/library.rs:98-121`
- **Description:** `validate_library_name()` checks `/`, `\`, null bytes but doesn't explicitly check `..` or path traversal patterns.
- **Dependencies:** None
- **Wave:** 4

#### CLIP-5: Thread Leak on Rapid Scheduling
- **Status:** TODO
- **Location:** `src/clipboard.rs`
- **Description:** If `schedule_clipboard_clear(1)` called 1000 times rapidly, 1000 threads spawned.
- **Dependencies:** None
- **Wave:** 4

#### CLIP-6: No Clipboard Content Type Preservation
- **Status:** Known Limitation
- **Location:** `src/clipboard.rs`
- **Description:** Only handles text. Does not support images, files, or rich content.
- **Dependencies:** None
- **Wave:** 4

#### CLIP-7: Clipboard Operations Have No Timeout
- **Status:** TODO
- **Location:** `src/clipboard.rs`
- **Description:** `Clipboard::new()`, `set_text()` on Windows and `ClipboardContext::new()`, `set_contents()` on Unix have no timeout.
- **Dependencies:** None
- **Wave:** 4

#### CONFIG-5: Config Files Use Default Permissions
- **Status:** Known Limitation
- **Location:** `src/config.rs`, `src/utils/config.rs`
- **Description:** Config files created with default umask. `sync.toml` with API key could be readable by others on multi-user systems.
- **Dependencies:** None
- **Wave:** 4

#### CONFIG-6: No Integrity Checking for sync.toml
- **Status:** Known Limitation
- **Location:** `src/config.rs`
- **Description:** `sync.toml` has no checksum or signature. Tampering is undetected.
- **Dependencies:** None
- **Wave:** 4

#### CONFIG-7: TOML Parsing on Every Load
- **Status:** Known Limitation
- **Location:** `src/config.rs`
- **Description:** `load_sync_settings()` parses TOML every time.
- **Dependencies:** None
- **Wave:** 4

#### CONFIG-8: Missing Documentation
- **Status:** TODO
- **Location:** Documentation
- **Description:** `device_id` field has no documentation on generation/rotation; `clipboard_auto_clear_seconds` behavior when `None` not documented; keychain integration not in architecture.
- **Dependencies:** None
- **Wave:** 4

#### ENCRYPT-2: Could Use `std::mem::take` for Explicit Key Cleanup
- **Status:** TODO
- **Location:** `src/encryption.rs`
- **Description:** Instead of ineffective `drop(key)`, could use `std::mem::take` for guaranteed cleanup before payload construction.
- **Dependencies:** None
- **Wave:** 4

#### ENCRYPT-3: Missing ZeroizeDerive for DerivedKey
- **Status:** TODO
- **Location:** `src/encryption.rs`
- **Description:** Could use `zeroize::ZeroizeFrom` or `ZeroizeDefault` for cleaner implementation.
- **Dependencies:** None
- **Wave:** 4

#### ENCRYPT-4: Constant-Time Comparison for Salt/Nonce Extraction
- **Status:** Known Limitation
- **Location:** `src/encryption.rs`
- **Description:** `from_base64` uses slice operations that could theoretically leak timing info.
- **Dependencies:** None
- **Wave:** 4

---

### Improvements

#### CMD-12: Multiple LibraryManager Instantiations Per Command
- **Status:** TODO
- **Location:** Throughout commands
- **Description:** `get_library_path()` creates `LibraryManager`, then `run_snippet_selection()` creates another via `load_library()`. Each reads from disk.
- **Dependencies:** None
- **Wave:** 4

#### CMD-13: Add Re-registration Support
- **Status:** TODO
- **Location:** `src/commands/register_cmd.rs`
- **Description:** Cannot re-register without manually editing config file.
- **Dependencies:** None
- **Wave:** 4

#### CMD-14: Add --dry-run for Sync
- **Status:** TODO
- **Location:** `src/commands/sync_cmd.rs`
- **Description:** No dry-run mode to preview sync changes.
- **Dependencies:** None
- **Wave:** 4

#### CMD-15: Add Timeout for Editor
- **Status:** TODO
- **Location:** `src/commands/edit_cmd.rs`
- **Description:** Editor execution has no timeout.
- **Dependencies:** None
- **Wave:** 4

#### CMD-16: Consider --json/--csv for list_cmd
- **Status:** TODO
- **Location:** `src/commands/list_cmd.rs`
- **Description:** No machine-readable output formats for scripting.
- **Dependencies:** None
- **Wave:** 4

#### TUI-2: Add Signal Handling for Clean Exit
- **Status:** TODO
- **Location:** `src/ui/mod.rs`
- **Description:** Ctrl+C during TUI may leave terminal in raw mode. `TERMINATE` static exists but is never checked.
- **Dependencies:** None
- **Wave:** 4

#### TUI-3: Fix Keybinding Documentation or Code
- **Status:** TODO
- **Location:** `src/ui/mod.rs:831,838,843-846`
- **Description:** Multiple keybinding discrepancies:
- `h`/`l` work in all modes, not just visual
- `gg` is `Ctrl+g`
- `n` sorts Newest not Name
- `o` sorts Oldest not Date
- `Esc` does nothing (not quit)
- **Dependencies:** None
- **Wave:** 4

#### TUI-4: Terminal Size Change Handling
- **Status:** TODO
- **Location:** `src/ui/mod.rs`
- **Description:** Code checks minimum terminal size but doesn't handle resize events gracefully.
- **Dependencies:** None
- **Wave:** 4

#### TUI-18: Create SelectState Struct
- **Status:** TODO
- **Location:** `src/ui/mod.rs`
- **Description:** State scattered across local variables. Consider `SelectState` struct for better organization.
- **Dependencies:** None
- **Wave:** 4

#### TUI-19: Unmatched Variable Warning
- **Status:** TODO
- **Location:** `src/ui/mod.rs`
- **Description:** Show warning in preview when snippet contains unmatched `<`.
- **Dependencies:** None
- **Wave:** 4

#### TUI-20: Visual Mode Boundaries
- **Status:** TODO
- **Location:** `src/ui/mod.rs`
- **Description:** In visual mode, `h`/`l` should respect `visual_start`/`visual_end` boundaries.
- **Dependencies:** TUI-3
- **Wave:** 4

#### TUI-21: Add `gg` as Alternative to `Ctrl+g`
- **Status:** TODO
- **Location:** `src/ui/mod.rs`
- **Description:** Many vim users expect `gg` to work.
- **Dependencies:** TUI-3
- **Wave:** 4

#### TUI-22: Error Propagation for event::poll and event::read
- **Status:** TODO
- **Location:** `src/ui/mod.rs:559`
- **Description:** IO errors silently ignored. Consider logging warnings.
- **Dependencies:** None
- **Wave:** 4

#### TUI-23: Theme Caching Optimization
- **Status:** TODO
- **Location:** `src/ui/theme.rs:64-66`
- **Description:** `get_theme()` called multiple times per frame, each locks mutex. Consider returning `&'static Theme`.
- **Dependencies:** None
- **Wave:** 4

#### SYNC-6: Configurable Retry Parameters
- **Status:** TODO
- **Location:** `src/sync.rs`
- **Description:** `MAX_RETRIES`, `INITIAL_DELAY_MS`, `MAX_DELAY_MS` are hardcoded.
- **Dependencies:** None
- **Wave:** 4

#### SYNC-7: Missing get_snippets and push_snippets Client Methods
- **Status:** TODO
- **Location:** `src/sync.rs`
- **Description:** Proto defines these but SyncClient doesn't expose them.
- **Dependencies:** None
- **Wave:** 3

#### SYNC-8: Device ID Conflict Detection
- **Status:** TODO
- **Location:** `src/sync.rs`
- **Description:** No validation that `device_id` is unique per user/library.
- **Dependencies:** None
- **Wave:** 4

#### SYNC-9: Library Identification by Filename
- **Status:** TODO
- **Location:** `src/sync_commands.rs:187`
- **Description:** Libraries matched by filename. Rename breaks sync. Consider stable library ID.
- **Dependencies:** None
- **Wave:** 3

#### SYNC-10: Sync Status Reporting Clarity
- **Status:** TODO
- **Location:** `src/sync_commands.rs`
- **Description:** When sync succeeds with skipped snippets, result message not prominent.
- **Dependencies:** None
- **Wave:** 4

#### SYNC-11: Backup on Merge Failure
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** `backup_library()` error silently ignored on merge failure.
- **Dependencies:** None
- **Wave:** 3

#### SYNC-12: Retryable vs Non-Retryable Error Classification
- **Status:** TODO
- **Location:** `src/sync.rs`
- **Description:** All gRPC errors trigger same retry behavior. Distinguish for better UX.
- **Dependencies:** None
- **Wave:** 4

#### SERVER-12: Add SQL Injection Defense in Library Delete
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs`
- **Description:** Delete check for `req.library_id == "default"` is confusingly redundant with UUID-based check.
- **Dependencies:** None
- **Wave:** 4

#### SERVER-13: Document deleted Snippet Merge Semantics
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs`, `src/sync_commands.rs`
- **Description:** Architecture should explicitly note that `deleted: true` snippets signal destruction, not tombstoning.
- **Dependencies:** None
- **Wave:** 3

#### SERVER-14: Consider TLS Warning in Health/Ready Endpoint
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs`
- **Description:** TLS warning only in startup log. Production may miss.
- **Dependencies:** None
- **Wave:** 4

#### PROTO-5: Add Request ID / Correlation ID
- **Status:** TODO
- **Location:** `proto/sync.proto`
- **Description:** No request tracing ID across operations.
- **Dependencies:** None
- **Wave:** 4

#### PROTO-6: Batch API Key Verification
- **Status:** TODO
- **Location:** `snip-sync/src/db.rs:216-234`
- **Description:** `get_user_by_api_key` fetches all users with matching prefix, then iterates. O(n) per auth.
- **Dependencies:** None
- **Wave:** 3

#### PROTO-7: Missing Index for Deleted Snippets Query
- **Status:** TODO
- **Location:** `snip-sync/src/db.rs:372-437`
- **Description:** No compound index on `(user_id, library_id, deleted, updated_at)`.
- **Dependencies:** None
- **Wave:** 3

#### PROTO-8: Premade Library Content Not Validated
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs:784-821`
- **Description:** Premade content returned as raw string with no size limit or content validation.
- **Dependencies:** None
- **Wave:** 3

#### PROTO-9: Sync Skipped Snippets Not Persisted
- **Status:** TODO
- **Location:** `snip-sync/src/main.rs:578-608`
- **Description:** Validation failures added to `skipped_ids` but client never receives feedback.
- **Dependencies:** None
- **Wave:** 3

#### LOG-10: Add Log Level Filter Per Module
- **Status:** TODO
- **Location:** `src/logging.rs`
- **Description:** Could allow `SNP_LOG_SYNC=debug` style filtering.
- **Dependencies:** None
- **Wave:** 4

#### LOG-11: Add Structured Metadata to Audit Log
- **Status:** TODO
- **Location:** `src/logging.rs`
- **Description:** Could include ISO 8601 timestamp, user, library name, execution duration.
- **Dependencies:** None
- **Wave:** 4

#### LOG-12: Add Async Audit Log Writer
- **Status:** TODO
- **Location:** `src/logging.rs`
- **Description:** Currently blocking IO in main thread.
- **Dependencies:** None
- **Wave:** 4

#### LOG-13: Add Startup Self-Check
- **Status:** TODO
- **Location:** `src/logging.rs`
- **Description:** Verify log directory writable before initializing.
- **Dependencies:** None
- **Wave:** 4

#### LOG-14: Add Structured Error Context to log_command_execution
- **Status:** TODO
- **Location:** `src/logging.rs`
- **Description:** Could include error kind, source location, stack trace.
- **Dependencies:** None
- **Wave:** 4

#### LOG-15: Implement tracing::instrument for Function Spans
- **Status:** TODO
- **Location:** `src/logging.rs`
- **Description:** Could provide automatic span creation for key functions.
- **Dependencies:** None
- **Wave:** 4

#### LIB-11: Add sort_by_updated_at() Method to Snippets
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** Make explicit and testable.
- **Dependencies:** LIB-1
- **Wave:** 4

#### LIB-12: Consider Using chrono DateTime Instead of i64
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** `created_at`/`updated_at` stored as `i64`. Using `DateTime<Utc>` would be more self-documenting.
- **Dependencies:** None
- **Wave:** 4

#### LIB-13: Expose Library Backup Functionality Through LibraryManager
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** Add `backup_library()` method to `LibraryManager`.
- **Dependencies:** LIB-2
- **Wave:** 4

#### LIB-14: Document That Deleted Snippets Filtered Elsewhere
- **Status:** TODO
- **Location:** `src/library.rs`
- **Description:** Add note that `deleted: true` filtering happens in `sync_commands.rs`, not here.
- **Dependencies:** None
- **Wave:** 4

#### OV-2: Rate Limiter Should Support Persistence
- **Status:** TODO
- **Location:** `snip-sync/src/rate_limiter.rs`
- **Description:** In-memory rate limiter loses state on restart.
- **Dependencies:** None
- **Wave:** 4

#### OV-3: Snippet ID Collision on Merge
- **Status:** TODO
- **Location:** `src/sync_commands.rs:394-475`
- **Description:** When `updated_at` ties, `device_id` comparison is string-based and non-deterministic.
- **Dependencies:** None
- **Wave:** 3

#### OV-4: Missing Input Validation on Snippet Creation
- **Status:** TODO
- **Location:** `src/commands/new_cmd.rs`
- **Description:** Should validate command/description length, tag count/length limits.
- **Dependencies:** None
- **Wave:** 3

---

## KNOWN/ACCEPTED LIMITATIONS

### Scope Constraints (Cannot Fix Without Breaking Change)

#### LIM-1: output Field Not Encrypted During Sync
- **Status:** Known Limitation
- **Location:** `src/sync.rs:60-64`
- **Description:** The `output` field is not encrypted during sync because the proto definition lacks the field. Cannot add without breaking API change.
- **Note:** Per AGENTS.md scope constraints.

#### LIM-2: \< Escape Inconsistency in variables.rs
- **Status:** Known Limitation (Documented)
- **Location:** `src/utils/variables.rs`
- **Description:** `\<` escape sequence inconsistency between parse and expand is a documented known edge case per AGENTS.md.
- **Note:** Per AGENTS.md scope constraints.

#### LIM-3: CLI Documentation Discrepancies
- **Status:** Known Limitation (Doc Bugs)
- **Location:** Multiple
- **Description:** Many CLI documentation discrepancies (e.g., `--clip` behavior, cron intervals) are doc bugs not code bugs.
- **Note:** Per AGENTS.md scope constraints.

### Deferred Items (Not Implemented)

#### KL-1: Command Injection Warning (Safe Mode)
- **Status:** DEFERRED
- **Location:** `src/commands/run_cmd.rs`
- **Description:** Not implemented. "Safe mode" that warns before executing shell commands with variables.
- **Files:** `src/commands/run_cmd.rs`

#### KL-2: TUI Pre-computed Highlights Memory Pressure
- **Status:** DEFERRED
- **Location:** `src/ui/mod.rs`
- **Description:** Syntax highlighting pre-computed once at startup. Could cause memory pressure for large libraries.
- **Files:** `src/ui/mod.rs`, `src/ui/highlight.rs`

---

## Remaining Items from Original Plan

### 6. Command Injection Warning — ⚠️ PARTIAL

**Status:** The `run` command executes snippet commands via `Command::new(&shell).arg("-c").arg(&final_command)` with no user confirmation or sandboxing.

**What was done:** Basic execution flow works. No warning prompt before running commands.

**What remains (optional):**
- Add a confirmation prompt before executing snippet commands (opt-in via config)
- Add a `--safe` flag that restricts execution to allowlisted commands
- Log executed commands to the audit log

**Risk:** Low for personal use (user creates their own snippets). Higher for premade libraries from untrusted sources.

**Files:** `src/commands/run_cmd.rs`

---

### 13. TUI Pre-computed Highlights Memory Pressure — 🔲 TODO

**Status:** All snippet commands are syntax-highlighted eagerly when the TUI opens (`src/ui/mod.rs:137-140`). For large libraries (10K+ snippets), this allocates many small `String` objects that persist for the entire TUI session.

**Proposed mitigations:**
- On-demand highlighting: only highlight visible items + a small buffer
- LRU cache with eviction for off-screen items
- Truncate very long commands in highlights

**Files:** `src/ui/mod.rs`, `src/ui/highlight.rs`

---

## Completed in Prior Work

The following items were verified as fixed during architecture review implementation (per AGENTS.md Implementation Notes 2026-05-29):

| ID | Issue | Location | Fix Applied |
|----|-------|----------|-------------|
| ENCRYPT-1 | Encryption ineffective `drop(key)` (encrypt) | `src/encryption.rs:176` | Removed no-op `drop(key)` calls |
| ENCRYPT-2 | Encryption ineffective `drop(key)` (decrypt) | `src/encryption.rs:195` | Removed no-op `drop(key)` calls |
| CLIP-1 | Clipboard auto-clear debug→warn | `src/clipboard.rs:37` | Changed to `tracing::warn` |
| CLIP-2 | Clipboard redundant drop | `src/clipboard.rs:42` | Removed redundant drop |
| UI-1 | Visual mode copy bug | `src/ui/mod.rs:672` | `y` now copies commands |
| SYNC-2 | Merge equal timestamps | `src/sync_commands.rs:429` | Changed `>` to `>=` |
| SYNC-1 | Push-only counter bug | `src/sync_commands.rs:306-323` | `completed` increments regardless |
| SERVER-1 | Premade TOCTOU | `snip-sync/src/premade.rs:199` | Reads from `canonical_path` |
| SERVER-2 | Health check DB ping | `snip-sync/src/main.rs:343-352` | Now verifies DB connectivity |

---

## Dependencies Graph

```
WAVE 1 (Security-Critical):
SEC-1 (Output Path Validation)     └─ No dependencies
SEC-2 (Editor Path Resolution)     └─ No dependencies
SEC-3 (TLS Verification)           └─ No dependencies
SEC-5 (Shell $SHELL)               └─ No dependencies
SEC-6 (API Key in Memory)          └─ No dependencies
CLI-1 (TOCTOU Race)                └─ No dependencies (lines 92-93)

WAVE 2 (Core Bugs):
CORE-1 (Atomic Config Saves)       └─ No dependencies
CORE-2 (deleted flag filter)      └─ No dependencies
CORE-3 (Silent Migration)          └─ No dependencies
CLI-3 (TUI Exit Always Syncs)      └─ No dependencies
CORE-5 (Empty Command Validation)  └─ No dependencies
CORE-6 (Config Error Swallowing)  └─ No dependencies

WAVE 3 (Improvements):
SEC-7 (TLS Enforcement)           └─ No dependencies
SEC-8 (HTTPS Default)             └─ No dependencies
SEC-9 (Keychain Failure)          └─ No dependencies
CMD-3 (run_cmd --clip)             └─ No dependencies
CMD-10 (sync error propagation)    └─ No dependencies
CMD-11 (premade sync return)       └─ No dependencies
LIB-1 (Sort on Save)               └─ No dependencies
LIB-2 (Auto backup)               └─ No dependencies
LOG-2 (Audit log growth)           └─ No dependencies
LOG-3 (Audit log failure silent)    └─ No dependencies

WAVE 4 (Low Priority):
TUI-3 (Keybinding Fixes)           → TUI-7, TUI-8, TUI-9, TUI-10, TUI-20, TUI-21
LIB-1 (Sort on Save)               → LIB-11 (sort_by_updated_at method)
LIB-6 (name->description migration)→ LIB-8 (name alias)
LIB-2 (Auto backup)               → LIB-13 (expose via LibraryManager)
```

---

## Summary Statistics

| Category | Count |
|----------|-------|
| **WAVE 1 Security** | 6 |
| **WAVE 2 Core Bugs** | 6 |
| **WAVE 3 Improvements** | 10 |
| **WAVE 4 Low Priority** | 50+ |
| **Known Limitations** | 6 |
| **Already Fixed** | 9 |
| **Total Unique Items** | 80+ |