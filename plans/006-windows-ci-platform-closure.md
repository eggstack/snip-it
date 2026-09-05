# Plan 006: Windows CI and platform closure

Status: ready

Depends on: Plans 001–005

## Objective

Restore a green Windows platform job on current `main` without weakening the repository's existing cross-platform verification contract.

This is a corrective closure pass, not a feature expansion. The current distribution/MCP implementation should not be considered fully closed while `cargo check --workspace --all-targets` fails on `windows-latest` and prevents the Windows platform smoke test from running.

## Audit finding

The Windows failure is not specific to the latest MCP commit and should not be diagnosed by reverting MCP/startup/update work blindly.

Observed GitHub Actions history:

- the planning-only commit `80b4299130710a3160a07f4b7708fe11993d249d` already failed in `Platform smoke (windows-latest)` at `Check workspace`;
- Plans 001–005 continued to show the same Windows failure shape;
- Linux correctness is green;
- macOS platform smoke is green;
- the release workflow successfully built and smoke-checked the Windows x86_64 `snp` release executable at the older release tag used for validation.

Therefore begin by obtaining the actual Windows compiler diagnostic and determining whether the failure is production code, test-only compilation, feature gating, dependency/platform configuration, or a stale CI assumption. Do not infer the cause from the job status alone.

## Scope

Expected files are limited to the smallest set identified by the Windows compiler diagnostic, plus focused tests or CI documentation when needed.

Potentially relevant areas include:

```text
.github/workflows/ci.yml
Cargo.toml
snip-sync/Cargo.toml
src/**
snip-sync/src/**
tests/**
tests/support/**
```

Do not modify release targets, installer policy, startup architecture, MCP semantics, or updater behavior unless the Windows compiler error directly proves such a change is required.

## Non-goals

Do not:

- remove Windows from CI;
- mark the Windows job `continue-on-error`;
- replace `cargo check --workspace --all-targets` with a narrower command merely to make CI green;
- skip arbitrary test targets on Windows without identifying why they are not intended to compile there;
- add a new Windows service framework;
- add Windows ARM64 to the binary matrix in this pass;
- refactor unrelated platform code;
- upgrade dependency families broadly unless the compiler diagnostic establishes that as the narrow fix.

## Execution steps

### 1. Reproduce and capture the exact failure

Use the current `main` commit, not an older release tag.

Primary CI command:

```text
cargo check --workspace --all-targets
```

On Windows, capture the first root compiler/link/configuration error and enough dependent errors to establish causality. Do not plan around cascaded diagnostics.

Also compare with:

```text
cargo check --workspace --lib --bins
cargo test --test platform_smoke --features test-support --no-run
```

These comparisons answer whether the failure is in ordinary production targets or only in an integration/example/bench/test target compiled by `--all-targets`.

### 2. Classify the failure before editing

Choose exactly one primary class:

1. **Production portability defect** — a normal library/binary target does not compile on Windows.
2. **Test/support gating defect** — production targets compile but a test-only module or feature is referenced incorrectly by an all-target check.
3. **Platform dependency/configuration defect** — dependency features, build scripts, or target-specific dependency declarations are wrong for Windows.
4. **CI contract defect** — the command is valid in intent but CI lacks a prerequisite that the source tree legitimately requires.

Record the classification and exact diagnostic in the Plan 006 completion record.

### 3. Apply the minimum source fix

For a production portability defect, fix the platform implementation using existing project patterns (`#[cfg(...)]`, Windows APIs already present in the repository, or portable standard-library behavior).

For a test/support gating defect, correct the target/feature boundary. The repository already documents that only selected integration targets require `test-support`; preserve that architecture rather than enabling broad test-only behavior in normal builds.

For a dependency/configuration defect, prefer target-specific Cargo features/dependencies over runtime branching or duplicate implementations.

For a CI prerequisite defect, install only the missing prerequisite on the Windows job and document why it is required. Do not turn the smoke job into a full deployment environment.

### 4. Re-qualify code added by Plans 003–005

Even though the red job predates those commits, the fixed Windows job must compile the current implementations, including:

- `snip-sync` Task Scheduler/startup code;
- Windows process termination and kernel-lock behavior;
- binary-first updater Windows replacement/helper code;
- `packaging/install.ps1`-related Rust-facing contracts where applicable;
- `snp mcp serve` and client registration code.

Do not accept a fix that only makes the historical baseline compile while current additions remain cfg-hidden from intended Windows support.

### 5. Run the intended Windows smoke

The current CI sequence must reach and pass both steps:

```text
cargo check --workspace --all-targets
cargo test --test platform_smoke --features test-support
```

If the compiler fix affects a specific Windows-only behavior, add one focused regression test rather than a broad new platform suite.

### 6. Preserve Linux/macOS behavior

After the Windows correction:

```bash
bash scripts/check.sh
```

must remain green on Linux, and the ordinary macOS platform smoke must remain green in Actions.

## Likely traps

### Do not hide the diagnostic with cfg attributes

A `#[cfg(not(windows))]` wrapper is only valid when the behavior is genuinely unsupported and the public command/feature has an intentional Windows alternative. It is not an acceptable way to bypass a compile error in functionality that Plans 001–005 explicitly advertise on Windows.

### Keep test-support narrow

Do not globally enable `test-support` or `test-helpers` for production builds to satisfy an integration target. Repair the importing test or feature boundary instead.

### Preserve the Windows stack-size configuration

`.cargo/config.toml` raises the Windows thread stack because the large clap command enum exceeds the default stack during compilation. Do not remove or reduce that workaround as part of cleanup.

### Do not conflate binary release success with source-tree correctness

The release workflow built an older tagged package successfully on Windows. That does not prove current `main` passes `cargo check --workspace --all-targets`.

## Acceptance criteria

Plan 006 is complete only when all of the following are true:

1. The exact pre-fix Windows diagnostic and root-cause classification are recorded in the completion note.
2. `cargo check --workspace --all-targets` passes on `windows-latest` at the implementation commit.
3. `cargo test --test platform_smoke --features test-support` runs and passes on Windows rather than being skipped after a failed check.
4. Current Windows-targeted startup/update/MCP code compiles under that check.
5. Linux `bash scripts/check.sh` remains green.
6. macOS platform smoke remains green.
7. CI is not weakened with `continue-on-error`, broad target exclusions, or arbitrary cfg suppression.
8. Any new regression test is focused on the actual root cause.
9. `plans/README.md` is updated to mark Plan 006 complete only after the GitHub Actions run is green.
10. Plan 000 remains open until Plan 007 also completes its distribution validation.

## Handoff note

The first task is diagnostic retrieval, not code editing. The repeated job-status pattern only establishes that Windows all-target compilation is the blocker; it does not establish which source line is wrong. Make the smallest change justified by the compiler output and preserve the existing CI command if at all possible.
