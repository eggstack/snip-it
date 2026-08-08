# Phase 14D — Dependency and Binary Footprint Reduction

Status: READY FOR IMPLEMENTATION

Parent roadmap: `plans/snip-it-phase-14-correctness-simplification-roadmap.md`

Required predecessor: Phase 14A credential backend correctness

Date: 2026-08-08

Execution target: smaller coding models operating sequentially with narrow context.

## 1. Purpose

Reduce production dependency and binary weight without removing user-visible functionality.

The release profile is already aggressively size-oriented:

```toml
lto = true
codegen-units = 1
opt-level = "z"
strip = true
panic = "abort"
```

Do not spend this phase experimenting with more compiler flags. The remaining useful wins are dependency/features and deleting code that supports unused formats.

This plan is measurement-driven. Each workstream must record an actual before/after release binary size and may be reverted if it increases complexity for negligible benefit.

## 2. Baseline measurement

Before changing dependencies, from a clean tree run:

```text
cargo build --release --bin snp
```

Record:

```text
commit SHA
platform + architecture
rustc version
size in bytes of target/release/snp (or snp.exe)
```

Also capture feature/dependency evidence:

```text
cargo tree -p snip-it -e features
cargo tree -p snip-it -i arboard
cargo tree -p snip-it -i tonic
cargo tree -p snip-it -i image
cargo tree -p snip-it -i tar
cargo tree -p snip-it -i flate2
```

If `cargo-bloat` is already installed, it may be used as diagnostic evidence. Do not add it to this repository or CI.

Measure `snip-sync` separately only when a change touches its manifest or shared feature resolution. The primary target of this plan is the installed `snp` client binary.

## 3. Small-model rules

1. Make one dependency change at a time.
2. Regenerate `Cargo.lock` naturally with Cargo.
3. Build and test after every dependency change before proceeding.
4. Record actual release size after every accepted workstream.
5. Do not remove a feature because it looks unused in the manifest; search production code first.
6. Do not replace one dependency with a new dependency of equivalent scope merely to report fewer package names.
7. Do not add custom crypto, TLS, archive, clipboard, or compression code to avoid a maintained crate.
8. Revert any change that reduces portability or supported behavior.

## 4. Workstream A — Remove unused arboard image support

### 4.1 Baseline

The non-Windows clipboard implementation only uses text operations:

```text
Clipboard::new()
set_text(...)
```

Arboard 3.6.x enables `image-data` by default. That feature pulls image/image-framework dependencies which snip-it does not use.

### 4.2 Required manifest change

On the non-Windows target dependency, start with:

```toml
arboard = { version = "3", default-features = false }
```

Do not add `wayland-data-control` as part of this size pass unless the repository already documents native Wayland data-control support as a required feature. X11/XWayland text support remains the existing baseline behavior.

### 4.3 Verification

```text
cargo check -p snip-it
cargo build --release --bin snp
cargo tree -p snip-it -i image
```

Expected dependency result: the production `snp` graph no longer includes `image` solely through arboard.

Required platform checks:

- Linux compile;
- macOS compile;
- existing ignored/manual clipboard text smoke where a display is available.

Windows uses `clipboard-win` directly and should remain unaffected.

### 4.4 Acceptance

- [ ] No image clipboard API is used by snip-it.
- [ ] Arboard image-data defaults are disabled.
- [ ] Text copy/clear behavior compiles and existing tests pass.
- [ ] Release binary size is recorded before/after.

## 5. Workstream B — Narrow root Tonic features to client requirements

### 5.1 Baseline

The root client currently uses:

```toml
tonic = { version = "0.14", features = ["tls-ring"] }
```

Tonic defaults enable `codegen`, `router`, and `transport`; `transport` enables both server and channel support. The `snp` client needs generated client types, `Channel`/`Endpoint`, and TLS, but it does not host a Tonic server or Axum router.

### 5.2 Required audit

Before editing, inspect:

```text
src/sync.rs
src/proto.rs
snip-proto generated/build configuration
```

List every root-client Tonic API used and map it to documented Tonic features.

### 5.3 Preferred experiment

Test the smallest expected root dependency:

```toml
tonic = {
    version = "0.14",
    default-features = false,
    features = ["codegen", "channel", "tls-ring"]
}
```

If the current TLS setup explicitly uses native or webpki roots, add only the corresponding required trust-root feature. Do not restore full `transport` merely because one symbol is missing until the feature mapping is checked.

The `snip-sync` server crate may continue using its own server/transport feature set. Do not cripple the server to optimize the client.

### 5.4 Workspace feature-unification check

Build the client specifically:

```text
cargo build --release -p snip-it --bin snp
```

and inspect the client feature tree. Do not infer client binary contents from `cargo build --workspace`, where server features may be unified for the broader build graph.

### 5.5 Acceptance

- [ ] Root `snp` no longer requests Tonic router/server functionality it does not use.
- [ ] HTTPS sync behavior and plaintext loopback opt-in behavior remain unchanged.
- [ ] Sync integration tests pass.
- [ ] `snip-sync` server features remain intact.
- [ ] Release client size before/after is recorded.

## 6. Workstream C — Evaluate self-update archive removal

### 6.1 Baseline

The standalone GitHub-release update path downloads a `.tar.gz`, verifies `SHA256SUMS`, and locally performs bounded tar/gzip extraction. This keeps `tar` and `flate2` in the production client.

Bundled theme decompression also uses `flate2`, so removing updater gzip logic does **not** automatically remove `flate2` from the client. Confirm the dependency graph before claiming a dependency removal.

### 6.2 Goal

Preserve:

```text
snp update
standalone release installs
checksum verification
cross-platform releases
```

while deleting archive extraction code if the release pipeline can cheaply publish a raw self-update asset.

### 6.3 Preferred design

Keep human-facing `.tar.gz` release archives if desired, but additionally publish one raw executable asset per target, with a predictable name such as:

```text
snp-<tag>-<target>
snp-<tag>-<target>.exe
```

Include those files in the existing `SHA256SUMS` manifest.

The updater then:

1. selects the raw asset for the current target;
2. downloads it over HTTPS using the existing fetch path;
3. verifies the SHA-256 manifest entry;
4. installs the verified executable using the existing safe replacement logic.

Delete tar parsing/extraction limits only after the release assets exist.

### 6.4 Discovery step

Before editing, identify the existing release asset builder/workflow by searching `.github/workflows/`, `scripts/`, and release documentation for the current `snip-it-v<VERSION>-<target>.tar.gz` naming.

Record the exact packaging files in the implementation completion record.

### 6.5 Stop condition

If adding raw assets duplicates substantial packaging/release logic or makes release maintenance more complex than the updater extraction code, retain the current archive path and record `NO CHANGE — not a net simplification`.

Do not change release format solely to remove one crate name.

### 6.6 Acceptance if implemented

- [ ] Existing human install archives remain available or equivalent user docs are updated.
- [ ] `snp update` still verifies SHA-256 before replacement.
- [ ] Archive path-traversal code is removed only because no archive is parsed.
- [ ] No new parsing/decompression dependency replaces tar/gzip.
- [ ] Standalone update integration tests cover raw asset naming/checksum/install.
- [ ] Size effect is measured honestly, including the fact that theme gzip may retain `flate2`.

## 7. Workstream D — Narrow tracing-subscriber features only if measurable

The root manifest enables `tracing-subscriber` with `env-filter` while accepting all default features.

Audit the actual logging API usage in `src/logging.rs` and test an explicit feature set with `default-features = false` that retains:

- formatted file logging;
- registry/layer composition;
- `EnvFilter`/`SNP_LOG` behavior;
- no ANSI requirement for file logs.

Do not remove `env-filter`; that is existing user-visible diagnostic behavior.

Accept the change only if:

- all logging code compiles;
- the filter behavior is preserved;
- release size or dependency graph improves measurably;
- the manifest is not made substantially harder to understand.

Otherwise record `NO CHANGE`.

## 8. Workstream E — Measure current-thread manual sync runtime

The root binary currently creates a lazy multi-thread Tokio runtime for explicit async commands. Auto-sync already uses a current-thread runtime.

This workstream is optional and must be evidence-driven.

Experiment on a branch/working tree with a current-thread runtime built via Tokio `Builder`. Verify:

- registration;
- manual sync;
- premade fetch;
- explicit `--sync` paths;
- timers/retries/Tonic channel behavior.

If the current-thread runtime allows removing root `rt-multi-thread` without behavioral regression and yields a meaningful size reduction, keep it.

If any Tonic/network behavior becomes fragile, retain the multi-thread runtime. There is no product need to force this change.

Do not change `snip-sync` server runtime in this workstream.

## 9. Workstream F — Final dependency hygiene

After accepted changes:

```text
cargo tree -p snip-it -d
cargo tree -p snip-it -e features
```

Look only for obvious duplicate major versions or features directly caused by snip-it manifest choices.

Do not launch a dependency-upgrade campaign. Do not replace stable dependencies merely to collapse duplicate transitive versions unless there is a concrete binary/security/maintenance benefit.

## 10. Required verification

For each accepted manifest change:

```text
cargo check -p snip-it
cargo test -p snip-it --lib
cargo build --release -p snip-it --bin snp
```

At phase end:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check.sh
```

Cross-platform CI must remain green.

## 11. Completion record

Record a compact table in this plan when implemented:

| Change | Before bytes | After bytes | Delta | Kept? |
|---|---:|---:|---:|---|
| arboard image features | | | | |
| tonic client features | | | | |
| raw updater asset | | | | |
| tracing-subscriber features | | | | |
| current-thread runtime | | | | |

Use the same platform/architecture for each comparison.

## 12. Final acceptance criteria

- [ ] Every accepted size change preserves features.
- [ ] Unused arboard image support is removed.
- [ ] Root Tonic features match client-only usage.
- [ ] Updater archive removal is implemented only if it is a net simplification.
- [ ] Tracing/Tokio pruning is kept only when measured and behaviorally safe.
- [ ] No replacement dependency adds equivalent or greater weight/complexity.
- [ ] Actual release binary deltas are recorded.
- [ ] `bash scripts/check.sh` passes.
- [ ] macOS/Windows/Linux CI remains green.

## 13. Suggested implementation commit

```text
phase-14d: reduce client dependency and binary footprint
```
