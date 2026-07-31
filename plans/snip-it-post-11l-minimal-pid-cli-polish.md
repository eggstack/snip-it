# Post-Phase-11L — Minimal PID CLI Test and API-Surface Polish

Status: COMPLETE

Baseline: `9bc87bc703bc0312f087932661216249c6d0313e`

This is a narrow polish pass after `plans/snip-it-post-11l-lightweight-closure-pass.md`.

The production behavior from that pass is substantially complete. This plan does not reopen Phase 11L, the kernel-lock work, or the lightweight closure implementation. It adds only the small amount of direct CLI proof and API-surface clarification that was missing when the prior plan was marked complete.

---

## 1. Purpose

The current tree already contains the intended lightweight behavior:

- `snp sync repair` ignores persistent kernel-lock files;
- `snip-sync stop` parses structured and numeric legacy PID files;
- dead PID records are cleaned only after server-lock acquisition;
- live unrelated processes are refused unless `--force` is supplied;
- PID cleanup rereads the file and removes it only when the expected identity remains;
- `restart` sends legacy PID state through `cmd_stop`;
- malformed PID data produces an explicit error;
- Windows stop behavior remains unchanged.

The remaining issue is proof quality, not architecture.

The previous verification primarily compiled `snip-sync` and ran library tests. The legacy stop and restart branches live in `snip-sync/src/main.rs`, so three claims were not exercised directly at the command boundary:

1. a dead numeric PID file is cleaned by `snip-sync stop`;
2. a live unrelated numeric PID is refused and preserved;
3. `snip-sync restart` actually delegates legacy state through the stop path instead of proceeding to server startup.

There is also a minor API-presentation issue: `remove_pid_if_unchanged` is public because the package binary is a separate Rust crate from the package library. That is acceptable for this project, but the intent should be explicit and the helper should not be presented as a general-purpose public API.

This pass closes those points without introducing another test architecture.

---

## 2. Complexity budget

This pass is intentionally small.

### Expected production changes

At most one of the following:

- a documentation attribute and comment on `snip-sync::process::remove_pid_if_unchanged`; or
- no production-code change if the existing documentation is judged sufficient.

### Expected test changes

- one new Linux-only integration test file under `snip-sync/tests/`;
- exactly two command-level tests;
- no helper binary;
- no test feature;
- no mock process framework.

### Expected plan/status changes

- this plan;
- a short follow-up note in `plans/snip-it-post-11l-lightweight-closure-pass.md` after implementation;
- final SHA and verification record in this plan.

### Hard size limit

The implementation should normally remain below:

- 20 changed production lines;
- 180 lines in the new integration test;
- status/plan text excluded.

Exceeding this budget requires a concrete compile or portability reason. It is not permission to refactor the command layer.

---

## 3. Explicit non-goals

Do not add or change any of the following:

- the kernel-backed lock implementation;
- server-lock acquisition or lifetime;
- PID record schema;
- PID atomic-write implementation;
- process start-token logic;
- Windows process termination;
- `snp sync repair` behavior;
- auto-sync scheduling;
- transaction recovery;
- sync protocol behavior;
- TUI behavior;
- crates or dependencies;
- feature flags;
- helper binaries;
- CI jobs or matrices;
- release automation;
- failpoints;
- stress loops;
- generalized command test frameworks;
- generalized process-management abstractions;
- broad movement of functions from `main.rs` into the library.

Do not turn this into a production-hardening pass. The product is a small local TUI with a local/self-hosted sync server.

---

## 4. Execution rules for handoff

1. Implement the test commit first.
2. Do not alter production behavior merely to make the tests easier to write.
3. Use only `std::process::Command`, `std::fs`, and the existing `tempfile` development dependency.
4. Keep the integration test Linux-only because its isolated state-dir setup relies on `XDG_STATE_HOME`, and the command uses Unix signaling and `ps` process-name validation.
5. Use a real unrelated process such as `sleep`, not the test process itself, so process-name validation is deterministic.
6. Never invoke `--force` against a test-owned process.
7. Do not start a real `snip-sync` network server in these tests.
8. Do not add sleeps as the main assertion mechanism.
9. Do not rerun broad lock stress tests as part of implementation.
10. Stop when the exact closure checklist is satisfied.

---

# Workstream A — Add two direct CLI integration tests

## Goal

Prove the remaining user-visible legacy PID behavior through the compiled `snip-sync` binary.

## File

Create:

```text
snip-sync/tests/legacy_pid_cli.rs
```

The file must begin with:

```rust
#![cfg(target_os = "linux")]
```

Linux-only scope is deliberate:

- `XDG_STATE_HOME` gives the child process an isolated state directory;
- stop signaling is Unix-only;
- process-name validation invokes `ps`;
- Windows behavior is intentionally unchanged and already compile-covered by existing CI.

Do not create parallel macOS or Windows test implementations.

## Test setup

Use the compiled binary directly:

```rust
fn snip_sync_bin() -> &'static str {
    env!("CARGO_BIN_EXE_snip-sync")
}
```

Use a temporary root and derive the expected state directory:

```rust
fn isolated_command(root: &Path) -> Command {
    let mut command = Command::new(snip_sync_bin());
    command.env("XDG_STATE_HOME", root);
    command.env("HOME", root);
    command
}

fn state_dir(root: &Path) -> PathBuf {
    root.join("snip-sync")
}
```

Before each command:

1. create `state_dir(root)`;
2. write `state_dir(root).join("snip-sync.pid")`;
3. invoke the binary with the same `XDG_STATE_HOME` and `HOME` values.

Do not add a new production environment override.

## Test A1 — Dead legacy PID is cleaned by `stop`

### Setup

1. Create a temporary root.
2. Create the isolated state directory.
3. Write a numeric legacy PID known to be unavailable, for example:

```rust
const DEAD_PID: u32 = 99_999_999;
```

Do not use `u32::MAX`, because conversion to a signed Unix PID could produce a special negative value.

Write:

```text
99999999
```

to `snip-sync.pid`.

### Action

Run:

```text
snip-sync stop
```

### Exact assertions

Require all of the following:

- process exit status is success;
- the PID file no longer exists;
- stdout indicates stale/dead PID cleanup;
- stderr does not claim that a live unrelated process was refused;
- the persistent `snip-sync.server.lock` file may exist after the command and must not be treated as failure.

Do not assert that the state directory becomes empty.

### Purpose

This proves the real command path:

```text
LegacyPid -> not running -> acquire server lock -> reread -> remove unchanged PID
```

A direct parser or helper unit test is not a substitute.

## Test A2 — `restart` delegates legacy state to stop and preserves a live unrelated PID

### Setup

1. Spawn a real unrelated process:

```text
sleep 30
```

2. Record its PID.
3. Write that PID as numeric text to the isolated `snip-sync.pid`.
4. Keep the child handle so the test can terminate it during cleanup.

Use a small RAII cleanup guard or an explicit `kill`/`wait` sequence at the end. Do not leave the process running when an assertion fails; structure cleanup so it runs before final assertions where practical.

### Action

Run:

```text
snip-sync restart
```

without `--force`.

### Exact assertions

Require all of the following:

- command exit status is nonzero;
- stderr reports refusal because the PID does not appear to be a `snip-sync` process;
- the PID file still exists;
- its bytes remain exactly the original numeric PID plus newline;
- the unrelated `sleep` process is still running immediately after the command;
- no server startup message indicating successful serve initialization appears;
- terminate and wait for the `sleep` process during test cleanup.

### Purpose

This single test proves two missing claims without starting a server:

1. `restart` recognizes `LegacyPid` and calls `cmd_stop`;
2. the legacy stop path refuses a live unrelated process and preserves its PID record.

Do not add a separate network-server test.

## Test naming

Use clear names such as:

```rust
#[test]
fn stop_cleans_dead_legacy_pid_file()

#[test]
fn restart_refuses_live_unrelated_legacy_pid_and_preserves_file()
```

## Test failure diagnostics

Every process-status assertion should include stdout and stderr in the failure message:

```rust
assert!(
    output.status.success(),
    "status={:?}\nstdout={}\nstderr={}",
    output.status.code(),
    String::from_utf8_lossy(&output.stdout),
    String::from_utf8_lossy(&output.stderr),
);
```

Do not use permissive branches accepting multiple unrelated outcomes.

## Workstream A acceptance criteria

- one Linux-only integration test file exists;
- it contains exactly the two required command tests;
- dead legacy cleanup is exercised through the binary;
- restart delegation is exercised through the binary;
- live unrelated process refusal is exercised through the binary;
- the unrelated process remains alive and the PID file remains unchanged;
- no real server is started;
- no new dependency, feature, helper binary, or environment hook is added.

## Suggested commit

```text
post-11L: add focused legacy PID CLI tests
```

---

# Workstream B — Clarify the narrow helper API without refactoring

## Goal

Make the intent of `snip_sync::process::remove_pid_if_unchanged` explicit while avoiding a command-layer rewrite.

## Background

The function is currently public because:

- `snip-sync/src/main.rs` is a binary crate;
- `snip-sync/src/lib.rs` is a separate library crate;
- the binary accesses the library through the `snip_sync` crate boundary;
- `pub(crate)` in the library is not visible to the package binary.

Moving command code into the library solely to avoid one public function would add more complexity than it removes.

## Required decision

Keep the helper in `snip-sync/src/process.rs` unless a trivial existing private path already allows removal.

Preferred polish:

```rust
#[doc(hidden)]
pub fn remove_pid_if_unchanged(expected: &ParsedPidFile) {
    // existing implementation
}
```

Update its documentation to state:

- it exists for the package CLI binary;
- it is not a general ownership or lock-reclamation API;
- callers must hold the server singleton lock before invoking it;
- it rereads the PID file and removes only an unchanged structured or legacy record.

Recommended documentation shape:

```rust
/// CLI support helper: remove the PID file only if it still matches `expected`.
///
/// The caller must hold the server singleton lock. This function is public
/// only because the package binary and library are separate Rust crates; it is
/// hidden from generated API documentation and is not a lock-ownership API.
#[doc(hidden)]
pub fn remove_pid_if_unchanged(expected: &ParsedPidFile) {
    // unchanged body
}
```

Do not:

- move `cmd_stop` or `cmd_restart` into the library;
- create a command service abstraction;
- add a trait;
- expose the path-taking helper;
- change the function signature;
- change cleanup behavior;
- add another public type.

The existing unit test that verifies replacement PID preservation remains the required helper-level proof.

## Workstream B acceptance criteria

- helper behavior is unchanged;
- generated API docs no longer advertise the helper;
- the caller-lock precondition is explicit;
- no command code is relocated;
- no new public API is added;
- no new unit tests are required beyond the existing unchanged-record test.

## Suggested commit

This may be included in the CLI-test commit if it is only documentation and `#[doc(hidden)]`. Do not create a separate commit for a two-line annotation unless repository convention requires it.

---

# Workstream C — Correct the closure record without reopening architecture

## Goal

Record that the earlier implementation remains complete while this follow-up supplies the missing direct CLI proof.

## Update the prior lightweight plan

Update:

```text
plans/snip-it-post-11l-lightweight-closure-pass.md
```

Add a short note near the status or verification record:

```text
Follow-up CLI proof: `plans/snip-it-post-11l-minimal-pid-cli-polish.md`.
The production implementation remains closed; the follow-up adds direct binary-level tests for dead legacy cleanup and restart refusal/preservation.
```

Do not rewrite its implementation history.

Do not change Phase 11L status.

## Update this plan after implementation

Change:

```text
Status: READY FOR IMPLEMENTATION
```

to:

```text
Status: COMPLETE
```

Record:

- one exact final implementation SHA;
- the CLI-test commit SHA;
- the focused commands actually run;
- CI result only if observed for the exact implementation SHA.

Do not claim:

- exhaustive process lifecycle proof;
- all-platform command-level stop testing;
- server crash testing;
- stronger security properties;
- publication failpoint coverage.

## Suggested closure commit

```text
post-11L: close minimal PID CLI polish
```

---

## 5. Verification

Run this focused set only:

```bash
cargo fmt --all -- --check
cargo test -p snip-sync --test legacy_pid_cli -- --test-threads=1
cargo test -p snip-sync --lib -- --test-threads=1
cargo check -p snip-sync --all-targets
bash scripts/check.sh
```

Notes:

- The new integration target is Linux-only and will compile to zero tests on other platforms if included by broader commands.
- Do not rerun the full workspace test suite solely for this polish pass unless `scripts/check.sh` already does so.
- Do not rerun publish dry-runs.
- Do not add a link-check dependency.
- Existing CI topology remains unchanged.

The exact final implementation SHA should pass the existing repository CI. Do not add new CI jobs specifically for these two tests; the Linux correctness job should discover them through the existing test command if it runs package integration tests. If it does not, add the focused test invocation to the existing Linux job only when that is a one-line inclusion. Do not create a separate workflow or matrix.

---

## 6. Ordered commit sequence

### Commit 1 — Focused CLI proof and helper documentation

Expected files:

- `snip-sync/tests/legacy_pid_cli.rs`;
- optionally `snip-sync/src/process.rs` for `#[doc(hidden)]` and comment only.

Required result:

- dead legacy `stop` path directly tested;
- legacy `restart` delegation and unrelated-process refusal directly tested;
- helper intent clarified without refactoring.

### Commit 2 — Closure record

Expected files:

- this plan;
- `plans/snip-it-post-11l-lightweight-closure-pass.md`.

Required result:

- exact implementation SHA recorded;
- actual focused verification recorded;
- prior implementation history preserved;
- no overstatement of test scope.

Do not split this into additional cleanup, refactor, documentation, or CI commits.

## 7. Verification record

CLI-test implementation commit: `fff1a91` (`post-11L: add focused legacy PID CLI tests`).

The focused verification completed locally on 2026-07-31:

- `cargo fmt --all -- --check`;
- `cargo test -p snip-sync --test legacy_pid_cli -- --test-threads=1`;
- `cargo test -p snip-sync --lib -- --test-threads=1`;
- `cargo check -p snip-sync --all-targets`;
- `bash scripts/check.sh`.

The existing CI topology is unchanged. Remote CI is verified separately for
the pushed closure commit; no all-platform command-level stop claim is made
by this Linux-only proof.

---

## 8. Prohibited outcomes

The pass fails if it introduces any of the following:

- a new crate or dependency;
- a new feature flag;
- a new test helper binary;
- process mocks or injectable process backends;
- a new state-directory production override;
- a server startup integration harness;
- test loops intended as stress testing;
- changes to kernel locking;
- changes to PID schema or atomic publication;
- changes to Windows stop support;
- command functions moved wholesale into the library;
- a new public process-management abstraction;
- more than one new integration test file;
- more than two new CLI tests;
- new CI workflow files;
- automated publishing;
- reopening Phase 11L;
- broad claims of production-grade hardening.

---

## 8. Explicit closure criteria

All statements below must be true:

- [ ] `snip-sync/tests/legacy_pid_cli.rs` exists and is Linux-only;
- [ ] the file contains exactly two focused command tests;
- [ ] `snip-sync stop` with a dead numeric PID exits successfully;
- [ ] the dead numeric PID file is removed;
- [ ] cleanup occurs through the real compiled binary path;
- [ ] `snip-sync restart` with a live unrelated numeric PID exits nonzero;
- [ ] restart reports process-name refusal rather than starting the server;
- [ ] the live unrelated process remains running after refusal;
- [ ] the numeric PID file remains byte-for-byte unchanged after refusal;
- [ ] the test terminates and waits for its unrelated child process;
- [ ] the existing replacement-record preservation unit test remains present and passing;
- [ ] `remove_pid_if_unchanged` behavior is unchanged;
- [ ] the helper is hidden from generated API documentation or an equally small rationale is recorded;
- [ ] no command-layer refactor was introduced;
- [ ] no new dependency, crate, feature flag, helper binary, or production environment hook was added;
- [ ] no kernel-lock, server-lock, PID-schema, or Windows-stop behavior changed;
- [ ] focused formatting, tests, check, and `scripts/check.sh` pass;
- [ ] existing CI topology remains unchanged;
- [ ] one exact final implementation SHA is recorded in this plan;
- [ ] the prior lightweight closure plan links to this follow-up;
- [ ] Phase 11L remains closed;
- [ ] implementation stops after these criteria are satisfied.

When all criteria are met, this line of work is closed. Do not create another hardening or verification phase for these behaviors unless a real user-visible bug is reproduced.
