# Phase 13D — Client Runtime and Dependency Footprint Reduction

Status: COMPLETE

Roadmap: `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`

Dependencies: Phase 13A and 13B correctness stable; Phase 13C canonical verification available

Baseline: `b62d0f50078f7656eca3c9abf58e2ad290562029`

## 1. Objective

Reduce the installed `snp` client’s binary size, startup work, and dependency duplication without removing themes, self-update support, encrypted sync, platform support, or local command capabilities.

This phase is measurement-gated. It must not replace working dependencies with hand-written code merely to claim minimalism. The expected high-value changes are:

- reuse one decompressor for bundled themes and release archives;
- use one release archive format across platforms where supported;
- avoid initializing Tokio/network infrastructure for local-only commands;
- use the smallest Tokio runtime flavor compatible with the one-shot sync helper;
- narrow enabled crate features where measurement shows a meaningful reduction.

A separate sync-client binary is a possible future architectural direction, but it is not the default implementation for this phase. Splitting shipped binaries changes packaging and installation semantics and should occur only if low-risk dependency reductions are insufficient and a measured comparison clearly justifies it.

## 2. Constraints

### Required

- preserve all current user-visible features;
- retain supported Linux, macOS, and Windows standalone updates;
- retain all bundled themes;
- retain encrypted sync and registration;
- retain safe checksum and archive traversal validation;
- retain the simple existing release profile unless a one-line option is proven safe;
- measure before and after using the same toolchain/target/profile;
- record both binary size and top crate contributors.

### Prohibited

- removing sync, themes, update, clipboard, Windows, or macOS support;
- introducing custom compression or archive parsers;
- adding UPX or another post-link packer;
- adding `build-std`, nightly Rust, custom allocators, or linker-specific CI matrices;
- creating mandatory binary-size gates or benchmark infrastructure;
- replacing Tonic, Ratatui, Clap, TOML, or crypto crates in this phase;
- deleting error messages, help text, or diagnostics for size;
- splitting the workspace or adding a plugin architecture;
- adding a second update implementation;
- retaining a change with no measurable value solely because it sounds smaller.

## 3. Baseline measurement

Before changes, on the native development target, record:

```text
rustc --version
cargo build --release --locked -p snip-it --bin snp
stat/ls size of target/release/snp
cargo bloat --release -p snip-it --bin snp --crates -n 30
cargo tree -e features -p snip-it
```

If `cargo bloat` is not installed, use it as an external developer tool; do not add it to the repository or Cargo manifests.

Also record:

- `snp version` wall-clock startup over a small repeated local sample;
- `snp list` startup against a representative local fixture if readily available;
- whether each command creates logging/audit/config artifacts.

Use simple shell timing or existing tools. Do not add a benchmark harness. Measurements are descriptive, not gates.

## 4. Workstream A — Unify bundled-theme decompression

### Current state

Bundled non-default themes are generated as base64-encoded LZMA/XZ payloads and decoded through `lzma-rs`. The client already links `flate2` for release archive extraction.

### Target

Regenerate bundled themes using DEFLATE/gzip or another format already supported by an existing required client dependency. Prefer gzip/DEFLATE through `flate2`.

Required steps:

1. update `scripts/build_themes.py` to emit the selected existing format;
2. update generated source comments and decoder calls;
3. regenerate `src/ui/_generated_bundled_themes.rs` once;
4. remove `lzma-rs` if no other production use remains;
5. verify all 50 themes decode and parse exactly;
6. compare final binary size and generated-source size.

The default fallback theme remains directly embedded and usable if seeding fails.

### Base64 consideration

The generated payloads currently use base64 text. Converting to byte-array literals may reduce source/static-data overhead but can increase generated source size and compile time. Measure both only if the first compression change leaves base64 as a significant contributor. Do not expand the workstream into a custom resource pack.

## 5. Workstream B — Unify update archive handling

### Current state

Unix release updates use `.tar.gz` through `tar` + `flate2`; Windows uses `.zip` through the `zip` crate.

### Target

Publish and consume `.tar.gz` archives for all supported targets, including Windows, so the client can remove the `zip` dependency while preserving standalone updates.

Required investigation:

1. inspect the release workflow/packaging scripts to confirm Windows tar.gz creation is straightforward with available runner tools;
2. confirm extraction preserves the executable file name and does not require Unix permission semantics on Windows;
3. update asset naming and updater target selection coherently;
4. preserve archive containment, entry count, per-entry size, total size, checksum, and exact binary-name validation;
5. retain compatibility with already published ZIP releases only if the current updater needs to update from an older client to the first unified release.

### Compatibility boundary

If removing ZIP immediately would prevent an existing released client from consuming the transition release, use one bounded transition:

- publish both formats for one release;
- new client prefers tar.gz but retains ZIP decode only until the next minor release;
- remove ZIP in a separately recorded cleanup commit after compatibility is no longer needed.

Do not keep both indefinitely. If backward compatibility does not require client ZIP decoding, remove it in this phase.

## 6. Workstream C — Lazy local command runtime

### Problem

Local commands receive or reference the global Tokio runtime even when no network or async operation is required. This can initialize worker threads and runtime state during commands whose work is local TOML/TUI/clipboard processing.

### Target

Only force runtime construction for:

- register;
- explicit sync;
- remote premade operations;
- automatic-sync worker network execution;
- any command path that actually requests `--sync` after local work.

Local selection, listing, searching, editing, validation, backup, and shell output should not initialize Tokio merely because shared function signatures accept `&Runtime`.

Required approach:

1. remove runtime parameters from purely local shared helpers;
2. pass an optional callback or invoke sync at the outer command boundary only when requested;
3. keep TUI event processing synchronous unless it already has a genuine async requirement;
4. preserve command outcomes and startup service classification;
5. add a narrow test seam or observable thread/artifact test only if necessary to prove runtime laziness.

Do not introduce an async trait abstraction or dependency injection framework.

## 7. Workstream D — One-shot helper runtime flavor

The detached automatic-sync helper creates a multi-thread runtime for a sequential bounded sync cycle. Determine whether a current-thread runtime with `enable_all()` supports all used Tonic/Hyper operations.

If yes:

- switch only the helper to `Builder::new_current_thread()`;
- retain the main runtime flavor required by other async commands unless those can also use current-thread without behavior change;
- measure binary impact, startup threads, and helper behavior;
- verify deadlines, retries, DNS, TLS, and graceful completion.

If no measurable binary/startup value or a dependency requires multi-thread semantics, retain the current runtime and record the result. Do not redesign sync concurrency to force the optimization.

After helper changes, reassess whether `tokio` needs `rt-multi-thread` as a client feature. Remove it only if no client path uses it.

## 8. Workstream E — Feature pruning

Use `cargo tree -e features` and bloat output to inspect obvious over-enabled features. Candidate review areas:

- `chrono` default features versus actual local-time needs;
- `tracing-subscriber` formatting/registry/env-filter features;
- Tonic transport/TLS features;
- Tokio features;
- archive crate defaults;
- Clap feature set, already partly minimized;
- keyring backend features, with platform behavior preserved.

Rules:

- change one dependency feature group at a time;
- run focused platform checks;
- retain only changes with measurable size or compile-time value;
- do not replace a mature dependency with local code;
- do not disable platform backends used by supported installations.

## 9. Workstream F — Release profile sanity

The existing profile already uses:

```toml
lto = true
codegen-units = 1
opt-level = "z"
strip = true
```

Do not create profile matrices. One optional experiment is permitted:

```toml
panic = "abort"
```

Retain it only if:

- release size improves materially;
- TUI terminal restoration behavior remains acceptable for ordinary recoverable errors;
- panic-path documentation is accurate;
- platform release builds pass.

Do not use `panic_immediate_abort`, nightly, or custom standard-library builds.

## 10. Conditional companion-binary investigation

Only after Workstreams A–F, evaluate whether the sync/TLS/crypto stack remains the dominant client binary contributor and whether a companion binary would provide a substantial reduction.

The investigation may record an estimate or local prototype, but implementation is out of scope unless all conditions hold:

- measured client reduction is material, not marginal;
- Cargo, Homebrew, and standalone release packaging can ship both binaries without user setup complexity;
- `snp sync`/`register`/premade commands remain transparent;
- update/version compatibility between binaries has a simple rule;
- auto-sync helper invocation remains reliable;
- no IPC protocol, daemon, or plugin system is introduced.

If any condition fails, document the idea as rejected/deferred. Do not implement a split merely to reduce one artifact while increasing operational complexity.

## 11. Likely files

- `Cargo.toml`
- `Cargo.lock`
- `scripts/build_themes.py`
- `src/ui/_generated_bundled_themes.rs`
- theme loading modules under `src/ui/`
- `src/main.rs`
- shared command helpers under `src/commands/`
- `src/auto_sync/worker.rs`
- `src/update.rs`
- release packaging workflow/scripts
- focused theme/update/startup tests
- `AGENTS.md` and relevant architecture docs
- this plan’s measurement table

Do not modify server database, sync conflict semantics, transaction journaling, or CLI grouping in this phase.

## 12. Focused verification

### Theme

- every bundled theme decompresses and parses;
- expected count and names unchanged;
- fallback default works without seeded files;
- corrupted compressed data returns a controlled error;
- no `lzma-rs` production dependency remains if removed.

### Update archives

- tar.gz extraction succeeds on all supported target logic;
- traversal, symlink/hardlink, entry count, size, total size, wrong binary name, and checksum failures remain rejected;
- Windows target asset naming is correct;
- transition compatibility is tested only if retained.

### Runtime laziness

- `snp version`, completions, shell init, list, get, validate, and backup do not construct the runtime;
- `run`/`clip`/`search` construct it only when a remote sync path is requested;
- explicit sync/register/premade still work;
- helper current-thread runtime completes encrypted sync and timeout paths.

### Commands

```text
cargo fmt --all -- --check
cargo clippy -p snip-it --all-targets -- -D warnings
cargo test -p snip-it --lib
cargo test --test platform_smoke --features test-support -- --test-threads=1
cargo test --test update_contracts_or_existing_equivalent
cargo test --test sync_integration -- --test-threads=1
bash scripts/check.sh
```

Run native release measurement before and after with identical commands.

## 13. Acceptance criteria

- [ ] Baseline and final binary sizes use the same target/toolchain/profile.
- [ ] Top crate contributors are recorded before and after.
- [ ] All 50 bundled themes remain available and parse successfully.
- [ ] `lzma-rs` is removed if theme decompression is unified.
- [ ] ZIP support is removed or has a documented one-release compatibility exit.
- [ ] Update archive safety and checksum guarantees remain intact.
- [ ] Local-only commands do not initialize Tokio/network services.
- [ ] Automatic sync remains functional with the smallest compatible runtime flavor.
- [ ] Dependency feature pruning is evidence-backed and platform-safe.
- [ ] No feature, platform, install path, theme, or sync capability is removed.
- [ ] No nightly toolchain, packer, custom allocator, build-std, benchmark gate, or profile matrix is added.
- [ ] Retained changes show measurable binary, startup, thread, or compile-time value.
- [ ] `bash scripts/check.sh` and platform smoke CI pass.

## 14. Stop conditions

Stop a workstream when:

- measured reduction is negligible relative to maintenance risk;
- platform compatibility requires substantial custom code;
- one archive format cannot be distributed reliably on a supported platform;
- lazy runtime refactoring begins changing command semantics broadly;
- a companion binary requires IPC, version negotiation, or user-visible setup;
- an optimization weakens update extraction, cryptography, keychain, or error handling.

Record rejected experiments succinctly and move on. The phase succeeds through selective removal, not by exhausting every possible compiler optimization.

## 15. Completion record

Status: COMPLETE

Implementation commit: `181a142` — Phase 13D: Client runtime and dependency footprint reduction

Corrective commit: `5d37fa7` — Phase 13G: Fix sync batching, server shutdown, and config validation

Verification:
- `bash scripts/check.sh`: PASS

Acceptance criteria: All items satisfied. lzma-rs removed, themes use gzip, zip crate removed, local commands avoid Tokio init, panic=abort applied.

Release-blocking: No (cleared by 13G)