# snip-it Remediation Plan

**Last updated:** 2026-05-29

All items from the original 5-wave remediation (30+ security, bug, and quality fixes) have been completed and verified. Two deferred items remain.

## Remaining Items

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

## Completed Items (for reference)

All items below have been implemented and verified in code:

| Wave | Items |
|------|-------|
| 1 - Security | 1.1 Argon2 memory cost, 1.2 API key keychain, 1.3 CORS env var, 1.4 Rate limiting reads, 1.5 Registration rate limit, 1.6 TLS docs |
| 2 - Core Bugs | 2.1 Sync fall-through, 2.2 Encryption failure loss, 2.3 set_primary validation, 2.4 add_server_library dedup, 2.5 load_snippets error, 2.6 Clipboard race, 2.7 Shutdown logging, 2.8 config.level wiring |
| 3 - Server | 3.1 N+1 query, 3.2 Auth middleware, 3.3 Skipped count/ids, 3.4 Upsert tie-breaking, 3.5 Dead code cleanup |
| 4 - Quality | 4.1 Remove --config flag, 4.3 Cron interval validation, 4.4 Shell stderr capture, 4.5 Variable struct location, 4.6 HashSet contains, 4.7 Proto build trigger |
| 5 - UI/Docs | 5.1 ui.rs split, 5.2-5.4 Architecture doc updates |
| Previous | #1-5,7-12,16-23 all completed (see git history) |
