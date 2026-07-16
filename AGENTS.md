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

# Run only snip-sync tests
cargo test -p snip-sync

# Lint (warnings are errors)
cargo clippy --workspace --all-targets -- -D warnings

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
tests/            Integration tests (integration.rs, pty_integration.rs, sync_integration.rs, auto_sync_*.rs)
themes/           50 Halloy TOML theme files
scripts/          build_themes.py — LZMA-compresses themes/ into src/ui/_generated_bundled_themes.rs
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

### `AGENTS.override.md` exists
Contains session-specific pitfall notes and plan review findings. Consult it for implementation guidance.

## Key Architecture Notes

### Auto-Sync (two-process-per-cycle)
- Detached worker (`snp auto-sync-worker`) spawns killable executor subprocess (`snp auto-sync-execute`)
- Parent never holds the worker lock — it's the worker's responsibility
- All sync operations acquire `SyncExecutionLock` to prevent concurrent sync
- Local mutations always commit before remote work; failed sync never rolls back local state
- Module: `src/auto_sync/` — policy, pending, lock, execution_lock, executor, spawn, worker, notification

### Error Handling
- `SnipError` enum in `src/error.rs`, `SnipResult<T> = Result<T, SnipError>`
- IO errors auto-convert via `From<io::Error>`

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

## Configuration Files

- `~/.config/snp/snippets.toml` — main storage (or per-library in `libraries/`)
- `~/.config/snp/sync.toml` — sync settings
- `~/.config/snp/libraries.toml` — library metadata
- `~/.config/snp/libraries/*.toml` — individual library files
- `~/.config/snp/usage.toml` — local usage metadata (not synced)

## Testing Notes

- Integration tests use `TempDir` with `XDG_CONFIG_HOME` env override
- Server tests use `sqlite::memory:` for isolation
- Sync integration tests (`tests/sync_integration.rs`) are async `#[tokio::test]` with real in-process server
- Golden command corpus: 24 edge cases verifying exact-text preservation across all acquisition sources
