# AGENTS.md

## Build & Test Commands

```bash
# Build the entire workspace (snip-it, snip-proto, snip-sync)
cargo build --workspace
cargo build --release

# Run all tests across the workspace (unit + integration + server)
cargo test --workspace

# Run only CLI integration tests
cargo test --test integration

# Run only sync integration tests (async, needs test-helpers feature)
cargo test --test sync_integration

# Run PTY end-to-end tests (MUST run single-threaded)
cargo test --test pty_integration -- --test-threads=1

# Run only auto-sync closure tests
cargo test --test auto_sync_closure

# Run only snip-sync tests
cargo test -p snip-sync

# Run Phase 05A test suites
cargo test --test deterministic_e2e
cargo test --test failure_class_contracts
cargo test --test debounce_matrix
cargo test --test sync_contracts
cargo test --test mutual_exclusion
cargo test --test process_lifecycle
cargo test --test local_contracts
cargo test --test package_evidence

# Run Phase 07A test suites
cargo test --test persistence_unit
cargo test --test identity_contract

# Lint (warnings are errors)
cargo clippy --workspace --all-targets -- -D warnings

# Run clippy (warnings are errors)
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Format check
cargo fmt --all -- --check
cargo fmt  # auto-format
```

**Key gotcha:** The main `snip-it` crate is binary-only — `cargo test --lib -p snip-it` does not work. Use `cargo test -p snip-it` (binary + integration tests) or `cargo test --workspace`.

## Toolchain

- **Rust 1.94**, edition 2024 (unusual — not 2021). See `rust-toolchain.toml`.
- `rustfmt.toml`: max_width=100, 4-space indent, Unix newlines, `edition = "2024"`.

## Project Structure

```
snip-it/          Main crate — binary "snp" (src/main.rs)
snip-proto/       Protobuf definitions, tonic-generated gRPC code
snip-sync/        Sync server (gRPC + HTTP/axum)
tests/            Integration tests (integration.rs, pty_integration.rs, sync_integration.rs, auto_sync_*.rs,
                          deterministic_e2e.rs, failure_class_contracts.rs, debounce_matrix.rs,
                          sync_contracts.rs, mutual_exclusion.rs, process_lifecycle.rs,
                          local_contracts.rs, package_evidence.rs)
themes/           50 Halloy TOML theme files
scripts/          build_themes.py — LZMA-compresses themes/ into src/ui/_generated_bundled_themes.rs
```

### Key Source Modules (`src/`)

```
src/main.rs              CLI entry point, clap dispatch
src/lib.rs               Library crate (exports for integration tests)
src/commands/             16 command modules (new, list, run, clip, select, search, edit,
                          sync, register, library, premade, import, doctor, cron, shell,
                          keybindings, status) + shared helpers in mod.rs
src/auto_sync/            Auto-sync subsystem (policy, pending, lock, executor, worker, spawn, notification,
                          status, schedule)
src/auto_sync/test_events.rs Test-only event emission for worker/executor lifecycle tracking
src/auto_sync/status.rs  Durable status persistence (auto-sync-status.toml), failure/success recording, integrity checks
src/auto_sync/schedule.rs Centralized schedule decision function, worker storm prevention, ScheduleDecision enum
src/auto_sync/policy.rs  Expanded FailureClass (11 variants), RetryDisposition, transient_backoff()
src/ui/                   TUI (ratatui + crossterm), theme system, syntax highlighting, variable prompts
src/utils/                Config paths, TOML helpers, variable parsing, shell keywords, temp files, atomic writes
src/utils/atomic.rs         Atomic file-write helpers (write_private_atomic, atomic_replace with durability classes)
src/library.rs            Snippet/library data structures and TOML persistence
src/sync.rs               gRPC client for snip-sync server
src/sync_commands.rs      Sync orchestration and merge logic
src/encryption.rs         AES-256-GCM + Argon2id end-to-end encryption
src/config.rs             Sync settings, path resolution, keychain API key
src/error.rs              SnipError enum (with SyncFailure variant + SyncFailureKind) and SnipResult type
src/logging.rs            Structured logging with file rotation
src/clipboard.rs          Cross-platform clipboard access
src/sort.rs               Sort modes, ranking, tie-break chain
src/usage.rs              Local usage metadata (use count, last-used)
src/output.rs             Snippet output field rendering/presentation
src/diagnostics.rs        Shared diagnostic model for import/doctor
src/status_snapshot.rs    Canonical read-only status projection for `snp status` and doctor
src/proto.rs              Prost-generated protobuf types
src/update.rs             Self-update support (crates.io, Homebrew, GitHub releases)
src/transaction.rs          Local mutation transaction boundary (journal, lock, commit, rollback)
src/migration.rs            Migration framework (schema versioning, trait-based migrations)
src/commands/validate_cmd.rs Validation command (comprehensive read-only checks)
src/commands/backup_cmd.rs  Backup snapshot command (manifest, checksums, secret-free)
src/commands/restore_cmd.rs Restore command (dry-run, merge, replace, rollback)
src/commands/repair_cmd.rs  Conservative repair command (idempotent, backed-up)
src/selector.rs          Shared snippet selector model (SnippetSelector, ResolutionPolicy)
src/outcome.rs           CLI outcome types and exit-code mapping (CliOutcome)
src/commands/get_cmd.rs  Deterministic non-TUI snippet retrieval
```

## Critical Gotchas

### Binary-only crate
`snp` is the binary name. The crate is `snip-it`. The workspace members are `snip-proto` and `snip-sync`.

### Generated code
`src/ui/_generated_bundled_themes.rs` is generated at build time by `scripts/build_themes.py` (invoked from `build.rs`). Never edit it directly.

### Sync tests need `test-helpers` feature
`snip-sync` has a `test-helpers` feature for in-process server testing. `snp`'s dev-dependencies enable it automatically, but if you test sync crates individually, pass `--features test-helpers`.

### PTY tests must be single-threaded
`tests/pty_integration.rs` uses `portable-pty` and creates real PTY pairs. Always pass `--test-threads=1`.

### TOML backslash escape handling
`src/utils/toml_helpers.rs` has `fix_invalid_toml_escapes()` (load) and `quote_strings_containing_backslashes()` (utility). The save path does NOT post-process — `toml::to_string_pretty` output is written directly. The earlier regex-based post-processing was removed because it corrupted tabs, trailing whitespace, and CRLF. The golden command corpus includes tabs, trailing spaces, and CRLF that must survive the full save/load pipeline.

### No command filtering (by design)
Snippet commands execute as-is — no sanitization, no guardrails. This is intentional for power users.

### Executor subprocess never reacquires execution lock
`src/auto_sync/executor.rs` invokes `crate::sync_commands::run_sync` directly. The worker (`src/auto_sync/worker.rs`) holds the `SyncExecutionLock` for the entire detached cycle. Adding any `execution_lock::try_acquire` or `wait_acquire` call to the executor would deadlock the worker waiting on its own child. The closure-phase structural test `test_executor_source_does_not_reference_execution_lock` pins this invariant.

### `AGENTS.override.md` exists
Contains session-specific pitfall notes and plan review findings. Consult it for implementation guidance.

### Deterministic test assertions
Phase 05A tests must use exact counts (not `>= 1`), prove server-side state effects,
and verify pending clear ordering. See `tests/deterministic_e2e.rs` for the headline test pattern.

### Test event emission
Worker and executor processes emit lifecycle events when `SNP_TEST_EVENTS_DIR` is set.
Events are JSON-lines in `<SNP_TEST_EVENTS_DIR>/test-events.jsonl`.
Use `EventSink` (test-side) and `EventWriter` (child-side) from `tests/support/event_sink.rs`.
Production code uses `src/auto_sync/test_events.rs` which checks the env var at runtime.

### Atomic write durability classes
`atomic_replace` supports four durability classes: DurableUserData (fsync), SensitiveConfig (0o600), RecoverableMetadata (no fsync), EphemeralCoordination (no dir sync). Use `AtomicWriteOptions::for_durability()` for correct defaults.

### Transaction journals
Multi-file operations should use `transaction.rs` for crash-safe coordination. The journal is persisted to disk and can be recovered on startup. `commit_transaction` removes the journal; `rollback_transaction` restores from backups.

### Migration schema versioning
Library files can carry a `schema_version` key. Use `migration.rs` for version-gated operations. `write_schema_version` uses `toml::Table` (not `toml::Value`) to preserve array-of-tables structure.

## Key Architecture Notes

### Auto-Sync (two-process-per-cycle)
- Detached worker (`snp auto-sync-worker`) spawns killable executor subprocess (`snp auto-sync-execute`)
- Parent never holds the worker lock — it's the worker's responsibility
- All sync operations acquire `SyncExecutionLock` to prevent concurrent sync
- Local mutations always commit before remote work; failed sync never rolls back local state
- Debounce returns `DebounceResult` with latest observed state; preflight check before executor spawn
- `Clock` trait for deterministic testing of time-dependent logic
- Executor timeout (30s default) is independent of debounce; configurable via `sync_timeout` in `AutoSyncPolicy`
- `max_delay` separate from `debounce` — bounded latency prevents starvation
- `schedule_sync()` is the sole scheduling authority; replaces per-mutation spawn paths
- Startup recovery always schedules workers for valid pending work regardless of age
- Failure classification is typed (`FailureClass` enum, 11 variants) with variant-based classification via `SyncFailureKind` (no string matching for sync errors)
- Typed policy loading distinguishes `NotConfigured` (no sync account) from config failure
- Status is persisted in `auto-sync-status.toml` with CRC32 integrity (not DefaultHasher), secret redaction, and config fingerprint
- Backoff is durable (survives CLI restarts) with exponential schedule capped at 15 minutes
- Config-change detection releases deferred failures when credentials/settings change
- Foreground `snp sync` records durable status alongside detached workers
- Executor maps errors → `FailureClass` → `ExecutorExitCode` (11 distinct codes: TransientTimeout, CredentialStore, Configuration, Partial); worker maps back on exit
- Signal death on Unix is captured and logged for executor processes
- Windows process liveness uses actual `GetExitCodeProcess` API checks (not placeholder)
- Module: `src/auto_sync/` — policy, pending, lock, execution_lock, executor, spawn, worker, notification, status, schedule

### Error Handling
- `SnipError` enum in `src/error.rs`, `SnipResult<T> = Result<T, SnipError>`
- IO errors auto-convert via `From<io::Error>`
- `CryptoError` auto-converts via `From<CryptoError>`

### Async (Tokio)
- Global `RUNTIME: LazyLock<Runtime>` created lazily on first access
- Only async commands (`run`, `clip`, `search`, `sync`, `register`, `premade`) trigger initialization
- Sync operations use `runtime.block_on()` for async gRPC calls

### Selection Outcome Architecture
- `SnippetSelection` (TUI layer) → `SelectionOutcome` (lib) → `CommandOutcome` (commands)
- Cancellation maps to exit code 4 for `select`; `run`/`clip`/`search` treat cancellation as exit 0

### Output Field
- `output` is local-only — not synced, not in `ProtoSnippet`
- `snp edit --output`, `--output-stdin`, `--clear-output` for editing
- `--filter` required when using output edit flags

### Themes
- Halloy-compatible TOML at `~/.config/snp/themes/<name>.toml`
- Default theme (`Cyber Red`) hardcoded as fallback via `include_str!`
- `SNP_THEME` env var for backward compat

### CLI and Automation (Phase 08A)
- `snp get` provides deterministic non-TUI snippet retrieval (never executes)
- Exact selectors (`--id`, `--description-exact`, `--command-exact`) bypass TUI on `run`, `clip`, `edit`
- `--var key=value` provides explicit noninteractive variable assignment (repeatable)
- `CliOutcome` enum maps typed results to stable exit codes (0-9)
- `SnippetSelector` / `ResolutionPolicy` provide shared selector model for all deterministic targeting
- `VariableAssignments` type handles explicit variable values with duplicate detection
- Machine-output modes: `--json`, `--csv`, `--raw`, `--field`, `--expanded`
- Noninteractive modes never prompt; TTY detection prevents unexpected prompts

## Configuration Files

- `~/.config/snp/snippets.toml` — main storage (or per-library in `libraries/`)
- `~/.config/snp/sync.toml` — sync settings
- `~/.config/snp/libraries.toml` — library metadata
- `~/.config/snp/libraries/*.toml` — individual library files
- `~/.config/snp/premade/*.toml` — downloaded premade libraries
- `~/.config/snp/themes/*.toml` — Halloy-compatible theme files
- `~/.config/snp/themes.toml` — active theme selection
- `~/.config/snp/usage.toml` — local usage metadata (not synced)
- `~/.config/snp/auto-sync-status.toml` — Durable sync attempt status (not synced, private)
- `~/.config/snp/transaction-journals/` — Transaction journals (Phase 07A)
- `~/.config/snp/backups/` — Backup snapshots (Phase 07A)

## Testing Notes

- Integration tests use `TempDir` with `XDG_CONFIG_HOME` env override
- Server tests use `sqlite::memory:` for isolation
- Sync integration tests (`tests/sync_integration.rs`) are async `#[tokio::test]` with real in-process server
- Golden command corpus: 24 edge cases verifying exact-text preservation across all acquisition sources

## Deterministic Test Infrastructure (Phase 05A)

The `tests/support/` module provides reusable test infrastructure for deterministic end-to-end tests:

- `environment.rs` — `TestEnvironment` builder with isolated HOME, XDG, config, and credential handling
- `recording_server.rs` — `RecordingServer` wrapper around snip-sync test helpers with event tracking
- `event_sink.rs` — Cross-process JSON-lines event channel for worker/executor lifecycle evidence

### TestEnvironment Usage

```rust
use support::environment::TestEnvironment;

let env = TestEnvironment::builder()
    .with_server_url(&server_url)
    .with_debounce(2)
    .build()?;

// All commands are pre-configured with XDG_CONFIG_HOME and SNP_ALLOW_PLAINTEXT_API_KEY
env.snp_output(&["new", "--command-stdin", "--description", "test"]);
env.create_library("mylib");
env.new_snippet("my-snippet");
```

### Key Design Decisions

- Tests never use the developer's real config, keychain, or ports
- `SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on all test commands
- Each test gets a unique `device_id` and fixed `api_key`
- `TempDir` provides automatic cleanup
- Event sink uses JSON-lines format for process-safe concurrent writes

## Phase 06A: Public API Tightening

Phase 06A tightened the public API surface and documented the logical layering of the crate.

### Docs Directory

The `docs/` directory contains reference documents produced during Phase 06A:

| Document | Subject |
|----------|---------|
| `docs/PUBLIC_API.md` | Full public API surface inventory |
| `docs/LOGICAL_LAYERS.md` | Logical layer separation (public vs internal) |
| `docs/CANONICAL_OPERATIONS.md` | Canonical operation contracts |
| `docs/API_TIGHTENING_FINDINGS.md` | Findings from the API tightening audit |
| `docs/OBSOLETE_ITEMS.md` | Items removed as dead code |
| `docs/FEATURE_BOUNDARIES.md` | Feature boundary documentation |

### Dead Items Removed

The following items were removed as dead public API:

- `AutoSyncPolicy.max_retries` — field was never read; backoff is now durable and retry-count-based
- `STALE_LOCK_THRESHOLD_SECS` — constant was unused; lock staleness is handled by timeout logic
- `encryption::ct_eq` — constant-time equality helper was unreferenced; replaced by downstream crate functionality

### `#[non_exhaustive]`

Public enums now carry `#[non_exhaustive]` to allow future variant additions without breaking downstream callers.

## Phase 08A: CLI and Automation Polish

Phase 08A adds deterministic noninteractive retrieval, shared exact selectors, stable output and exit contracts, explicit variable assignment, and safe composition.

### New Commands
- `snp get` — deterministic non-TUI retrieval (never executes, no clipboard, no mutation)

### New Flags
- `--id`, `--description-exact`, `--command-exact` on `run`, `clip`, `edit` (bypass TUI)
- `--var key=value` on `get` (explicit variable assignment, repeatable)
- `--resolution` on `get` (unique, first, all)

### Exit Codes
- 0: success
- 1: general error
- 2: usage/argument error
- 3: not found
- 4: user cancelled
- 5: ambiguous match
- 6: validation/persistence failure
- 7: sync failure
- 8: execution failure
- 9: conflict/refused

### Key Types
- `SnippetSelector` — shared selector model for all deterministic targeting
- `ResolutionPolicy` — Unique, First, All
- `SelectionResult` — One, Many, NotFound, Ambiguous
- `CliOutcome` — typed application outcome for exit-code mapping
- `VariableAssignments` — explicit noninteractive variable values
- `GetField` — output field selector for `snp get`

### Verification
```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo test --workspace --all-features
```

## Architecture Documentation

The `architecture/` directory contains deep-dive documents for each module. Use them as reference when working on specific subsystems:

| Document | Subject |
|----------|---------|
| `architecture/overview.md` | Bird's-eye view, data flow, key patterns |
| `architecture/cli.md` | CLI entry point, argument parsing, dispatch |
| `architecture/commands/*.md` | Per-command deep dives |
| `architecture/core.md` | Core types, error handling |
| `architecture/library.md` | Data structures, persistence |
| `architecture/config.md` | Sync settings, path resolution |
| `architecture/encryption.md` | AES-256-GCM encryption |
| `architecture/sync.md` | Sync protocol, merge logic |
| `architecture/auto_sync.md` | Auto-sync policy, debounce, triggers |
| `architecture/status.md` | Status snapshot, recovery commands, diagnostics |
| `architecture/tui.md` | TUI keybindings, state machine |
| `architecture/ui.md` | UI components, theme system |
| `architecture/server.md` | snip-sync server architecture |
| `architecture/proto.md` | Protobuf definitions |
| `architecture/sort.md` | Sort modes, ranking |
| `architecture/usage.md` | Usage metadata |
| `architecture/output.md` | Output field rendering |
| `architecture/logging.md` | Structured logging |
| `architecture/clipboard.md` | Cross-platform clipboard |
| `architecture/utils.md` | Config paths, TOML helpers |
| `architecture/selector.md` | Snippet selector model, resolution policies |
| `architecture/outcome.md` | CLI outcome types, exit-code mapping |

## Skills

The `.skills/` directory contains specialized reference documents for agents working on specific modules. Load relevant skills when working in those areas:

| Skill | When to use |
|-------|-------------|
| `architecture-review.md` | Reviewing architecture docs against code |
| `encryption-module.md` | Working with encryption, Argon2, key cache |
| `keychain-integration.md` | API key storage, keyring crate patterns |
| `remediation-patterns.md` | Bug fix patterns, code quality |
| `server-module.md` | Working on snip-sync server |
| `sync-module.md` | Working on sync protocol, merge logic |
| `ui-module.md` | Working on TUI, themes, syntax highlighting |
