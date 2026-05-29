# Overview Module Review

**Date:** 2026-05-29
**Architecture Doc:** architecture/overview.md
**Source Files:** src/main.rs, src/commands/, snip-sync/src/, snip-proto/

## Document Accuracy

### Verified Correct
- All file paths in the workspace layout exist (`src/main.rs`, `src/commands/`, `src/utils/`, `src/clipboard.rs`, `src/config.rs`, `src/encryption.rs`, `src/error.rs`, `src/library.rs`, `src/logging.rs`, `src/sync.rs`, `src/sync_commands.rs`, `src/ui.rs`, `snip-proto/proto/sync.proto`, `snip-proto/build.rs`, `snip-proto/src/lib.rs`, `snip-sync/src/main.rs`, `snip-sync/src/db.rs`, `snip-sync/src/rate_limiter.rs`, `snip-sync/src/metrics.rs`, `snip-sync/src/premade.rs`, `tests/integration.rs`)
- All 12 architecture doc links (`cli.md`, `ui.md`, `core.md`, `config.md`, `sync.md`, `encryption.md`, `clipboard.md`, `utils.md`, `logging.md`, `server.md`, `proto.md`) exist in `architecture/`
- "TOML storage" claim: Snippets use `[[Snippets]]` TOML arrays via serde (`src/library.rs:29`)
- "End-to-end encryption" claim: AES-256-GCM + Argon2id confirmed (`src/encryption.rs`)
- "Last-write-wins merge" claim: `merge_snippets()` at `src/sync_commands.rs:375` compares `updated_at` timestamps
- "Local-only fields preserved" claim: `output`, `folders`, `favorite` are kept from local when server wins (`src/sync_commands.rs:415-418`)
- "Pre-computed TUI highlights" claim: `highlight_command()` called once at `src/ui.rs:357`, not inside draw loop
- "Lazy async runtime" claim: `RUNTIME: LazyLock<Runtime>` at `src/main.rs:43`
- "Audit logging" claim: `audit_log()` function at `src/logging.rs:215`, called from `run_cmd.rs:60`
- Data flow descriptions match actual code paths
- Component index links and descriptions are accurate

### Discrepancies

1. **"12 commands" count is ambiguous** (lines 13, 57, 87)
   - The `Commands` enum in `src/main.rs` has **13** top-level subcommands: Version, New, List, Run, Clip, Search, Edit, Keybindings, Sync, Cron, Register, Library, Premade
   - The `src/commands/` directory has **12** module files (no `version_cmd.rs` — Version is inlined)
   - The doc says "12 CLI subcommands (one module each)" — the "one module each" qualifier is accurate for the 12 non-Version commands, but the count conflates module files with CLI subcommands
   - Library and Premade each have sub-subcommands (LibraryCommands, PremadeCommands), so the total user-facing command surface is larger

2. **"pet format" compatibility unverifiable** (line 132)
   - The doc claims "compatible with `pet` format" but there is no reference to `pet` anywhere in the source code
   - The TOML format uses `[[Snippets]]` with mixed casing (e.g., `Id`, `Description`, `Tag`, `command`, `output`) which may not match `pet`'s exact schema
   - Serde aliases (`alias = "Description"`, `alias = "Tag"`) provide read-compatibility, but written format uses renamed variants (`Id`, `Description`, `Output`, `Tags`)

3. **TOML field naming inconsistency** (not documented)
   - `Snippet` struct fields use inconsistent casing: `Id`, `Description`, `Output`, `Tags`, `Command` (capitalized serde renames) vs `folders`, `favorite`, `created_at`, `updated_at`, `device_id`, `deleted` (lowercase, no renames)
   - This is a legacy artifact from the original format vs newer sync fields, but it's not mentioned in the architecture doc

## Bugs & Issues

### B1. Server `list_libraries` skips rate limiting (MEDIUM)
- **Location:** `snip-sync/src/main.rs:706-754`
- The `list_libraries` gRPC handler authenticates the user but does NOT call `self.rate_limiter.allow()` before proceeding. All other mutating endpoints (`register`, `push_snippets`, `sync`, `create_library`, `delete_library`) and even `list_premade_libraries` and `get_premade_library` apply rate limiting.
- **Impact:** `list_libraries` can be called without rate limit, enabling potential abuse for enumeration.

### B2. Server `get_snippets` skips rate limiting (MEDIUM)
- **Location:** `snip-sync/src/main.rs:364-439`
- Same issue as B1: `get_snippets` authenticates but does not check rate limits.

### B3. Audit log errors silently swallowed in production (LOW)
- **Location:** `src/logging.rs:220-224`
- `audit_log()` returns `Ok(())` when it can't get the log path or open the file. Callers (`run_cmd.rs:60-62`, `run_cmd.rs:85-87`) log failures at `debug` level only. This is documented as intentional ("non-critical feature"), but it means audit trail gaps are invisible in production unless debug logging is enabled.

## Design Issues

### D1. Mixed casing in TOML format creates user confusion
- **Location:** `src/library.rs:41-64`
- The `Snippet` struct produces TOML with inconsistent field casing: `[[Snippets]]` section uses `Id`, `Description`, `Output`, `Tags` (capitalized) alongside `command`, `folders`, `favorite` (lowercase). This makes the format feel inconsistent and harder for users to hand-edit.

### D2. `Keybindings` subcommand has no short alias
- **Location:** `src/main.rs:146-147`
- Every other subcommand has an alias (`v`, `n`, `l`, `r`, `c`, `s`, `e`, `k`, `y`, `reg`, `lib`, `p`). Keybindings has `alias = "k"` but the struct shows `#[command(alias = "k")]` which IS present. However, this is the only command where the alias is the same as the short flag pattern used elsewhere (`-k`). This is actually fine — noted for completeness.

### D3. `process_snippet` in `run_cmd.rs` calls `audit_log` on success but not on copy-to-clipboard path
- **Location:** `src/commands/run_cmd.rs:54-74` vs `run_cmd.rs:83-92`
- When a snippet is executed via shell (`run`), audit log is called at line 60. When a snippet is copied via the `copy` flag (double-click in TUI), audit log is called at line 85. Both paths are covered — this is correct.

### D4. Server `sync` endpoint returns `skipped_count: 0` and `skipped_ids: vec![]` unconditionally
- **Location:** `snip-sync/src/main.rs:645-652`
- The response always returns `skipped_count: 0` and empty `skipped_ids` regardless of whether snippets were actually skipped during validation. This is misleading — the validation at lines 585-588 can skip invalid snippets, but this isn't reflected in the response.

## Security Concerns

### S1. Argon2 memory cost is extremely low (HIGH)
- **Location:** `src/encryption.rs:32`
- `ARGON2_MEMORY_COST_KIB = 1 << 6` = **64 KiB**
- OWASP recommends Argon2id with 19-46 MiB minimum memory cost. 64 KiB is ~300-700x below the minimum recommended. While the API key is stronger than a typical password, this makes brute-force attacks on the derived key significantly cheaper, especially on GPUs.
- **Recommendation:** Increase to at least `1 << 16` (64 MiB) or `1 << 19` (512 MiB) depending on acceptable latency.

### S2. Server gRPC has no TLS in default configuration (MEDIUM)
- **Location:** `snip-sync/src/main.rs:920-921`
- The server logs a warning: "TLS is not enabled. For production, use a reverse proxy with TLS (nginx, traefik, etc.)" but still starts without TLS. Snippets (even encrypted ones) are transmitted over plaintext gRPC. The client (`src/sync.rs`) DOES use TLS (`create_tls_channel`), so there's a TLS mismatch — the client expects TLS but the server doesn't provide it by default.
- **Impact:** In a default deployment, the TLS handshake from the client would fail. This is a usability issue that could confuse new users.

### S3. Server CORS warning is misleading (LOW)
- **Location:** `snip-sync/src/main.rs:954-956`
- When no CORS origins are configured, the warning says "requests from any origin will be allowed" but the actual code at line 998-1003 creates a `CorsLayer::new()` with NO allow-origin rules, which means cross-origin requests are actually **blocked** (default CORS behavior denies requests without matching origin).
- **Impact:** Misleading log message could cause operators to think their server is more permissive than it actually is.

### S4. Metrics endpoint uses plaintext Basic Auth (LOW)
- **Location:** `snip-sync/src/main.rs:1034-1045`
- The metrics endpoint authenticates via HTTP Basic Auth with `subtle::ConstantTimeEq` (good), but credentials are stored in plaintext in the config file or environment variables. This is standard practice but worth noting.

## Performance Issues

### P1. Argon2 with 64 KiB memory cost is fast but weak
- **Location:** `src/encryption.rs:32-34`
- With only 64 KiB memory cost, 3 iterations, and parallelism 4, key derivation is very fast but provides minimal resistance to GPU/ASIC attacks. For a security-focused tool with E2E encryption, this is a tradeoff favoring speed over security.

### P2. `merge_snippets` uses O(n*m) algorithm for duplicate detection
- **Location:** `src/sync_commands.rs:375-456`
- The function builds a HashMap for local snippets (good), but the outer loop iterates server snippets and does lookups against the map. For each server snippet, it does O(1) HashMap lookups. The inner loop for "local-only" snippets at line 444-448 iterates all local snippets and checks a HashSet. This is O(n+m) overall, which is fine.

### P3. `snip-sync` server `sync` endpoint processes snippets sequentially
- **Location:** `snip-sync/src/main.rs:583-610`
- The for-loop at line 596 processes snippets one-by-one with individual `upsert_snippet` calls. For large sync operations, this could be slow. A batch upsert would be more efficient.

## Priority Ranking

| Issue | Severity | Description |
|-------|----------|-------------|
| S1 | high | Argon2 memory cost 64 KiB is 300-700x below OWASP minimum (19-46 MiB) |
| B1 | medium | `list_libraries` gRPC endpoint skips rate limiting |
| B2 | medium | `get_snippets` gRPC endpoint skips rate limiting |
| S2 | medium | Server has no TLS by default; client expects TLS — deployment friction |
| D1 | medium | Mixed TOML field casing (`Id`/`Description` vs `folders`/`favorite`) confuses users |
| B3 | low | Audit log failures invisible in production (debug-level only) |
| S3 | low | CORS warning message is factually incorrect |
| S4 | low | Metrics Basic Auth credentials in plaintext config |
| D4 | low | Server `sync` response always reports `skipped_count: 0` even when snippets are skipped |
| P3 | low | Server processes sync snippets sequentially, not in batch |

## Recommendations

1. **Increase Argon2 memory cost** to at least 64 MiB (`1 << 16`). This is the single most impactful security improvement. The current 64 KiB is dangerously low for a tool that provides end-to-end encryption as a core feature.

2. **Add rate limiting to `list_libraries` and `get_snippets`** endpoints to match all other endpoints.

3. **Fix CORS warning message** to accurately state that requests from any origin will be blocked (not allowed) when no origins are configured.

4. **Normalize TOML field casing** — consider a migration path to consistent lowercase field names, or document the mixed casing as a deliberate compatibility choice.

5. **Clarify the "12 commands" claim** — either say "13 subcommands" or clarify that `version` is a built-in informational command not backed by a separate module.

6. **Address the pet format claim** — either verify compatibility with a test or remove the claim from the architecture doc.

7. **Consider batch upsert** in the server sync endpoint for better performance with large sync operations.

8. **Return accurate `skipped_count` and `skipped_ids`** in the sync response when snippets fail validation.
