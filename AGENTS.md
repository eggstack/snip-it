# AGENTS.md

## Phase 13A — Server Lifetime and Configuration Correctness

- The server runs indefinitely until a process shutdown signal or unexpected service failure; normal operation has no arbitrary lifetime timeout.
- Both services share a single `broadcast` shutdown signal; only the graceful drain phase after shutdown is bounded (default 30s).
- Unexpected service/task failure notifies the sibling, drains, and returns an error — it is never swallowed into a log-only success.
- Environment variable overrides are strictly parsed via `parse_env` and `parse_bool_env`; present but invalid values cause startup to fail.
- Boolean env vars (`TLS_ENABLED`, `SNIP_SYNC_ALLOW_HTTP`, `CORS_ALLOW_ALL`, `PERSIST_RATE_LIMITS`) accept case-insensitive `true`/`1`/`yes`/`on` and `false`/`0`/`no`/`off`; unknown values fail.
- Range validation rejects zero ports, zero connection limits, and zero timeouts after env/file/default resolution.

## Phase 13B — Bounded Sync Uploads and Clock-Skew Diagnostics

- Sync uploads are byte-bounded using Prost `encoded_len()` to measure actual request size before transmission. The client ceiling defaults to 3.5 MiB (below the server's 4 MiB gRPC limit).
- `build_upload_batches()` splits encrypted snippets into deterministic ID-sorted batches that fit within the ceiling. A single oversized item fails before any remote mutation.
- The first upload batch goes via `Sync` RPC (upload + first response page); subsequent batches go via `PushSnippets` RPC (upload only). Response pages are paginated after all uploads complete.
- `PushSnippets` is idempotent by snippet identity — retrying an already-accepted batch is safe (server upserts use `ON CONFLICT ... WHERE newer`).
- Server clock-skew rejection now reports the skew magnitude and corrective action (e.g., "updated_at is 742 seconds ahead of server time; synchronize the client clock and retry").
- `InvalidArgument` gRPC errors containing timestamp-related messages map to `SyncFailureKind::ClockSkew` → `FailureClass::Configuration`.
- `SyncFailureKind::RequestTooLarge` maps to `FailureClass::Configuration` (requires operator attention).
- `sync_encrypted()` accumulates response pages via `accumulate_page()` helper to avoid variable lifecycle warnings.

## Phase 12B Auto-Sync Correctness Closure

- `schedule_sync`, `schedule_and_spawn`, and `schedule_existing_pending` return typed local scheduling errors. Pending-read, execution-lock, and worker-spawn failures must never be collapsed into `NoPending`, `SpawnNow`, or a successful notification.
- Startup recovery and scheduling use the kernel-backed execution lock as the sole ownership authority. Persistent PID/nonce metadata is diagnostic only.
- Pending generations are monotonic. A lower generation observed during debounce or preflight is corrupt/inconsistent state: preserve the marker, log the failure, and do not spawn sync work.
- The detached helper runs canonical sync directly; network/request retry bounds remain in the sync client and no child executor is supervised.

## Build & Test Commands

```bash
# Focused developer verification (same as Linux CI) — fmt, clippy, build, unit tests, selected integration tests
bash scripts/check.sh

# Exhaustive pre-release verification (requires clean working tree)
bash scripts/release-check.sh verify

# Production seam proof — verifies test-only env vars are inactive in production builds (no test-support)
bash scripts/ci/test-production-seams.sh

# Build the workspace
cargo build --workspace
cargo build --release

# Lint (warnings are errors)
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Format check / auto-format
cargo fmt --all -- --check
cargo fmt
```

### Running Tests

```bash
# All tests (unit + integration + server, single-threaded)
cargo test --workspace --all-features -- --test-threads=1

# Unit tests only
cargo test --workspace --all-features --lib -- --test-threads=1

# Single focused auto-sync contract test by name
cargo test --test auto_sync_closure --features test-support -- --test-threads=1

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
tests/            Integration tests (~50 files, see below)
scripts/          check.sh, release-check.sh, ci/ helpers
themes/           50 Halloy TOML theme files
```

### Key Source Modules (`src/`)

- `main.rs` — CLI entry point, clap dispatch
- `lib.rs` — Library crate (exports for integration tests)
- `commands/` — 23 command modules + shared helpers in `mod.rs`
- `auto_sync/` — Auto-sync subsystem (execution_lock, lock, mod, notification, pending, pending_lock, policy, schedule, spawn, status, test_events, worker)
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
- `~/.config/snp/themes/*.toml` — Halloy-compatible theme files
- `~/.config/snp/themes.toml` — active theme selection
- `~/.config/snp/usage.toml` — local usage metadata (not synced)
- `~/.config/snp/auto-sync-status.toml` — durable sync status (not synced, private)
- `~/.config/snp/transaction-journals/` — transaction journals
- `~/.config/snp/backups/` — backup snapshots

## Testing Notes

- Integration tests use `TempDir` with `XDG_CONFIG_HOME` env override
- Server tests use `sqlite::memory:` for isolation
- `tests/support/` provides reusable infrastructure: `TestEnvironment`, `RecordingServer`, `EventSink`
- Tests never use the developer's real config, keychain, or ports
- `SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on all test commands
- Golden command corpus: 24 edge cases verifying exact-text preservation

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
