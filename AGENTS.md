# AGENTS.md

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

# Single test by name (e.g. one deterministic_e2e test)
cargo test --test deterministic_e2e --features test-support -- --exact test_observer_headline_sync_e2e --test-threads=1

# snip-sync tests (needs test-helpers feature)
cargo test -p snip-sync --features test-helpers
```

**Key gotcha:** `cargo test --lib -p snip-it` does not work — `snip-it` is binary-only. Use `cargo test -p snip-it` or `cargo test --workspace`.

**Key gotcha:** Many integration tests (`deterministic_e2e`, `restore_crash_failpoints`, `transaction_crash_recovery`, `cleanup_crash_failpoints`, `process_lifecycle`, etc.) require `--features test-support` to compile.

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
- `auto_sync/` — Auto-sync subsystem (execution_lock, executor, lock, mod, notification, pending, pending_lock, policy, schedule, spawn, status, test_events, worker)
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

## Critical Gotchas

### Generated code
`src/ui/_generated_bundled_themes.rs` is generated at build time by `scripts/build_themes.py` (invoked from `build.rs`). Never edit it directly.

### TOML backslash escape handling
The save path does NOT post-process `toml::to_string_pretty` output. The golden command corpus includes tabs, trailing spaces, and CRLF that must survive the full save/load pipeline. See `src/utils/toml_helpers.rs`.

### Executor subprocess must never reacquire execution lock
The worker holds `SyncExecutionLock` for the entire detached cycle. Adding any `execution_lock::try_acquire` call to the executor would deadlock. Pinned by `test_executor_source_does_not_reference_execution_lock`.

### AGENTS.override.md
Contains session-specific pitfall notes and plan review findings. Consult it for implementation guidance.

### Kernel-backed process file locks
All auto-sync locks and the `snip-sync` server singleton use `flock` (Unix) / `LockFileEx` (Windows). The kernel alone is authoritative — persistent lock files may contain stale metadata. `Drop` releases the lock without unlinking the file.

### Mutation gate
`gate_mutation_on_interrupted_transactions()` must be called before any local mutating operation. Single journal = auto-rollback; multiple/incomplete = refuse and direct to `snp repair`.

### Deterministic test assertions
Tests must use exact counts (not `>= 1`), prove server-side state effects, and verify pending clear ordering. See `tests/deterministic_e2e.rs`.

### Test event emission
Worker/executor processes emit lifecycle events when `SNP_TEST_EVENTS_DIR` is set (JSON-lines). See `tests/support/event_sink.rs` (test-side) and `src/auto_sync/test_events.rs` (production).

### No command filtering (by design)
Snippet commands execute as-is — no sanitization. Intentional for power users.

## Key Architecture Notes

### Auto-Sync (two-process-per-cycle)
- Detached worker (`snp auto-sync-worker`) spawns killable executor (`snp auto-sync-execute`)
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
- `.skills/` — specialized agent reference docs (encryption, keychain, server, sync, UI, etc.)

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
| Test infra | `architecture/test-infrastructure.md` | Deterministic E2E, event sink, temp-dir isolation |
