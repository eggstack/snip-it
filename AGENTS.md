# AGENTS.md

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

# Single integration test target
cargo test --test platform_smoke
```

### Lint & Format

```bash
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

# All tests including integration (serial)
cargo test --workspace --all-features -- --test-threads=1

# snip-sync tests (needs test-helpers feature)
cargo test -p snip-sync --features test-helpers
```

**Key gotcha:** Only 3 integration tests require `--features test-support` to compile: `repair_transactions`, `process_lock_concurrency`, and `local_data_lock_barriers`. All other integration tests compile without the feature.

**Key gotcha:** PTY tests (`pty_integration.rs`) use real terminal pairs — always pass `--test-threads=1`.

## Toolchain & Environment

- **Rust 1.94**, edition 2024 (not 2021). See `rust-toolchain.toml`.
- `rustfmt.toml`: `max_width=100`, 4-space indent, Unix newlines, `edition = "2024"`.
- `.cargo/config.toml` raises the Windows thread stack to 8 MB — the large `Commands` enum overflows the 1 MB default. Do not remove.
- Linux needs `libdbus-1-dev pkg-config` installed (CI installs them); tonic TLS also needs OpenSSL headers on Linux.

## Release & Branching

- Publishing to crates.io is manual and local — no publish workflow exists; CI has no crates.io token.
- Publish in dependency order: `snip-proto` → `snip-sync` → `snip-it`. If `snip-proto` changes, bump its version in both dependents.
- Topic branches squash-merge into `main`; commits: imperative mood, first line under 72 chars.
- Version bumps + `CHANGELOG.md` update go together in one PR (see `CONTRIBUTING.md`, `RELEASING.md`).

## Project Structure

```
snip-it/          Main crate — binary "snp" (src/main.rs)
snip-proto/       Protobuf definitions, tonic-generated gRPC code
snip-sync/        Sync server (gRPC + HTTP/axum)
tests/            Integration tests (~45 files)
scripts/          check.sh, release-check.sh, ci/ helpers
themes/           50 Halloy TOML theme files
```

### Key Source Modules (`src/`)

- `main.rs` — CLI entry point, clap dispatch
- `lib.rs` — Library crate (exports for integration tests)
- `commands/` — one module per command + shared helpers in `mod.rs`
- `auto_sync/` — Auto-sync subsystem (execution_lock, lock, mod, notification, pending, pending_lock, policy, schedule, status, test_events, worker)
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
- `transaction.rs` — Transaction boundary with journal, lock, begin/commit/rollback
- `process_file_lock.rs` — Kernel-backed cross-process file lock (`flock`/`LockFileEx`)
- `logging.rs` — Structured logging and audit trail

## Critical Gotchas

### Generated code
`src/ui/_generated_bundled_themes.rs` is generated explicitly by `python3 scripts/build_themes.py`. Never edit it directly. Protobuf code in `snip-proto/src/snip_proto.rs` is checked in and regenerated only as an explicit maintainer operation after changing `snip-proto/proto/sync.proto`; normal builds do not require `protoc`.

### TOML backslash escape handling
The save path does NOT post-process `toml::to_string_pretty` output. The golden command corpus includes tabs, trailing spaces, and CRLF that must survive the full save/load pipeline. See `src/utils/toml_helpers.rs`.

### Single-helper execution lock
The detached `auto-sync-worker` holds `SyncExecutionLock` for the entire bounded cycle and runs `sync_commands::run_sync_with_limits` directly. Manual sync and cron acquire the same lock.

### Kernel-backed process file locks
All auto-sync locks and the `snip-sync` server singleton use `flock` (Unix) / `LockFileEx` (Windows). The kernel alone is authoritative — persistent lock files may contain stale metadata. `Drop` releases the lock without unlinking the file.

Linux process start tokens use `/proc/<pid>/stat` field 22 (`starttime`). Unix `kill(pid, 0)` probes treat `EPERM` and unknown errors as a live process; only `ESRCH` proves absence.

### Mutation gate
`gate_mutation_on_interrupted_transactions()` must be called before any local mutating operation. Single journal = auto-rollback; multiple/incomplete = refuse and direct to `snp repair`.

### No command filtering (by design)
Snippet commands execute as-is — no sanitization. Intentional for power users.

### AGENTS.override.md
Contains session-specific pitfall notes and plan review findings. Consult it for implementation guidance.

## Sync & Persistence Invariants

### Conflict resolution
- Live snippet conflicts use `(updated_at, device_id, SHA-256(synced fields))`; never reintroduce role-dependent `>=` server-wins behavior.
- Deletion wins over live content, including when the live copy has a later timestamp. This is intentional no-resurrection behavior, not pure LWW.
- `output`, `folders`, and `favorite` are local-only and must not enter the conflict fingerprint.
- `output` is local-only — not synced, not in `ProtoSnippet`. `snp edit --output` requires `--filter`.

### Sync uploads
- Sync uploads are byte-bounded using Prost `encoded_len()`. The client ceiling defaults to 3.5 MiB (below the server's 4 MiB gRPC limit).
- `PushSnippets` is idempotent by snippet identity — retrying an already-accepted batch is safe.
- Multi-batch `PushSnippets` errors preserve the original `SyncFailureKind` (e.g., `ClockSkew`, `Timeout`) via `add_batch_context()`.

### Auto-sync scheduling
- `schedule_sync`, `schedule_and_spawn`, and `schedule_existing_pending` return typed local scheduling errors. Pending-read and worker-spawn failures must never be collapsed into `NoPending`, `SpawnNow`, or a successful notification.
- Pending generations are monotonic. A lower generation observed during debounce or preflight is corrupt state: preserve the marker, log the failure, and do not spawn sync work.

### Transaction boundaries
- `restore` uses `begin_transaction` / `advance_to_backups_durable` / `advance_to_committing` / `advance_to_committed_local` / `commit_transaction` / `rollback_transaction`.
- `gate_mutation_on_interrupted_transactions` checks for journal-based interrupted state only.

### Persistence validation
- Malformed library TOML (`load_library()`) and `libraries.toml` (`LibraryManager::new()`) fail closed: best-effort backup + `SnipError` return, never synthesized empty writable library/config.
- Missing/empty files produce valid defaults; malformed files produce backup + error.

### Sync recovery
- Missing-library recovery uses atomic `<library>.sync_recovery` TOML state. Preserve corrupt markers, reuse exactly one normalized remote-name match, fail on ambiguity, and remove a marker only after relink and retry sync are durable.
- Recovery linkage and `last_sync` reset must be persisted in one `LibraryManager` save.

## Async & Runtime

- Global `RUNTIME: LazyLock<Runtime>` — only initialized by async commands (`run`, `clip`, `search`, `sync`, `register`, `premade`).
- Local-only commands (`select`, `list`, `get`, `validate`, `backup`, `new`, `edit`, `keybindings`, `completions`, `shell`, `doctor`, `status`, `repair`, `restore`, `import`) do not initialize the Tokio runtime.
- `run_snippet_selection` accepts `Option<&tokio::runtime::Runtime>` — pass `None` when `do_sync` is false, `Some(&RUNTIME)` when true.
- The auto-sync detached helper uses `Builder::new_current_thread()` instead of `new_multi_thread()`.
- The client retains `tokio`'s `rt-multi-thread` feature because the production detached auto-sync worker creates its own multi-thread runtime; do not prune it.

## Error Handling & CLI Surface

- `SnipError` enum (`src/error.rs`), `SnipResult<T> = Result<T, SnipError>`. `SnipError` never carries credentials or API keys.
- `FailureClass` (`src/sync_failure.rs`) has 4 variants: `Transient`, `Configuration`, `LocalFailure`, `Internal`.
- `SnipError` variants map to stable exit codes via `CliOutcome` → `exit_code::*`; see `docs/EXIT_CODES.md`.
- Exact selector construction is canonicalized via `resolve_exact_target()` in `selector.rs`.
- Clipboard copy side effects (audit log, usage index update) are canonicalized via `copy_to_clipboard()` in `clip_cmd.rs`.

### Selection & exit codes
- `SnippetSelection` (TUI) → `SelectionOutcome` (lib) → `CommandOutcome` (commands)
- Cancellation maps to exit code 4 for `select`; `run`/`clip`/`search` treat cancellation as exit 0
- Output-file execution failures (timeout/spawn) map to exit code 8

## Keyring

- `keyring = "4"` relies on its default `v1` feature set for platform stores: Apple Keychain, Windows Credential Manager, zbus Secret Service (Linux desktop). Do not build with `default-features = false` without re-enabling a store — credential persistence silently degrades to keyring's mock store.
- Tests bypass the OS keychain via `SNP_ALLOW_PLAINTEXT_API_KEY=true`; never remove that seam from test commands.

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
| Multi-batch sync | serial target | `sync_multibatch.rs` — also runs in `check.sh` with `--test-threads=1` |
| PTY | serial target | `pty_integration.rs` — real terminal pairs |
| Cross-process lock | serial target | `process_lock_concurrency.rs` — kernel flock, real subprocesses |
| Barrier-coordinated | serial target | `local_data_lock_barriers.rs`, `repair_transactions.rs` — `set_var`, barrier protocol |
| Deep recovery | manual/release | `transaction_crash_recovery.rs`, `cleanup_crash_failpoints.rs`, `restore_crash_failpoints.rs` |
| Release smoke | manual/release | `release-check.sh` Phase 3 — version/help, crash recovery, production seams, `manifest_contracts.rs` |
| Architecture | parallel | `architecture.rs` — source-scanning layer boundary enforcement |

### Deterministic test assertions
Tests must use exact counts (not `>= 1`), prove server-side state effects, and verify pending clear ordering. Auto-sync closure cases live in `tests/auto_sync_closure.rs`; sync-boundary cases live in `tests/sync_integration.rs` and `tests/sync_contracts.rs`.

### Test event emission
The helper emits lifecycle events when `SNP_TEST_EVENTS_DIR` is set (JSON-lines). See `tests/support/event_sink.rs` (test-side) and `src/auto_sync/test_events.rs` (production).

## Reference Docs

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

| Topic | File |
|-------|------|
| Bird's-eye view | `architecture/overview.md` |
| Auto-sync | `architecture/auto_sync.md` |
| Sync protocol | `architecture/sync.md` |
| Persistence | `architecture/persistence.md` |
| Server | `architecture/server.md` |
| TUI | `architecture/tui.md` |
| CLI | `architecture/cli.md` |
| Outcome/exit codes | `architecture/outcome.md` |
| Selector | `architecture/selector.md` |
| Sort/ranking | `architecture/sort.md` |
| Status | `architecture/status.md` |
| Encryption | `architecture/encryption.md` |
| Config | `architecture/config.md` |
| Core data model | `architecture/core.md` |
| Library | `architecture/library.md` |
| Logging | `architecture/logging.md` |
| Protobuf | `architecture/proto.md` |
| Clipboard | `architecture/clipboard.md` |
| Usage | `architecture/usage.md` |
| Output | `architecture/output.md` |
| Utilities | `architecture/utils.md` |
| UI modules | `architecture/ui.md` |
| Test infra | `architecture/test-infrastructure.md` |
