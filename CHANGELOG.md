# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- **Release 2 final serialization corrective — exact TOML round-trips for tabs, trailing spaces, and CRLF**
  - Removed `quote_strings_containing_backslashes` from the save pipeline (`save_library`, `save_snippets`, `save_config`, `save_sync_settings`). The helper silently corrupted tabs, trailing whitespace, and CRLF: its regex could not distinguish triple-quoted multi-line strings from ordinary double-quoted strings, and its single-quoted output preserved TOML escape sequences like `\t` as literal two-character pairs. The `toml::to_string_pretty` serializer already picks the correct quoting and escapes for every character.
  - Rewrote `fix_invalid_toml_escapes` as a hand-written TOML token scanner that correctly recognizes triple-quoted strings (`"""..."""` and `'''...'''`), single-quoted literal strings, line/block comments, and single-line basic strings. The previous regex-based implementation greedily consumed triple-quoted delimiters, corrupting multi-line TOML content.
  - **Release 2 closure pass — secure editor tempfiles, editor command parsing, and unified exact-source validation**
  - Editor temporary files are created atomically via `tempfile::Builder` in the OS temp directory with `0600` permissions and RAII cleanup. The previous hand-rolled PID/timestamp filename generation is gone.
  - `snp new --editor` now prefers `$VISUAL` over `$EDITOR`. The editor command specification is parsed with `shell-words`, so values like `code --wait`, `nvim -f`, or quoted paths containing spaces work without invoking a shell.
  - `snp new --from-file` now follows symlinks and validates the resolved target is a regular file. Broken symlinks, directories, FIFOs, sockets, and device nodes are rejected.
  - Stdin, file, and editor sources share a single `validate_exact_command_bytes` validator: 16 MiB cap, valid UTF-8, no NUL bytes, no empty/whitespace-only content.
  - Bash `snp_new_previous` now defensively checks for the `fc -ln` formatter prefix (`\t `) before stripping it; legitimate leading whitespace in the captured command is preserved.
  - Golden command corpus expanded from 15 to 24 entries including tabs, trailing spaces, CRLF, mixed newlines, and combinations with quotes and backslashes.

### Added
- Sort and ranking system (`--sort` and `--favorites-first` flags) for run, clip, search, select, and list commands
  - Sort modes: relevance (default), recent, last-used, most-used, description, command
  - `--favorites-first` groups favorited snippets before others
- Local-only usage tracking: records use count and last-used timestamp on successful run/clip
- Usage metadata stored in `~/.config/snp/usage.toml` (atomic writes, fail-open on corruption)
- Shared sort model in `src/sort.rs` with deterministic tie-break chain
- TUI sort indicators for all modes including `[used]` and `[freq]`
- Integration tests for sort flags, favorites-first, and CSV/JSON sort output
- **Output / notes presentation (Release 4B)**
  - Shared output presentation model (`src/output.rs`) with `OutputPresentation` type: safe terminal rendering, summary truncation, multiline bounding, ANSI/OSC sanitization, and fuzzy-search scoring budget.
  - TUI preview panel shows output below command with `--- Output / Notes ---` separator when present.
  - `snp edit --output <text>`, `--output-stdin`, `--clear-output` for structured output editing (requires `--filter`).
  - `snp list --search-output` includes the output field in fuzzy search matching.
  - Default `list` output hides empty output fields to reduce noise.
  - JSON and CSV output always include the raw `output` field exactly as stored.
  - `select`, `run`, and `clip` continue to act on `command` only; output is never emitted or executed.
  - Output content is treated as untrusted text: no eval, no shell execution, no ANSI interpretation during display.
  - 36 new tests covering JSON/CSV preservation, edit set/clear/stdin, search-output flag, multiline roundtrip, tab/special char roundtrip, ANSI preservation, conflict flags, no-eval security, and help text.
- **Explicit pet import command (Release 3B)**
  - `snp import pet <path>` creates a native named library from a pet TOML file. Source files are never modified.
  - Options: `--library <name>`, `--merge`, `--replace`, `--dry-run`, `--strict`, `--report human|json`, `--report-file <path>`.
  - Atomic writes via temp-file-and-rename; existing libraries are backed up before merge/replace.
  - Duplicate detection: exact duplicates (same command + description) are skipped during merge; semantic warnings for same-command-different-description and same-description-different-command.
  - Diagnostics: unknown TOML fields, missing description, empty command, choice variables, output fields preserved.
  - Human-readable report to stderr; JSON report to stdout; `--report-file` for persistent JSON output.
  - Library name derived from source filename when `--library` is omitted.
  - 35 new tests: 20 integration tests (default create, explicit name, collision, merge, dry-run, source untouched, JSON report, error cases, strict/permissive, replace, command preservation, choice variables, mixed aliases, help, flag conflicts) and 15 unit tests (name derivation, duplicate detection, TOML parsing, entry conversion).
- **Compatibility diagnostics (Release 3C)**
  - `snp doctor --pet-file <path>` performs read-only analysis of pet snippet files: TOML parse status, unknown fields, missing required fields, empty commands, choice variables, duplicates, output fields, normalization preview, and recommended import command.
  - `snp doctor --compatibility` audits the installed snp environment: binary version, config directory, library directory, primary library, sync config, shell availability, shell init syntax validation (bash -n/zsh -n/fish --no-execute), editor configuration ($EDITOR/$VISUAL), legacy paths, Release 1 select availability, Release 2 acquisition flags, and Release 3 choice-variable parser.
  - `snp doctor --library <name>` analyzes a specific library file using the same analysis as --pet-file.
  - `snp doctor --check-shell <bash|zsh|fish>` validates `snp shell init` output syntax for the specified shell.
  - Shared diagnostic model (`src/diagnostics.rs`) with `SourceSpan` type for byte-offset source positions, used by both import and doctor: `CompatibilityDiagnostic`, `DoctorReport`, `PetImportReport` with stable machine-readable codes (E-/W-/I- prefix convention).
  - Options: `--strict` (treat warnings as errors), `--report human|json` (output format).
  - Exit codes: 0 (no errors), 1 (operational failure), 2 (error diagnostics found).
  - Human-readable report to stderr; JSON report to stdout (same stream convention as `snp import`).
  - Doctor never mutates source, destination, config, or library state.
  - 29 integration tests and 18 unit tests covering file analysis, JSON output, compatibility audit, strict mode, non-mutation, command execution prevention, variable expansion prevention, API key leakage prevention, config preservation, and import/doctor consistency.
- **Auto-sync mutation trigger integration (Release 5C)**
  - Central mutation notification API: `notify_mutation(kind, origin)` and `notify_local_mutation(policy, context)`.
  - All syncable mutation commands now trigger auto-sync after successful local commit: `snp new` (all sources), `snp edit` (editor), TUI delete, `snp import pet` (once per import), `snp library create/delete`.
  - Output-only edits (`snp edit --output/--clear-output`) do NOT trigger sync (output is local-only).
  - Explicit sync (`--sync` flag, `snp sync`) clears pending auto-sync state to prevent duplicate delayed sync.
  - Sync-origin writes (`MutationOrigin::SyncMerge`) never trigger auto-sync (prevents feedback loops).
  - `run_auto_sync()` creates its own Tokio runtime internally — callers don't need to pass one.
  - 10 new unit tests covering notification API: disabled policy, sync-merge suppression, user/import origins, all mutation kinds, AccountConfig, library ID, clear-after-explicit-sync, result Debug/PartialEq, MutationContext construction.
- **Pet multiple-choice variable compatibility (Release 3A)**
  - Variable parser recognizes Pet `<name=|_opt1_||_opt2_||_opt3_||>` syntax and parses it into `VariableKind::Choices`.
  - TUI variable prompt renders choice variables as a navigable list selector (arrow keys / j/k).
  - `expand_command` expands choice variables with the selected value, just like required variables.
  - Raw command text is preserved in storage — choices are only expanded during interactive prompting.
  - Parser diagnostics warn on malformed choice syntax and duplicate variable names. Diagnostics now include machine-readable `code` and optional `suggested_fix` fields.
  - Repeated variables with the same name are deduplicated in the prompt — the user is prompted once and the value is reused for all occurrences.
  - Non-interactive fallback: `prompt_variables` returns an error when no controlling terminal is available instead of panicking.
  - Fuzz tests (500 iterations) verify the parser, expansion, and choice extraction never panic on arbitrary input. One subtraction-with-overflow bug in `extract_choices` was found and fixed.
  - 65+ new unit and integration tests covering choice parsing, prompting, expansion, serialization roundtrips, PTY end-to-end selection/default/cancel/dedup/restore, and edge cases.
- New unit tests in `src/commands/new_cmd.rs`: stdin rejection of empty/whitespace input, oversize-input rejection via the shared validator, symlink-following and broken-symlink behavior, FIFO/character-device rejection, `shell-words` parsing of editor specs, and ten multiline-prompt tests.
- New integration tests in `tests/integration.rs`: editor-source golden corpus round-trip, multiline terminator limitation, exact select-storage round-trip, backup-preserves-command, sync round-trip preservation, and run-storage plumbing.
- New Bash behavioral tests: `snp_new_previous` preserves leading tabs and quoted/backslash content.

## [1.3.1] - 2026-07-09

### Changed
- Bump `toml` 0.8 → 1.1 (unifies with snip-sync's toml)
- Bump `signal-hook` 0.3 → 0.4
- Bump `clipboard-win` 4.5 → 5
- Bump `prometheus` 0.13 → 0.14 (snip-sync)
- Relax `time` constraint to `<0.4`
- Bump GitHub Actions: `action-gh-release` v2→v3, `docker/login-action` v3→v4, `docker/build-push-action` v5→v7

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

[1.3.1]: https://github.com/eggstack/snip-it/releases/tag/v1.3.1
[1.3.0]: https://github.com/eggstack/snip-it/releases/tag/v1.3.0
[1.2.0]: https://github.com/eggstack/snip-it/releases/tag/v1.2.0
[1.1.0]: https://github.com/eggstack/snip-it/releases/tag/v1.1.0
[1.0.0]: https://github.com/eggstack/snip-it/releases/tag/v1.0.0
