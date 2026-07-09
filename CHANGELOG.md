# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.3.0] - 2026-07-09

### Fixed
- Preserve the real in-memory API key when migrating legacy plaintext
  `sync.toml` values into the OS keychain; the saved config gets the
  `@keychain` marker, but the current operation continues with the
  actual credential.
- Verify `sync.toml` integrity checks against the exact saved TOML body,
  preserving trailing newlines and later user-authored comments.
- **Harden crates.io release.** Remove `themes/` from published package (embeds default theme directly in generated Rust source). Shrink public API surface to 9 modules. Replace `getrandom` jitter with `SystemTime::now().subsec_nanos()`. Split `get_config_dir()` into pure getter + `ensure_config_dir()` — fixes macOS legacy migration (could never run because `get_config_dir()` eagerly created the new directory). Harden `list_libraries` pagination loop against buggy servers. Use `fs::rename` instead of `fs::copy` for atomic backup restore.
- Move `subtle` to `[dev-dependencies]` (only used in tests).

### Changed
- Align release-facing documentation with the current Rust 1.94 MSRV and
  gRPC authorization metadata behavior.

### Removed
- Remove inert --non-interactive sync flag

### Added
- **Halloy theme support.** `snp` now ships 50 themes adapted from [Halloy](https://themes.halloy.chat). Press `e` in normal mode to open the theme picker; use `j`/`k` (or arrow keys) to preview themes live, `i` to filter, and `Enter` to save. Bundled themes are extracted to `~/.config/snp/themes/` on first launch; the active theme is persisted to `~/.config/snp/themes.toml`. The `SNP_THEME` env var is still honored for backward compatibility.
- **Build pipeline for bundled themes.** `scripts/build_themes.py` LZMA-compresses and base64-encodes every `.toml` under `themes/`, emitting `src/ui/_generated_bundled_themes.rs`. The build hook in `build.rs` re-invokes the script when the source themes are newer than the generated file, keeping the binary lean.
- New dependency: `lzma-rs = "0.3"` (pure-Rust LZMA decoder; no C toolchain required).

## [1.2.0] - 2026-06-05

### Added
- `rust-toolchain.toml` pinning Rust 1.88 with required components (`rustfmt`, `clippy`, `llvm-tools-preview`).
- `assets/demo.tape` for regenerating the README demo GIF with [vhs](https://github.com/charmbracelet/vhs).
- `.github/workflows/link-check.yml` running `lychee` weekly and on PRs.
- `.github/ISSUE_TEMPLATE/security.md` redirecting security reports to email.
- Dependabot grouping for minor/patch updates to reduce PR noise.
- `repo-hygiene` CI job that fails if any `.DS_Store`/`Thumbs.db`/`desktop.ini` is tracked.
- `msrv` CI job now exercises all three crates (snp, snip-sync, snip-proto).

### Changed
- README rewritten for end-user audience: lead with tagline, demo, install matrix, security callout.
- USER_GUIDE.md table of contents now complete; added "Migrating from pet", "Reset and Recovery", "Keychain Issues" subsections.
- SECURITY.md expanded: threat model, key derivation parameters, known API-key-in-body limitation, server deployment checklist.
- CONTRIBUTING.md expanded: full release process, MSRV policy, branching rules, dependency list.
- crates.io metadata: added `homepage`, expanded `keywords` and `categories`, set `documentation = "https://docs.rs/snp"`, added author email.
- `docs.rs` config: added `rustdoc-args = ["--cfg", "docsrs"]`; explicit target list (now includes `aarch64-unknown-linux-gnu` and `aarch64-apple-darwin`).
- `dependencies` audit: removed unused crates; pruned feature flags.

## [1.1.0] - 2026-06-05

### Changed
- Bump MSRV to 1.88 and edition to 2024
- Mark snip-proto and snip-sync as non-publishable (`publish = false`)
- Remove unused dependencies (prost, rustls-native-certs, rand from root crate; tower, hyper, http, async-trait from snip-sync)
- Simplify uuid features to v4-only (removed unused v7)
- Default server URL changed from http:// to https://
- CI MSRV check updated to Rust 1.88
- CI server-test job now runs `cargo test -p snip-sync` instead of `cargo test` from snip-sync directory

### Added
- Configurable sync network timeouts via `SNP_SYNC_CONNECT_TIMEOUT` and `SNP_SYNC_REQUEST_TIMEOUT` environment variables
- Theme-aware syntax highlighting colors (string and escape colors adapt to dark/bright theme)
- TUI draw errors are now logged instead of silently discarded
- Mouse capture disable failure is now logged
- Doc comments on SnipError constructors, SnippetData, ProcessResult
- Config corruption now creates a backup before returning defaults

### Fixed
- TUI: pressing `/` in insert mode now correctly clears the filter (was desyncing display from actual matching)
- TUI: cursor position overflow protection for very long input text
- TUI: scrollbar thumb and variable prompt now use theme colors instead of hardcoded Cyan/Yellow
- Sync: `sync_cmd::run()` now returns errors instead of silently swallowing them with exit code 0
- Sync: `merge_and_save` now returns `SnipResult` instead of `Result<_, String>`
- Sync: pull-only sync now checks for failures before advancing `last_sync` timestamp
- Library: `delete_library` now saves config before deleting file (atomicity improvement)
- Signal handlers now log errors gracefully instead of panicking with `expect()`
- Removed `.github/.DS_Store` from tracking, added recursive .DS_Store to .gitignore

## [1.0.0] - 2026-06-04

### Added
- Terminal UI (TUI) with fuzzy search, syntax highlighting, and visual multi-select mode
- Variable expansion system with `<name=default>` syntax for dynamic snippet parameters
- Cross-platform clipboard integration (macOS, Linux, Windows)
- End-to-end encrypted cloud sync via gRPC (AES-256-GCM + Argon2id key derivation)
- Multiple snippet libraries with primary library support
- Premade community snippet library downloads
- Automated periodic sync via cron integration
- Shell keyword expansion (`$HOME`, `~`, `$(date)`, `$PWD`, `$RANDOM`)
- Audit logging for snippet operations
- Dark and bright theme support
- Command execution with configurable timeouts
- Snippet import/export
- OS keychain integration for API key storage
- Snip-sync server with SQLite storage, rate limiting, and Prometheus metrics

### Security
- AES-256-GCM authenticated encryption for sync data
- Argon2id key derivation with OWASP-recommended parameters
- API keys stored in OS keychain by default (plaintext fallback requires explicit opt-in)
- Server-side API key hashing with Argon2id
- TLS enforcement for all sync connections
- Path traversal protection for premade libraries
- Parameterized SQL queries throughout
- No unsafe code in the codebase

### Fixed
- Encryption key cleanup now uses `std::mem::take` for proper zeroization
- Clipboard auto-clear failures log at warn level instead of debug
- Visual mode copy now copies commands (not descriptions)
- Sync merge uses `>=` for timestamp comparison (server wins on ties)
- Push-only sync counter increments regardless of failures
- Premade library TOCTOU race condition resolved
- Health check RPC verifies database connectivity
- Deleted snippets filtered from TUI display
- Sync error propagation to callers
- Premade sync returns error on failure

[1.3.0]: https://github.com/eggstack/snip-it/releases/tag/v1.3.0
[1.2.0]: https://github.com/eggstack/snip-it/releases/tag/v1.2.0
[1.1.0]: https://github.com/eggstack/snip-it/releases/tag/v1.1.0
[1.0.0]: https://github.com/eggstack/snip-it/releases/tag/v1.0.0
