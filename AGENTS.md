# AGENTS.md

## Phase 13A — Server Lifetime and Configuration Correctness

- The server runs indefinitely until a process shutdown signal or unexpected service failure; normal operation has no arbitrary lifetime timeout.
- Both services share a single broadcast shutdown signal; only the graceful drain phase after shutdown is bounded (default 30s).
- On Unix, the server registers both `tokio::signal::ctrl_c()` and `tokio::signal::unix::SignalKind::terminate()` so `snip-sync stop` triggers the same graceful shutdown path as Ctrl-C.
- gRPC uses Tonic's `serve_with_incoming_shutdown` for connection-aware draining; HTTP uses `axum::serve().with_graceful_shutdown`.
- Both service task handles remain owned by the orchestrator and are awaited inside the real drain timeout.
- Unexpected service/task failure notifies the sibling, drains, and returns an error — it is never swallowed into a log-only success.
- Persistence shutdown occurs only after both request-serving tasks have completed or been aborted.
- Environment variable overrides are strictly parsed via `parse_env` and `parse_bool_env`; present but invalid values cause startup to fail.
- Boolean env vars (`TLS_ENABLED`, `SNIP_SYNC_ALLOW_HTTP`, `CORS_ALLOW_ALL`, `PERSIST_RATE_LIMITS`) accept case-insensitive `true`/`1`/`yes`/`on` and `false`/`0`/`no`/`off`; unknown values fail.
- Range validation rejects zero ports, zero connection limits, zero timeouts, and zero values for `MAX_ID_LENGTH`, `MAX_DEVICE_ID_LENGTH`, `MAX_API_KEY_LENGTH`, and `RATE_LIMIT_PER_MINUTE` after env/file/default resolution.

## Phase 13B — Bounded Sync Uploads and Clock-Skew Diagnostics

- Sync uploads are byte-bounded using Prost `encoded_len()` to measure actual request size before transmission. The client ceiling defaults to 3.5 MiB (below the server's 4 MiB gRPC limit).
- `build_upload_batches()` splits encrypted snippets into deterministic ID-sorted batches that fit within the ceiling. Each batch is measured against `SyncRequest` (the larger envelope); batches that fit `SyncRequest` also fit `PushSnippetsRequest`. A single oversized item fails before any remote mutation. After an overflow split, the new singleton is immediately re-validated.
- For one upload batch: `Sync(batch, offset=0)` carries the upload and returns the first response page. For two or more batches: each batch goes via `PushSnippets` (upload only), then an empty-upload `Sync(offset=0)` fetches the authoritative first response page. Response pages are paginated after all uploads complete.
- `PushSnippets` is idempotent by snippet identity — retrying an already-accepted batch is safe (server upserts use `ON CONFLICT ... WHERE newer`).
- Server clock-skew rejection now reports the skew magnitude and corrective action (e.g., "updated_at is 742 seconds ahead of server time; synchronize the client clock and retry").
- `InvalidArgument` gRPC errors containing timestamp-related messages map to `SyncFailureKind::ClockSkew` → `FailureClass::Configuration`.
- `SyncFailureKind::RequestTooLarge` maps to `FailureClass::Configuration` (requires operator attention).
- `sync_encrypted()` accumulates response pages via `accumulate_page()` helper to avoid variable lifecycle warnings.

## Phase 13H — Final Correctness Closure (with corrective follow-up)

- `sync_encrypted` and `sync_encrypted_with_ceiling` delegate to a single `sync_encrypted_inner` implementation; zero batches is a valid pull-only path, not an `unreachable!` panic.
- Multi-batch `PushSnippets` errors preserve the original `SyncFailureKind` (e.g., `ClockSkew`, `Timeout`) via `add_batch_context()` instead of flattening to `SyncRequestFailed`.
- Server shutdown orchestration uses a shared `run_services_until_shutdown()` helper called by both `serve_inner` and deterministic tests; the first terminal event is captured explicitly, a completed handle is never polled twice, and refusing tasks are explicitly aborted and awaited.
- Process lifetime tests use `SNIP_SYNC_STATE_DIR` for test isolation, `start_server_on_ports()` for same-port restart, and `wait_for_exit()` for bounded child waits.
- Partial-failure convergence test retains server state across crash/retry via file-based SQLite.
- `state_dir()` supports `SNIP_SYNC_STATE_DIR` env var override for test isolation.

## Phase 13I — Drain Result Accounting and Deterministic Regression Closure

- Orchestration uses explicit `grpc_consumed`/`http_consumed` booleans to track per-service handle lifecycle; a consumed handle is never awaited or aborted again.
- Drain updates completion state immediately when a service finishes during the bounded drain window; Phase 3 only aborts handles still marked pending.
- A requested shutdown fails if either service returns an error or panics during drain; only a clean dual-service exit without forced abort is success.
- Push failure injection uses `push_fail_after` (threshold) and `push_fail_counter` (atomic counter) on `SnipSyncService`; counter starts at 0, increments per push, and rejects when count ≥ threshold.
- `encrypt_snippets_with()` extracts the encryption loop for test-only failure injection.
- Deterministic retained-state convergence: push failure on Nth batch proves partial mutation, retry against same file DB converges exactly once.
- Zero-batch regressions: empty-local/empty-remote, pull-only seeded remote, multi-page pagination with small sync_limit.
- All-encryption-failed accounting: skipped IDs/counts preserved, remote snippets still returned.
- Typed batch-context tests: `ClockSkew` and `Timeout` retain kind/classification and original detail through `add_batch_context()`.

## Phase 13J — Production Outcome Wiring and Test-Seam Closure

- `serve_inner` consumes the same `ensure_clean_requested_shutdown()` decision method exercised by orchestration unit tests. Requested shutdown with a service error, panic, or forced abort now returns failure after persistence cleanup; the previous boolean-only check (`requested && !forced`) was incomplete.
- Both service classifications and the original error/panic detail are retained in the production failure diagnostic.
- Exactly one method (`sync_prepared_encrypted_inner`) owns the zero/one/many batch transport logic. `sync_encrypted` and `sync_encrypted_with_ceiling` delegate to it. `sync_encrypted_inner` is now a thin wrapper that runs real encryption and then delegates.
- The custom-encryption test seam is private and `#[cfg(test)]` only — `sync_encrypted_with_test_encrypt` lives inside the unit-test module and drives the same prepared transport. The previous public `sync_encrypted_with_custom_encrypt` was removed.
- `add_batch_context` is private; its typed-error preservation tests live in the `src/sync.rs` unit-test module alongside the helper.
- Requested-shutdown orchestration tests use `std::future::ready(())` (or a held-pending oneshot channel) for the signal future so the helper itself is the only sender on the broadcast shutdown channel.
- The `no_pre_signal_lifetime_timeout` test constructs the helper with a held-pending oneshot signal and asserts via `tokio::time::timeout` that the orchestration future does not complete within 2× the drain timeout. It then triggers the signal and verifies a clean requested shutdown.

## Phase 13D — Client Runtime and Dependency Footprint Reduction

- Bundled themes use gzip compression (via `flate2`); `lzma-rs` has been removed. Regenerate with `python3 scripts/build_themes.py`.
- Update archives use `.tar.gz` for all platforms including Windows; the `zip` crate has been removed. `extract_zip` and `validate_zip_entry_path` are gone.
- Local-only commands (`select`, `list`, `get`, `validate`, `backup`, `new`, `edit`, `keybindings`, `completions`, `shell`, `doctor`, `status`, `repair`, `restore`, `import`) do not initialize the Tokio runtime. The `RUNTIME` lazy static is only accessed when `--sync` is requested or for explicit sync/register/premade commands.
- `run_snippet_selection` accepts `Option<&tokio::runtime::Runtime>` — pass `None` when `do_sync` is false, `Some(&RUNTIME)` when true.
- The auto-sync detached helper uses `Builder::new_current_thread()` instead of `new_multi_thread()`.
- `chrono` default features are pruned to `clock` and `std` only (no `wasmbind`, `oldtime`).
- Release profile includes `panic = "abort"` for smaller binaries (~19% reduction from baseline).

## Phase 13E — Auto-Sync and Persistence Simplification

- Worker acquisition is the sole sync execution authority. The scheduler does not probe the execution lock before spawn; concurrent helpers exit cheaply when the lock is held.
- `FailureClass` collapsed from 11 variants to 4: `Transient` (network/timeout/partial), `Configuration` (auth/config/credential), `LocalFailure` (persistence/conflict/corruption), `Internal` (unclassified). Legacy status codes are read compatibly via `from_code()`.
- Configuration/authentication failures defer until config change or explicit retry (`WaitForConfigurationChange`); transient failures retry with exponential backoff; local failures require repair; internal errors retry bounded (3 attempts).
- `auto_sync::lock::WorkerLock` is re-exported from `execution_lock.rs` for backward compatibility; the worker lock types were merged into `execution_lock.rs`.
- `ScheduleError::ExecutionLock` and `ScheduleDecision::AlreadyActive` were removed — the scheduler no longer probes the execution lock.
- `WorkerLockError` replaces `LockError` in the worker lock module.
- `worker_lifetime` and `max_delay` merged into a single `max_lifetime` field on `AutoSyncPolicy`; the `DeferredMaximumLifetime` debounce variant is removed.
- Transaction state `BackupsDurable` retained for backward-compatible recovery of old journals; new transactions use `Prepared` → `Committing` → `CleaningUp` directly.
- `advance_to_committed_local` retained for backward-compatible recovery of old `CommittedLocal` journals; new transactions record pending after commit without using the transaction state machine.
- Auto-sync `spawn.rs` merged into `execution_lock.rs`; `auto_sync::spawn` module removed.
- Status file now rejects unknown schema versions (future-version files return `StatusRead::Corrupt`).

## Phase 13F — API, CLI, Server, and Documentation Surface Consolidation

- Implementation-only modules (`auto_sync`, `commands`, `logging`, `process_file_lock`, `proto`, `selector`, `sync`, `ui`, `usage`) are `#[doc(hidden)]` in `lib.rs`. They remain `pub` for binary and integration-test crate access but are not part of the supported external API.
- Root-level TUI types (`SnippetData`, `ProcessResult`, `CommandOutcome`, `SelectionOutcome`) are `#[doc(hidden)]`.
- The supported Rust API is: `Snippet`, `Snippets`, `LibraryConfig`, `LibraryMeta`, `load_library`, `save_library`, `AtomicWriteOptions`, `AtomicWriteReport`, `Durability`, `atomic_replace`, `write_private_atomic`, `SnipError`, `SnipResult`, `SnippetSort`, `SortOptions`, `rank_snippets`, `SyncSettings`, `SyncDirection`, `AutoSyncFailureMode`, `CliOutcome`, `exit_code::*`, `OutputContext`.
- The `data` subcommand group (`snp data validate|backup|restore|repair|status`) is the canonical home for advanced data maintenance. Legacy top-level spellings (`snp validate`, `snp backup`, `snp restore`, `snp repair`, `snp status`) remain as compatibility aliases with identical exit codes and output.
- `FailureClass` in `architecture/sync.md` corrected to 4-variant enum (Phase 13E collapse).
- Architecture docs: LZMA → gzip for bundled themes, merge strategy corrected to no-resurrection behavior.

## Phase 14A — Credential Backend and Explicit-Sync Correctness

- `Cargo.toml` enables native keyring store features: `apple-native` (macOS), `windows-native` (Windows), `sync-secret-service` (Linux desktop). Without a supported store feature, keyring uses its mock store as default — not acceptable as production credential persistence.
- `run_explicit_sync(runtime)` in `commands/mod.rs` is the single canonical explicit-sync implementation shared by TUI `--sync` paths and exact-selector `--sync` paths. It acquires the execution lock, observes pending generation, runs `run_default_sync`, and clears pending on success.
- Exact `run --id/--description-exact/--command-exact --sync` now calls `run_explicit_sync` instead of `notify_mutation(SnippetRun)`. Running a snippet changes local usage metadata, not synced content; `--sync` means perform sync now.
- Exact `clip --id/--description-exact/--command-exact --sync` now accepts `do_sync` and `runtime` parameters and calls `run_explicit_sync`. The flag is no longer discarded by exact dispatch.
- `clip_cmd::run_exact` signature changed from `fn run_exact(snippet)` to `fn run_exact(snippet, do_sync, runtime)`. All call sites updated.
- TUI delete path in `run_snippet_selection` also uses `run_explicit_sync` instead of inline lock/sync/clear logic.

## Phase 14B — Persistence Fail-Closed Behavior and Stable Snippet Identity

- Malformed library TOML (`load_library()`) now fails closed: best-effort backup + `SnipError` return, never synthesized empty writable library.
- Malformed `libraries.toml` (`LibraryManager::new()`) now fails closed: best-effort backup + `SnipError` return, never default config.
- `commands::load_snippets()` already failed closed; all three persistence entry points now have a consistent rule: missing/empty → valid default; malformed → backup + error.
- Legacy snippet ID repair uses deterministic SHA-256 normalization instead of `uuid::Uuid::new_v4()`. Missing IDs get `legacy-<sha256hex>` from domain-separated content fingerprint. Duplicate IDs get deterministic replacements.
- Deterministic IDs are stable across repeated read-only loads. The next `save_library()` persists them naturally through the existing save path.
- No new dependencies added; `sha2` was already present. `uuid` crate retained for non-snippet-ID uses (temp files, locks, journal filenames).
- Snippet IDs are opaque strings throughout the codebase — no production path requires UUID syntax. Server validates only length (`max_id_length: 128`), not format.

## Phase 14C — Command and Control-Flow Consolidation

- Exact selector construction is canonicalized via `resolve_exact_target()` in `selector.rs`. Run, clip, and edit exact paths all delegate to this single helper.
- Clipboard copy side effects (audit log, usage index update) are canonicalized via `copy_to_clipboard()` in `clip_cmd.rs`. Both TUI callback and exact command path use it.
- Run post-execution bookkeeping (audit/usage/tracing) is consolidated into `record_execution_result()` in `run_cmd.rs`, called by both the output-file and normal-execution branches.
- Legacy and canonical data command dispatch share one repair exit-mapping helper (`exit_on_repair_status()` in `main.rs`).
- Startup recovery and logging/audit classification come from one match via `command_behavior()` in `main.rs`, which returns a `CommandBehavior` struct containing both `StartupRecoveryPolicy` and `StartupServices`.
- `StartupRecoveryPolicy` has five variants: `Allow`, `SuppressReadOnly`, `SuppressExplicitSync`, `SuppressInternal`, `SuppressConfiguration`.
- The obsolete `SubcommandTag` enum and tag-based `should_attempt_auto_sync_recovery()` have been removed; only the policy-based `should_attempt_auto_sync_recovery_for_policy()` remains.
- Explicit-sync orchestration uses a single `run_explicit_sync()` in `commands/mod.rs` for all paths (TUI delete, post-selection, exact run, exact clip).

## Phase 14D — Dependency and Binary Footprint Reduction

- `arboard` uses `default-features = false` to drop `image-data`; snip-it only uses text clipboard operations (`set_text`).
- Root `tonic` uses `default-features = false, features = ["codegen", "channel", "tls-ring"]`; `snip-proto` uses `default-features = false, features = ["codegen", "channel"]`. The client no longer pulls `router` (axum), `transport` (server), `h2`, or `socket2`. Server features remain intact via `snip-sync`.
- `tracing-subscriber` uses `default-features = false, features = ["fmt", "registry", "env-filter"]`; the `ansi` feature (nu-ansi-term) is dropped since file logs use `with_ansi(false)`.
- The client retains `tokio`'s `rt-multi-thread` feature because the production detached auto-sync worker creates its own multi-thread runtime.
- `tar` and `flate2` remain direct dependencies; `flate2` is also needed for bundled theme gzip decompression.
- Self-update archive removal (raw asset) was evaluated and deferred — release pipeline not visible in repository; not a net simplification.
- Total binary delta: -33,584 bytes (3,922,224 → 3,888,640) on macOS aarch64 release build.

## Phase 14E — Runtime and Internal Simplification

- `notify_mutation()` resolves `AutoSyncPolicy` once and passes the snapshot into `schedule_after_record()` instead of reloading config a second time.
- Pending-marker and status-file writes use the canonical `atomic_write_bytes()` from `utils/atomic.rs` (`DurableUserData` durability) instead of a duplicate platform-specific `atomic_write_unique()` in `pending_lock.rs`. The duplicate `unique_temp_path()`, `atomic_write_unique()`, `replace_existing()`, and `fsync_parent_dir()` functions have been removed from `pending_lock.rs`.
- Audit logging is synchronous: `audit_log()` calls `write_audit_log_entry_sync()` directly. The async channel (`AUDIT_TX`), bounded `sync_channel`, dedicated `AuditLogWriter` thread, and `init_async_audit_log()` have been removed.
- `StartupServices::LoggingAndAudit` collapsed to `StartupServices::Logging` since audit initialization no longer starts a background thread. `init_default_logging()` is now equivalent to `init_default_file_logging()`.
- No new dependencies added; no sync/pending/lock invariant changes.

## Phase 14F — Verification and CI Reduction

- Linux is the sole broad correctness lane; macOS/Windows CI no longer runs `cargo test --workspace --lib`.
- `manifest_contracts` moved from `scripts/check.sh` to `scripts/release-check.sh verify` (release-time only).
- `destination_permissions` remains in `scripts/check.sh` (proves production filesystem permission behavior via subprocess).
- Low-information unit tests consolidated: 17 individual tests in `notification.rs`, `policy.rs`, `config.rs`, and `outcome.rs` replaced with table-driven equivalents.
- `snip_sync_lifetime.rs` retained with two distinct cases (long-lived health + SIGTERM/same-port restart); no repeated 5/5 ceremony.

## Phase 12B Auto-Sync Correctness Closure

- `schedule_sync`, `schedule_and_spawn`, and `schedule_existing_pending` return typed local scheduling errors. Pending-read and worker-spawn failures must never be collapsed into `NoPending`, `SpawnNow`, or a successful notification.
- Startup recovery and scheduling use the kernel-backed execution lock as the sole ownership authority. Persistent PID/nonce metadata is diagnostic only.
- Pending generations are monotonic. A lower generation observed during debounce or preflight is corrupt/inconsistent state: preserve the marker, log the failure, and do not spawn sync work.
- The detached helper runs canonical sync directly; network/request retry bounds remain in the sync client and no child executor is supervised.

## Build & Test Commands

```bash
# Focused developer verification (same as Linux CI) — fmt, clippy, unit tests, selected integration tests
bash scripts/check.sh

# Manual pre-release verification (requires clean working tree)
bash scripts/release-check.sh verify

# Per-crate publish dry-run (manual)
bash scripts/release-check.sh dry-run snip-it

# Production seam proof — verifies test-only env vars are inactive in production builds
bash scripts/ci/test-production-seams.sh

# Build the workspace
cargo build --workspace
cargo build --release

# Lint (warnings are errors) — use --all-targets, NOT --all-features;
# test-support and test-helpers are enabled only for specific test targets
cargo clippy --workspace --all-targets -- -D warnings

# Format check / auto-format
cargo fmt --all -- --check
cargo fmt
```

### Running Tests

```bash
# Unit tests only (parallel — each test uses isolated TempDir)
cargo test --workspace --lib

# All tests including integration (serial — for migration checks only)
cargo test --workspace --all-features -- --test-threads=1

# snip-sync tests (needs test-helpers feature)
cargo test -p snip-sync --features test-helpers
```

**Key gotcha:** `cargo test --lib -p snip-it` does not work — `snip-it` is binary-only. Use `cargo test -p snip-it` or `cargo test --workspace`.

**Key gotcha:** Only 3 integration tests require `--features test-support` to compile: `repair_transactions`, `process_lock_concurrency`, and `local_data_lock_barriers` (they use `#[cfg(feature = "test-support")]` gated code). All other integration tests compile without the feature.

**Key gotcha:** PTY tests (`pty_integration.rs`) use real terminal pairs — always pass `--test-threads=1`.

## Toolchain

- **Rust 1.94**, edition 2024 (not 2021). See `rust-toolchain.toml`.
- `rustfmt.toml`: `max_width=100`, 4-space indent, Unix newlines, `edition = "2024"`.

## Project Structure

```
snip-it/          Main crate — binary "snp" (src/main.rs)
snip-proto/       Protobuf definitions, tonic-generated gRPC code
snip-sync/        Sync server (gRPC + HTTP/axum)
tests/            Integration tests (~46 files, see below)
scripts/          check.sh, release-check.sh, ci/ helpers
themes/           50 Halloy TOML theme files
```

### Key Source Modules (`src/`)

- `main.rs` — CLI entry point, clap dispatch
- `lib.rs` — Library crate (exports for integration tests)
- `commands/` — 24 files (23 command modules + shared helpers in `mod.rs`)
- `auto_sync/` — Auto-sync subsystem (execution_lock, mod, notification, pending, pending_lock, policy, schedule, status, test_events, worker)
- `ui/` — TUI (ratatui + crossterm), theme system, syntax highlighting
- `utils/` — Config paths, TOML helpers, atomic writes (`atomic.rs`)
- `library.rs` — Snippet/library data structures and TOML persistence
- `sync.rs` — gRPC client for snip-sync server
- `sync_commands.rs` — Sync orchestration and merge logic
- `encryption.rs` — AES-256-GCM + Argon2id end-to-end encryption
- `config.rs` — Sync settings, path resolution, keychain API key
- `error.rs` — `SnipError` enum, `SnipResult<T>`, `SyncFailureKind`
- `selector.rs` — Shared snippet selector model (`SnippetSelector`, `ResolutionPolicy`)
- `outcome.rs` — CLI outcome types and exit-code mapping (`CliOutcome`)
- `sort.rs` — Sort modes, ranking, `SnippetSort`
- `usage.rs` — Local usage metadata (`UsageIndex`)
- `process_file_lock.rs` — Kernel-backed cross-process file lock (`flock`/`LockFileEx`)
- `logging.rs` — Structured logging and audit trail
- `transaction.rs` — Transaction boundary with journal, lock, begin/commit/rollback
- `migration.rs` — Schema versioning (`SchemaVersion` ordinal type)
- `clipboard.rs` — Cross-platform clipboard integration
- `diagnostics.rs` — Internal diagnostics
- `local_data.rs` — Local data lock coordination
- `output.rs` — Output file handling
- `proto.rs` — Protobuf type re-exports
- `status_snapshot.rs` — Status snapshot and diagnostic codes
- `update.rs` — Update checking
- `test_failpoints.rs` — Test-only failpoint hooks (compiled with `test-support`)

## Critical Gotchas

### Generated code
`src/ui/_generated_bundled_themes.rs` is generated at build time by `scripts/build_themes.py` (invoked from `build.rs`). Never edit it directly.

### TOML backslash escape handling
The save path does NOT post-process `toml::to_string_pretty` output. The golden command corpus includes tabs, trailing spaces, and CRLF that must survive the full save/load pipeline. See `src/utils/toml_helpers.rs`.

### Single-helper execution lock
The detached `auto-sync-worker` holds `SyncExecutionLock` for the entire bounded cycle and runs `sync_commands::run_sync` directly. Manual sync and cron acquire the same lock.

### AGENTS.override.md
Contains session-specific pitfall notes and plan review findings. Consult it for implementation guidance.

### Kernel-backed process file locks
All auto-sync locks and the `snip-sync` server singleton use `flock` (Unix) / `LockFileEx` (Windows). The kernel alone is authoritative — persistent lock files may contain stale metadata. `Drop` releases the lock without unlinking the file.

Linux process start tokens use `/proc/<pid>/stat` field 22 (`starttime`), and
Unix `kill(pid, 0)` probes treat `EPERM` and unknown errors as a live process;
only `ESRCH` proves absence. The hidden auto-sync worker maps `Failed` to the
existing general-error exit code while `Success` and `NothingToDo` remain zero.

### Mutation gate
`gate_mutation_on_interrupted_transactions()` must be called before any local mutating operation. Single journal = auto-rollback; multiple/incomplete = refuse and direct to `snp repair`.

### Deterministic test assertions
Tests must use exact counts (not `>= 1`), prove server-side state effects, and verify pending clear ordering. Auto-sync closure cases live in `tests/auto_sync_closure.rs`; sync-boundary cases live in `tests/sync_integration.rs` and `tests/sync_contracts.rs`.

### Test event emission
The helper emits lifecycle events when `SNP_TEST_EVENTS_DIR` is set (JSON-lines). See `tests/support/event_sink.rs` (test-side) and `src/auto_sync/test_events.rs` (production).

### No command filtering (by design)
Snippet commands execute as-is — no sanitization. Intentional for power users.

## Key Architecture Notes

### Auto-Sync (single detached helper)
- Detached worker (`snp auto-sync-worker`) runs the canonical sync operation directly
- Parent never holds the worker lock
- All sync operations acquire `SyncExecutionLock` to prevent concurrent sync
- Local mutations always commit before remote work; failed sync never rolls back local state
- `schedule_sync()` is the sole scheduling authority
- Module: `src/auto_sync/`

### Error Handling
- `SnipError` enum (`src/error.rs`), `SnipResult<T> = Result<T, SnipError>`
- `SnipError` never carries credentials or API keys

### Async (Tokio)
- Global `RUNTIME: LazyLock<Runtime>` — only initialized by async commands (`run`, `clip`, `search`, `sync`, `register`, `premade`)
- Sync operations use `runtime.block_on()` for async gRPC calls

### Client footprint (Phase 12D)
- Release profile uses `opt-level = "z"`, selected from a controlled native-platform measurement; keep it as one simple profile, not a CI optimization matrix.
- `snp` parses the command before initializing file logging or the audit writer. Minimal read-only/configuration-output commands must not create `logs/`, `snp.log`, `audit.log`, or `.self_check`.
- The server's request observer is compiled only with tests or the explicit `test-helpers` feature. Do not add production branches for test event capture.
- The client retains Tokio's multi-thread feature because the production detached auto-sync worker creates its own multi-thread runtime; do not prune it without redesigning that supported path.

### Sync ordering and recovery (Phase 12E)
- Live snippet conflicts use `(updated_at, device_id, SHA-256(synced fields))`; never reintroduce role-dependent `>=` server-wins behavior.
- Deletion wins over live content, including when the live copy has a later timestamp. This is intentional no-resurrection behavior, not pure LWW.
- `output`, `folders`, and `favorite` are local-only and must not enter the conflict fingerprint.
- Missing-library recovery uses atomic `<library>.sync_recovery` TOML state. Preserve corrupt markers, reuse exactly one normalized remote-name match, fail on ambiguity, and remove a marker only after relink and retry sync are durable.
- Recovery linkage and `last_sync` reset must be persisted in one `LibraryManager` save.

### Selection & Exit Codes
- `SnippetSelection` (TUI) → `SelectionOutcome` (lib) → `CommandOutcome` (commands)
- Cancellation maps to exit code 4 for `select`; `run`/`clip`/`search` treat cancellation as exit 0
- Output-file execution failures (timeout/spawn) map to exit code 8

### Output Field
- `output` is local-only — not synced, not in `ProtoSnippet`
- `snp edit --output` requires `--filter`

## Configuration Files

- `~/.config/snp/snippets.toml` — main storage (or per-library in `libraries/`)
- `~/.config/snp/sync.toml` — sync settings
- `~/.config/snp/libraries.toml` — library metadata
- `~/.config/snp/libraries/*.toml` — individual library files
- `~/.config/snp/premade/*.toml` — downloaded premade libraries
- `~/.config/snp/themes/*.toml` — Halloy-compatible theme files
- `~/.config/snp/themes.toml` — active theme selection
- `~/.config/snp/usage.toml` — local usage metadata (not synced)
- `~/.config/snp/auto-sync-status.toml` — durable sync status (not synced, private)
- `~/.config/snp/auto-sync-pending.toml` — pending mutation marker
- `~/.config/snp/transaction-journals/` — transaction journals
- `~/.config/snp/backups/` — backup snapshots

## Testing Notes

- Integration tests use `TempDir` with `XDG_CONFIG_HOME` env override
- Server tests use `sqlite::memory:` for isolation
- `tests/support/` provides reusable infrastructure: `TestEnvironment`, `RecordingServer`, `EventSink`
- Tests never use the developer's real config, keychain, or ports
- `SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on all test commands
- Golden command corpus: 24 edge cases verifying exact-text preservation

### Test Classification

| Class | Execution | Targets |
|-------|-----------|---------|
| Unit/pure | parallel | `cargo test --workspace --lib` — parsing, sorting, batching, serialization |
| CLI/platform smoke | parallel | `platform_smoke.rs`, `local_contracts.rs` — real binary, isolated TempDir |
| Restore contracts | parallel | `destination_permissions.rs`, `backup_contracts.rs` |
| Auto-sync contracts | parallel | `auto_sync_closure.rs`, `sync_contracts.rs`, `debounce_matrix.rs` |
| Sync integration | serial target | `sync_integration.rs` — in-process server, random port |
| PTY | serial target | `pty_integration.rs` — real terminal pairs |
| Cross-process lock | serial target | `process_lock_concurrency.rs` — kernel flock, real subprocesses |
| Barrier-coordinated | serial target | `local_data_lock_barriers.rs`, `repair_transactions.rs` — `set_var`, barrier protocol |
| Deep recovery | manual/release | `transaction_crash_recovery.rs`, `cleanup_crash_failpoints.rs`, `restore_crash_failpoints.rs` |
| Release smoke | manual/release | `release-check.sh` Phase 3 — version/help, crash recovery, production seams, `manifest_contracts.rs` |
| Architecture | parallel | `architecture.rs` — source-scanning layer boundary enforcement |

## Reference Docs

- `architecture/` — deep-dive docs per module (see index below)
- `docs/` — public API, threat model, security audit, supply-chain policy
- `.skills/` — specialized agent reference docs (see below)

### Skills Index

| Skill | File | Key Content |
|-------|------|-------------|
| Architecture review | `.skills/architecture-review.md` | Review process, key files, verification checklists |
| Encryption | `.skills/encryption-module.md` | AES-256-GCM + Argon2id, key cache, payload format, security properties |
| Keychain | `.skills/keychain-integration.md` | OS keychain storage pattern, migration, platform notes |
| Remediation | `.skills/remediation-patterns.md` | Atomic writes, transactions, durability classes, repair patterns |
| Server | `.skills/server-module.md` | snip-sync server architecture, env vars, gRPC endpoints |
| Sync | `.skills/sync-module.md` | Sync flow, merge strategy, failure classification, recovery commands |
| UI | `.skills/ui-module.md` | TUI module structure, theme system, syntax highlighting |

### Architecture Index

| Topic | File | Key Content |
|-------|------|-------------|
| Bird's-eye view | `architecture/overview.md` | Module map, data flow, configuration files |
| Auto-sync | `architecture/auto_sync.md` | Two-process model, debounce, scheduling, backoff |
| Sync protocol | `architecture/sync.md` | Merge strategy, encryption, conflict resolution |
| Persistence | `architecture/persistence.md` | Atomic writes, transactions, validation, backup/restore |
| Server | `architecture/server.md` | gRPC/HTTP endpoints, database schema, rate limiting |
| TUI | `architecture/tui.md` | Keybindings, state machine, interaction model |
| CLI | `architecture/cli.md` | Entry point, argument parsing, dispatch |
| Outcome/exit codes | `architecture/outcome.md` | `CliOutcome` variants, stable exit codes |
| Selector | `architecture/selector.md` | Deterministic non-TUI snippet resolution |
| Sort/ranking | `architecture/sort.md` | Sort modes, tie-break chain, `--favorites-first` |
| Status | `architecture/status.md` | Status snapshot, diagnostic codes, `snp status` |
| Encryption | `architecture/encryption.md` | AES-256-GCM + Argon2id, key cache, payload format |
| Config | `architecture/config.md` | `SyncSettings`, `SyncDirection`, `AutoSyncFailureMode` |
| Core data model | `architecture/core.md` | `Snippet`, `Snippets`, `SnipError`, `LibraryManager` |
| Library | `architecture/library.md` | Library CRUD, TOML persistence, file layout |
| Logging | `architecture/logging.md` | Structured tracing, audit log, initialization |
| Protobuf | `architecture/proto.md` | `SnippetSync` service, message types, code generation |
| Clipboard | `architecture/clipboard.md` | Platform support, auto-clear, generation counter |
| Usage | `architecture/usage.md` | `UsageIndex`, usage tracking, update policy |
| Output | `architecture/output.md` | `OutputPresentation`, terminal display, security |
| Utilities | `architecture/utils.md` | Config paths, variables, TOML helpers, shell keywords |
| UI modules | `architecture/ui.md` | TUI module details, mode system, rendering |
| Test infra | `architecture/test-infrastructure.md` | Deterministic E2E, event sink, temp-dir isolation |
