# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

## [1.1.0] - 2026-06-05

### Added
- Environment variables documentation in README
- Security warning about snippet command execution
- Development setup instructions in CONTRIBUTING.md

### Changed
- Bump version to 1.1.0 (1.0.0 already published on crates.io)
- Add version requirement to snip-proto dependency for crates.io publishing
- Expand Cargo.toml exclude list for smaller published crate
- Refactor TUI select_snippet_inner into smaller functions
- Replace expect() with unwrap() + safety comments in variable parsing
- Use std::thread::scope in clipboard operations instead of thread-per-op
- Extend audit log escaping to cover all ASCII control characters
- Fix variable prompt to fill all defaults on Enter

### Fixed
- Formatting issues in sync_commands.rs
- Dead code removal (unused 'p' handler, #[allow(dead_code)] annotations)
- Audit log channel overflow now logs warning instead of silently dropping
- Audit log rotation uses symlink_metadata to prevent symlink attacks

### Removed
- Invalid `changelog` key from Cargo.toml

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

[1.1.0]: https://github.com/anomalyco/snip-it/releases/tag/v1.1.0
[1.0.0]: https://github.com/anomalyco/snip-it/releases/tag/v1.0.0
[1.2.0]: https://github.com/anomalyco/snip-it/releases/tag/v1.2.0
