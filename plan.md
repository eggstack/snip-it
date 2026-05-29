# snip-it Consolidated Remediation Plan

**Generated:** 2026-05-29 from architecture review files
**Last verified:** 2026-05-29 against codebase
**Implementation completed:** 2026-05-29

## Priority Legend

| Level | Meaning |
|-------|---------|
| **P0** | Security vulnerability or data loss risk |
| **P1** | Bug affecting functionality or correctness |
| **P2** | Performance or scalability concern |
| **P3** | Code quality, consistency, or minor improvement |

## Status Legend

| Symbol | Meaning |
|--------|---------|
| ✅ DONE | Fully implemented and verified |
| ⚠️ PARTIAL | Core fix done, additional work remains |
| 🔲 TODO | Not yet implemented |
| ⛔ WONTFIX | Accepted as-is |

---

## Implementation Waves

Items within a wave can be parallelized across sub-agents (each touches independent files). Waves must be executed sequentially.

| Wave | Focus | Parallelizable Items |
|------|-------|---------------------|
| **1** | Security Critical | 6 items — all independent files |
| **2** | Core Bugs | 8 items — mostly independent |
| **3** | Server Improvements | 5 items — all in `snip-sync/src/` |
| **4** | Code Quality & UX | 7 items — independent files |
| **5** | UI Refactor & Docs | 4 items — ui.rs split, doc updates |

---

## Wave 1: Security Critical

All items touch different files and can be implemented simultaneously.

### 1.1 — Increase Argon2 Memory Cost ✅ DONE

**Files:** `src/encryption.rs:32`, `snip-sync/src/db.rs:12`

**Problem:** `ARGON2_MEMORY_COST_KIB = 1 << 6` (64 KiB) is 300x below OWASP minimum of 19 MiB for Argon2id.

**Status:** Fixed. Changed to `1 << 14` (16 MiB) in both files.

**Verification:** `cargo test --lib` + `cargo test -p snip-sync`

---

### 1.2 — API Key Plaintext Storage ✅ DONE

**Files:** `src/config.rs:21`, `Cargo.toml`

**Problem:** `SyncSettings.api_key` is stored as plaintext in `sync.toml`. Any process with filesystem read access gets full sync account access.

**Fix:**
1. Add `keyring` crate dependency to `Cargo.toml`.
2. On `save_sync_settings`, write API key to OS keychain. Store marker `api_key = "@keychain"` in `sync.toml`.
3. On `load_sync_settings`, if `api_key == "@keychain"`, fetch from keychain.
4. On first run, migrate existing plaintext key to keychain.
5. Fallback: if keychain unavailable, keep plaintext but log warning.

**Verification:** `cargo test` + manual test: run sync, verify `sync.toml` contains `@keychain`, verify sync still works.

---

### 1.3 — CORS Env Var Not Implemented ✅ DONE

**Files:** `snip-sync/src/main.rs:998-1013` (CORS config block)

**Problem:** Warning message at line 1001 references `CORS_ALLOW_ALL=true` but the env var is never read. Users who set it get no effect. When origins are empty, `CorsLayer::new()` blocks all cross-origin requests (not allows all as the message implies).

**Fix:**
1. Read `CORS_ALLOW_ALL` env var in the CORS configuration block.
2. When set to `"true"`, use `CorsLayer::new().allow_origin(Any)`.
3. Update the warning message to accurately describe behavior.
4. The actual env var to read is `CORS_ALLOW_ALL` (not `CORS_ALLOWED_ORIGINS` which is different).

**Verification:** Start server with `CORS_ALLOW_ALL=true`, make cross-origin request, confirm it succeeds.

---

### 1.4 — Rate Limiting Gaps on Read Endpoints ✅ DONE

**Files:** `snip-sync/src/main.rs` — `get_snippets` (line 364), `list_libraries` (line 706)

**Problem:** `get_snippets` and `list_libraries` skip rate limiting. All other endpoints apply it.

**Fix:** Add `self.rate_limiter.allow()` check in both methods, matching the pattern used by mutating endpoints. Example pattern from `push_snippets`:
```rust
if !self.rate_limiter.allow(&req.api_key, self.config.rate_limit_per_minute as usize, Duration::from_secs(60)) {
    return Err(Status::resource_exhausted("Rate limit exceeded"));
}
```

**Verification:** `cargo test -p snip-sync` + manual test: hit endpoint repeatedly, confirm rate limit kicks in.

---

### 1.5 — Registration Rate Limit Bypass ✅ DONE

**Files:** `snip-sync/src/main.rs:330-336` — `register` method

**Problem:** Rate limit key is `req.device_id` which is client-controlled. Clients can rotate device_id to bypass registration rate limits.

**Fix:** Use IP address or a server-generated token as the rate limit key instead of client-provided `device_id`. The `RegisterRequest.device_id` field is ignored by the server anyway (server generates its own UUID at line 338).

**Verification:** `cargo test -p snip-sync`.

---

### 1.6 — TLS Documentation Mismatch ✅ DONE

**Files:** `snip-sync/src/main.rs:920-922`, `src/config.rs:61`, `src/sync.rs:299-316`

**Problem:** Client (`src/sync.rs`) always configures TLS, but server default is `http://localhost:50051` (no TLS). The TLS handshake from the client will fail on default server config. Default server URL in client config is also HTTP.

**Fix:**
1. Update documentation to clearly state: server requires TLS or reverse proxy for production.
2. Consider changing default server URL in `config.rs:61` to note this requirement.
3. Consider adding built-in TLS support via `tonic`'s TLS features (optional, lower priority).

**Verification:** Read updated docs. Test client → server connection with default config.

---

## Wave 2: Core Bugs

Most items touch independent files and can be parallelized.

### 2.1 — Sync Fall-Through Bug ✅ DONE

**Files:** `src/commands/sync_cmd.rs:210-253`

**Problem:** When `client.list_libraries()` fails at line 210, execution falls through (no `return`), hitting lines 237-251 which calls `list_and_link_server_libraries()` and then `run_sync()` again.

**Fix:** Add `return Ok(());` after the `Err` branch at line 234, or restructure to use `if let Ok(libs)`.

**Current buggy structure:**
```rust
match runtime.block_on(client.list_libraries()) {
    Ok(libs) => { ... return Ok(()); }  // returns
    Err(e) => eprintln!("..."),         // does NOT return!
}
// Falls through to line 237-251 which also calls sync
```

**Verification:** `cargo test --lib`.

---

### 2.2 — Sync Encryption Failures Cause Permanent Snippet Loss ✅ DONE

**Files:** `src/sync.rs:96-107`, `src/sync_commands.rs:300-320`

**Problem:** When encryption fails for a snippet, it's silently excluded from the sync request. But `last_sync` timestamp is still updated, so those snippets are never retried on subsequent syncs. They're permanently lost from sync.

**Fix:**
1. Track failed snippet IDs separately from successfully encrypted ones.
2. Only update `last_sync` to cover snippets that were successfully encrypted and sent.
3. Alternatively, abort sync entirely on encryption failure and don't update `last_sync`.

**Verification:** `cargo test --lib` (existing merge tests) + new test: encrypt failure mid-batch, verify last_sync doesn't advance past failed snippet.

---

### 2.3 — `set_primary()` No-Op on Missing Filename ✅ DONE

**Files:** `src/library.rs:346-352`

**Problem:** Calling `set_primary("nonexistent")` silently succeeds but leaves all libraries with `is_primary: false`. Config is saved in invalid state.

**Fix:** Return `SnipError::runtime_error` if the filename doesn't exist in the library list. Add validation:
```rust
if !self.config.libraries.iter().any(|lib| lib.filename == filename) {
    return Err(SnipError::runtime_error(
        "Library not found",
        Some(&format!("No library with filename '{}'", filename)),
    ));
}
```

**Verification:** `cargo test --lib`.

---

### 2.4 — `add_server_library()` Creates Duplicates ✅ DONE

**Files:** `src/library.rs:387-413`

**Problem:** Pushes a new `LibraryMeta` unconditionally. If library with same filename already exists, creates duplicate entry.

**Fix:** Check if a `LibraryMeta` with the same `filename` already exists. If so, update its `library_id` instead of pushing a new entry. Match the pattern used by `add_existing_library` (line 362-377):
```rust
if let Some(existing) = self.get_library_by_filename(&filename) {
    // Update existing library's library_id
    return Ok(());
}
```

**Verification:** `cargo test --lib`.

---

### 2.5 — `load_snippets` Silent Data Loss ✅ DONE

**Files:** `src/commands/mod.rs:102-141`

**Problem:** On TOML parse failure, returns `Ok(Snippets::default())`. Caller's subsequent `save_snippets` overwrites the file with empty collection. The backup saves the corrupted original, but in-memory state is empty.

**Fix:** Return `Err(SnipError::toml_error(...))` on parse failure instead of returning defaults. Let callers decide how to handle (the backup is already created). Three specific code paths to fix:
- Lines 108-111: file read error
- Lines 120-137: TOML parse error

**Verification:** `cargo test --lib`.

---

### 2.6 — Clipboard Auto-Clear Race Condition ✅ DONE

**Files:** `src/clipboard.rs:23-43`

**Problem:** When auto-clear timer is running and user copies new content, the new `schedule_clipboard_clear` is rejected (flag still true). Old thread wakes up and clears the new content.

**Fix:** Use a generation counter (`AtomicU64`) instead of `AtomicBool`. Each new schedule increments the counter. The sleeping thread checks if its generation matches the current counter before clearing:
```rust
static CLIPBOARD_GENERATION: AtomicU64 = AtomicU64::new(0);
// In schedule: let gen = CLIPBOARD_GENERATION.fetch_add(1, SeqCst) + 1;
// In sleeping thread: if gen != CLIPBOARD_GENERATION.load(SeqCst) { return; }
```

**Verification:** `cargo test --lib` + manual test: copy snippet A, immediately copy snippet B, verify B is not cleared after A's timeout.

---

### 2.7 — Shutdown Logging Order ✅ DONE

**Files:** `src/logging.rs:93-98`

**Problem:** `shutdown_logging` drops the guard (flushing writer) before logging "Logging shutdown complete". Message is lost.

**Fix:** Swap order: log the shutdown message, then drop the guard:
```rust
pub fn shutdown_logging() {
    if let Some(guard) = LOG_GUARD.lock().unwrap().take() {
        tracing::info!("Logging shutdown complete");
        drop(guard);
    }
}
```

**Verification:** `cargo test --lib`.

---

### 2.8 — Dead `config.level` Field ✅ DONE

**Files:** `src/logging.rs:33,42,61-62,81`

**Problem:** `LogConfig.level` is logged but never used to configure the filter. The actual filter is always `RUST_LOG` env var or hardcoded default.

**Fix:** Either wire `config.level` into the `EnvFilter` construction, or remove the field and document that `RUST_LOG` is the source of truth.

**Verification:** `cargo test --lib`.

---

## Wave 3: Server Improvements

All items in `snip-sync/src/`. Can be parallelized.

### 3.1 — N+1 Query in `list_libraries` ✅ DONE

**Files:** `snip-sync/src/db.rs:291-302`

**Problem:** For each library, a correlated subquery counts snippets. O(n) queries for n libraries.

**Fix:** Replace with a JOIN or batch count query. Use a single query that fetches libraries with snippet counts:
```sql
SELECT l.id, l.name, l.created_at, COUNT(s.id) as snippet_count
FROM libraries l
LEFT JOIN snippets s ON s.library_id = l.id AND s.deleted = 0
WHERE l.user_id = ? AND l.deleted_at IS NULL
GROUP BY l.id
```

**Verification:** `cargo test -p snip-sync`.

---

### 3.2 — Auth+Rate-Limit Middleware Extraction ✅ DONE

**Files:** `snip-sync/src/main.rs` (all RPC handlers)

**Problem:** Auth check + rate limit check + library ownership check is copy-pasted across 7+ endpoints.

**Fix:** Extract into a tonic interceptor or helper function:
```rust
async fn authenticate_and_rate_limit(&self, req: &impl HasApiKey) -> SnipResult<String> {
    // Rate limit check
    // Auth check
    // Return user_id
}
```

**Verification:** `cargo test -p snip-sync`.

---

### 3.3 — Sync Response `skipped_count`/`skipped_ids` ✅ DONE

**Files:** `snip-sync/src/main.rs:583-652`

**Problem:** `SyncResponse` always returns `skipped_count: 0` and empty `skipped_ids` even when snippets fail validation/upsert.

**Fix:** Track skipped snippet IDs during the sync loop and populate the response fields. Add a `Vec<String>` accumulator in the sync loop.

**Verification:** `cargo test -p snip-sync`.

---

### 3.4 — Same-Timestamp Upsert Tie-Breaking ✅ DONE

**Files:** `snip-sync/src/db.rs:478`

**Problem:** `WHERE excluded.updated_at > snippets.updated_at` silently drops updates when timestamps are equal.

**Fix:** Add tie-breaking:
```sql
WHERE excluded.updated_at > snippets.updated_at
   OR (excluded.updated_at = snippets.updated_at AND excluded.device_id > snippets.device_id)
```

**Verification:** `cargo test -p snip-sync`.

---

### 3.5 — Dead Code Cleanup (Server) ✅ DONE

**Files:** `snip-sync/src/db.rs:374-388`, `snip-sync/src/db.rs:22-23`, `snip-sync/src/main.rs:255`

**Problem:**
- `verify_snippet_ownership` is unused (`#[allow(dead_code)]` at line 374)
- `DbError::Unauthorized` variant is never constructed (line 22)
- `record_request` takes `_method` string but ignores it (line 255) — the method is NOT dead code, but the parameter is unused

**Fix:**
1. Remove `verify_snippet_ownership` function entirely
2. Remove `DbError::Unauthorized` variant
3. Either remove `_method` parameter from `record_request` or use it in logging

**Verification:** `cargo clippy --all-targets -- -D warnings` + `cargo test -p snip-sync`.

---

## Wave 4: Code Quality & UX

Items touch independent files, can be parallelized.

### 4.1 — `_config` Flag Silently Ignored ✅ DONE

**Files:** `src/commands/run_cmd.rs`, `src/commands/clip_cmd.rs`, `src/commands/search_cmd.rs`, `src/commands/edit_cmd.rs`, `src/commands/sync_cmd.rs`, `src/main.rs`

**Problem:** `run`, `clip`, `search`, `edit`, `sync` accept `--config` but the parameter is `_config: Option<PathBuf>` and never used. Users think they can point to alternate config files but the flag does nothing.

**Fix:** Remove the `--config` flag from these 5 commands in `main.rs`. These commands use library mode and the config path is always resolved via `get_library_path`.

**Verification:** `cargo clippy --all-targets -- -D warnings` + `cargo test`.

---

### 4.2 — ~~Dead Code in `run_cmd.rs`~~ REMOVED — Was Inaccurate

**Status:** This item was found to be inaccurate during verification. The `Component::Normal(c)` branch at `run_cmd.rs:35-40` checks `c.to_string_lossy().contains("..")` and this IS reachable (e.g., for path components like `foo..bar`). The `Component::ParentDir` case on line 29 handles literal `..` while the `Normal` case handles embedded `..` within a component name. Both are necessary. **No action needed.**

---

### 4.3 — Cron Interval Zero Validation ✅ DONE

**Files:** `src/commands/cron_cmd.rs:4-12`, `src/main.rs:166-168`

**Problem:** `--interval 0` produces invalid cron syntax `*/0 * * * *` (minute field must be 1-59 for step values). No validation.

**Fix:** Validate `interval >= 1` before generating crontab entry. Return error with helpful message:
```rust
if interval == 0 {
    return Err(SnipError::runtime_error(
        "Invalid interval",
        Some("Interval must be at least 1 minute"),
    ));
}
```

**Verification:** `cargo test --lib`.

---

### 4.4 — Shell Stderr Not Captured on Execution Failure ✅ DONE

**Files:** `src/commands/run_cmd.rs:99-111`

**Problem:** When a snippet command fails, only the exit code is logged. stderr is discarded.

**Fix:** Use `Command::new(&shell).arg("-c").arg(&final_command).output()` instead of `.status()`, then display stderr when the command fails:
```rust
let output = Command::new(&shell).arg("-c").arg(&final_command).output()?;
if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("Error: {}", stderr);
}
```

**Verification:** `cargo test --lib` + manual test: run a snippet that produces stderr, verify it's shown.

---

### 4.5 — Variable `Variable` Struct Location ✅ DONE

**Files:** `src/ui.rs:120-124`, `src/utils/variables.rs:6`

**Problem:** `Variable` struct (simple data struct with `name` and `default` fields) lives in `ui.rs`. `utils/variables.rs` imports it, creating a `utils → ui` dependency.

**Fix:** Move `Variable` struct to `src/utils/variables.rs` or a shared types module. Update imports in `ui.rs`. The struct is a plain data type with no UI dependencies.

**Verification:** `cargo clippy --all-targets -- -D warnings` + `cargo test`.

---

### 4.6 — Shell Keyword HashSet Linear Scan ✅ DONE

**Files:** `src/ui.rs:312`

**Problem:** `shell_keywords.iter().any(|kw| word == *kw)` iterates the HashSet instead of using `.contains()`. O(n) per word instead of O(1).

**Fix:** Change to `shell_keywords.contains(word.as_str())`.

**Verification:** `cargo clippy --all-targets -- -D warnings`.

---

### 4.7 — Proto Build Trigger Missing ✅ DONE

**Files:** `snip-proto/build.rs`

**Problem:** No `cargo:rerun-if-changed=proto/sync.proto` directive. Cargo may not re-run the build script when the proto file is modified.

**Fix:** Add `println!("cargo:rerun-if-changed=proto/sync.proto");` at the start of `build.rs`.

**Verification:** Modify `sync.proto`, run `cargo build`, verify `snip_proto.rs` is regenerated.

---

## Wave 5: UI Refactor & Documentation

### 5.1 — Split `ui.rs` Monolith ✅ DONE

**Files:** `src/ui.rs` (1416 lines)

**Problem:** Largest file in codebase. Contains theme system, syntax highlighting, main TUI loop, variable prompting, and tests.

**Fix:** Extract into submodules:
- `src/ui/theme.rs` — `Theme`, `resolve_theme`, `ACTIVE_THEME`, `get_theme()`
- `src/ui/highlight.rs` — `highlight_command`, syntax highlighting logic
- `src/ui/variables.rs` — `prompt_variables_inner`
- `src/ui/mod.rs` — `select_snippet_inner`, re-exports

**Verification:** `cargo clippy --all-targets -- -D warnings` + `cargo test` + manual TUI test.

---

### 5.2 — Fix Architecture Doc Line Count ✅ DONE

**Files:** `architecture/ui.md`

**Problem:** Doc says `~1250 lines` for `ui.rs`, actual is 1416.

**Fix:** Update to `~1400 lines`.

---

### 5.3 — Fix Architecture Doc Command Count ✅ DONE

**Files:** `architecture/overview.md`

**Problem:** System diagram says `(12 cmds)` but there are 13 subcommands.

**Fix:** Update to `(13 cmds)`.

---

### 5.4 — Fix `cli.md` Module Description ✅ DONE

**Files:** `architecture/cli.md`

**Problem:** Says "Each module exposes a `run()` function" but `premade_cmd` and `library_cmd` use subcommand-dispatched functions (`run_list`, `run_get`, etc.).

**Fix:** Add note that two modules use subcommand-dispatched functions instead of a single `run()`.

---

## Existing Items from Previous Plan

These items were already tracked. Their status is preserved:

| # | Status | Description |
|---|--------|-------------|
| 1 | ✅ DONE | API Key Verification O(n) — prefix indexing |
| 2 | ✅ DONE | Premade Rate Limiting shared key |
| 3 | ✅ DONE | Destructive soft-delete pull — merge logic done, last_sync now skips on encryption failure |
| 4 | ✅ DONE | API key plaintext → keychain (see Wave 1.2) |
| 5 | ✅ DONE | CORS allow-all — restrictive default done, env var pending (see Wave 1.3) |
| 6 | ⚠️ PARTIAL | Command injection warning — done, safe mode optional |
| 7 | ✅ DONE | Signal handler Windows compatibility |
| 8 | ✅ DONE | `_user_id` unused in premade listing |
| 9 | ✅ DONE | Backup function panic risk |
| 10 | ✅ DONE | Silent library parse error — backup done, Result return pending (see Wave 2.5) |
| 11 | ⚠️ PARTIAL | TOCTOU race in premade — canonicalize done, residual exists() check |
| 12 | ✅ DONE | Unbounded request limits |
| 13 | 🔲 TODO | TUI pre-computed highlights memory pressure |
| 14 | ⛔ WONTFIX | Clipboard thread leak |
| 15 | ⛔ WONTFIX | Rate limiter cleanup task |
| 16 | ✅ DONE | TOML backslash regex limitations |
| 17 | ⛔ WONTFIX | Duplicate TOML escape fix in premade |
| 18 | ✅ DONE | Inconsistent naming snip-it vs snp |
| 19 | ✅ DONE | `sync_with_retry` reimplements macro |
| 20 | ✅ DONE | Dead code PLUGINS static |
| 21 | ✅ DONE | Unused variable `_metrics_for_http` |
| 22 | ✅ DONE | Dead code on `AppState` |
| 23 | ✅ DONE | Missing CI linting |

---

## Verification Commands

```bash
# Unit tests
cargo test --lib

# Integration tests
cargo test --test integration

# Server tests
cargo test -p snip-sync

# Lint
cargo clippy --all-targets -- -D warnings

# Format check
cargo fmt --check

# Cross-platform check
cargo check --target x86_64-pc-windows-msvc
```
