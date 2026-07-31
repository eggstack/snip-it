# Phase 12A — Secret Handling, Process Identity, and Startup Boundary Correctness

Status: COMPLETE

Baseline: `956e0123dacad0927f5122eb33db1ebc1852ad1d`

Roadmap: `plans/snip-it-phase-12-lightweight-correctness-footprint-roadmap.md`

This phase corrects concrete defects at the client/server process boundary before any auto-sync architectural simplification begins.

It is intentionally a defect-correction pass. It does not redesign synchronization, add security infrastructure, or expand CI.

---

## 1. Required outcomes

Implement all of the following:

1. Remove production retention of bearer API keys in `snip-sync` test instrumentation.
2. Correct Linux `/proc/<pid>/stat` start-time parsing in every current implementation.
3. Make Unix process-liveness checks treat `EPERM` as an existing process and `ESRCH` as absent.
4. Preserve non-UTF-8 executable paths when re-executing `snp` helpers.
5. Return a nonzero process status when the hidden auto-sync worker reports failure.
6. Make an existing malformed or unreadable `snip-sync` configuration fail startup rather than silently use defaults.
7. Add only focused tests for these behaviors.

Do not modify current auto-sync scheduling decisions, pending-state semantics, or the worker/executor architecture in this phase. Those belong to 12B and 12C.

---

## 2. Complexity budget

Expected production scope:

- `snip-sync/src/lib.rs`
- `snip-sync/src/main.rs`
- `snip-sync/src/process.rs`
- `snip-sync/src/server_lock.rs`
- `src/process_file_lock.rs`
- `src/auto_sync/spawn.rs`
- `src/main.rs`
- narrowly scoped shared helper modules if duplication can be removed without cross-crate coupling

Expected tests:

- unit tests adjacent to process parsing and configuration loading;
- at most one focused integration test file if command exit behavior cannot be tested locally;
- no helper daemon, mock process framework, or broad end-to-end server test.

Expected change size is normally below 450 production lines including refactoring and below 350 new test lines. Smaller is preferred.

---

## 3. Explicit non-goals

Do not:

- change API-key authentication format;
- change the gRPC protocol;
- rotate, encrypt, or migrate existing API keys;
- add a secret manager;
- add zeroization wrappers around every temporary request string;
- redesign server configuration precedence;
- add schema validation libraries;
- add a process-inspection dependency;
- introduce a shared crate solely for four small process helpers;
- rewrite server PID lifecycle or stop/restart behavior beyond corrected identity/liveness semantics;
- add new platform CI jobs;
- change auto-sync pending or backoff behavior;
- begin the one-helper migration.

---

# Workstream A — Remove production bearer-key capture

## Problem

`SnipSyncService::captured_auth_header` is described as test-only but is present in production builds. Every authenticated RPC calls `capture_auth_header`, which stores the first nonempty bearer value in a long-lived mutex before authentication completes.

This is unnecessary production secret retention and test-only state leakage.

## Files

Primary:

```text
snip-sync/src/lib.rs
snip-sync/src/main.rs
snip-sync/src/test_helpers.rs
```

Tests that construct `SnipSyncService` may also require mechanical updates.

## Required implementation

Use compile-time gating, not a runtime boolean.

Preferred shape:

```rust
pub struct SnipSyncService {
    // production fields...
    #[cfg(any(test, feature = "test-helpers"))]
    pub captured_auth_header: Arc<Mutex<Option<String>>>,
}
```

Separate extraction from capture:

```rust
fn request_api_key<T>(request: &Request<T>, body_api_key: &str) -> String {
    let api_key = extract_api_key(request, body_api_key);
    #[cfg(any(test, feature = "test-helpers"))]
    self.capture_test_auth_header(&api_key);
    api_key
}
```

The production compilation path must contain no field that stores a bearer value and no mutex operation for this assertion.

If `cfg(any(test, feature = "test-helpers"))` causes construction friction, add a test-only constructor/helper. Do not keep a production `Option` field merely to simplify struct literals.

## Acceptance criteria

- [x] A normal `cargo check -p snip-sync --all-targets` compiles with no `captured_auth_header` production field.
- [x] Existing auth-header regression tests continue to verify metadata behavior under `test-helpers`.
- [x] The production request path extracts the key once and does not retain an extra long-lived copy.
- [x] No API behavior changes for clients.
- [x] No new dependency is added.

---

# Workstream B — Correct Linux process start-token parsing

## Problem

The current Linux parsers remove `/proc/<pid>/stat` fields 1 and 2, then read index 18 from the remaining fields. Linux `starttime` is field 22, so the correct zero-based index after removing fields 1 and 2 is 19.

The defect appears in at least:

```text
src/process_file_lock.rs
snip-sync/src/process.rs
snip-sync/src/server_lock.rs
```

## Required implementation

Create a small parser function in each crate boundary rather than repeating raw index arithmetic.

Client example:

```rust
#[cfg(target_os = "linux")]
fn parse_linux_proc_start_token(stat: &str) -> Option<String> {
    let after_comm = stat.rfind(')')?;
    let fields: Vec<&str> = stat.get(after_comm + 2..)?.split_whitespace().collect();
    fields.get(19).map(|value| (*value).to_owned())
}
```

The server may share one internal helper between `process.rs` and `server_lock.rs`. Do not create a cross-workspace process-identity crate.

The parser must account for spaces and parentheses inside `comm` by locating the final `)` as the current code does.

## Focused tests

Add table-style parser tests with synthetic `/proc/stat` strings where:

- `comm` contains spaces;
- `comm` contains a closing parenthesis;
- field 21 and field 22 have deliberately different sentinel values;
- truncated input returns `None`;
- the returned token is exactly field 22.

Example intent:

```rust
assert_eq!(parse_linux_proc_start_token(sample), Some("START22".into()));
assert_ne!(parse_linux_proc_start_token(sample), Some("FIELD21".into()));
```

At least one Linux-only test may compare the helper result for the current process with a separately parsed `/proc/self/stat` fixture, but synthetic tests are the primary proof because they catch the index regression directly.

## Acceptance criteria

- [x] Every Linux start-token implementation reads post-`comm` index 19.
- [x] No duplicated unexplained numeric index remains in the touched files.
- [x] Tests distinguish field 21 from field 22.
- [x] macOS and Windows implementations are unchanged except for formatting or helper placement.
- [x] PID identity schemas remain unchanged.

---

# Workstream C — Correct Unix process liveness semantics

## Problem

Current Unix helpers generally use:

```rust
libc::kill(pid, 0) == 0
```

A `-1` result with `EPERM` means the process exists but cannot be signaled. Only `ESRCH` proves absence.

## Files

Search all current production uses of `kill(pid, 0)` or equivalent raw declarations, including:

```text
snip-sync/src/process.rs
src/auto_sync/execution_lock.rs
src/auto_sync/pending_lock.rs
src/auto_sync/lock.rs
```

Limit edits to actual liveness helpers. Do not redesign lock ownership in this phase.

## Required helper semantics

Preferred shape:

```rust
#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    let rc = unsafe { libc::kill(pid as i32, 0) };
    if rc == 0 {
        return true;
    }
    !matches!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH))
}
```

This conservatively treats `EPERM` and unknown errors as alive. PID zero and signed conversion behavior must remain conservative; do not send a signal to a process group.

If a helper already rejects PID zero, preserve that behavior explicitly.

## Tests

Directly testing `EPERM` portably is difficult without privilege changes. Use focused unit tests for any extracted errno-classification function:

```rust
assert!(classify_kill_zero_error(Some(libc::EPERM)));
assert!(!classify_kill_zero_error(Some(libc::ESRCH)));
assert!(classify_kill_zero_error(Some(libc::EINVAL)));
```

Do not add container, sudo, user-namespace, or privilege-manipulation tests.

## Acceptance criteria

- [x] `ESRCH` maps to absent.
- [x] `EPERM` maps to present.
- [x] Unknown errors fail conservatively as present.
- [x] No lock is reclaimed solely because the caller lacks signal permission.
- [x] No privilege-dependent integration test is added.

---

# Workstream D — Preserve executable paths exactly

## Problem

`src/auto_sync/spawn.rs` converts `current_exe()` to `String` using `to_string_lossy()` before passing it to `Command::new`. Unix executable paths may contain non-UTF-8 bytes; lossy conversion can change a valid path and make helper re-execution fail.

## Required implementation

Use the `PathBuf` directly:

```rust
let exe = std::env::current_exe().map_err(SpawnError::Spawn)?;
let mut cmd = Command::new(&exe);
```

Apply this to both worker and executor spawning while both paths still exist.

Remove `NoExecutable` if it is truly unreachable and only retained for the old conversion path; otherwise preserve it for a real call site. Do not add path encoding abstractions.

## Tests

A direct non-UTF-8 integration test is optional and should be added only if it is simple on Unix. The required proof is code-level preservation plus existing spawn tests/compilation.

Do not add platform-specific fixture binaries solely for this case.

## Acceptance criteria

- [x] No `to_string_lossy()` is used to launch the current executable.
- [x] `Command::new` receives `Path`/`OsStr` data.
- [x] Windows compilation remains valid.
- [x] No user-visible command changes.

---

# Workstream E — Make hidden worker failures observable

## Problem

`Commands::AutoSyncWorker` currently discards `WorkerOutcome::Failed` and exits through the generic success path.

## Required implementation

Map outcomes explicitly:

```rust
match worker::run(&state_dir) {
    WorkerOutcome::Success | WorkerOutcome::NothingToDo => {}
    WorkerOutcome::Failed => std::process::exit(<documented internal failure code>),
}
```

Prefer the existing general error or sync execution failure code. Do not invent a new public exit-code taxonomy for a hidden command.

This behavior remains useful during Phase 12C even if the internal subcommand is later renamed or simplified.

## Focused test

Use an existing test seam or an isolated corrupt state directory to make the hidden worker return `Failed`, then assert nonzero status. Do not start a real remote server solely for this assertion.

If the current command cannot be deterministically forced to fail without invasive test code, add a small unit-tested mapping function and defer binary-level proof to 12B, where corrupt pending state is already tested.

## Acceptance criteria

- [x] `WorkerOutcome::Failed` never exits zero.
- [x] `NothingToDo` remains a successful no-op.
- [x] No public CLI command exit behavior changes.

---

# Workstream F — Fail closed on malformed existing server configuration

## Problem

`snip-sync::Config::load()` logs an existing malformed or unreadable configuration and silently returns defaults. A typo can therefore start the daemon on default ports or with an unintended database path.

## Required implementation

Change configuration loading to return a result:

```rust
pub fn load() -> Result<Self, ConfigLoadError>
```

Required distinction:

- missing file after bootstrap: defaults are permitted;
- valid file: merge file values with environment overrides and defaults;
- existing unreadable file: return error;
- existing malformed TOML: return error with path and parser detail.

`ensure_config_file()` remains best-effort only where currently appropriate, but `serve()` must not proceed after a load failure.

Keep environment variable precedence unchanged.

A small local error enum is sufficient. Do not add `figment`, `config`, JSON schema, or a validation framework.

## Focused tests

Add tests for:

1. no file -> defaults;
2. valid partial file -> expected merge;
3. malformed existing file -> error;
4. unreadable existing file -> error where testable on the current platform;
5. environment override remains higher priority.

To avoid global environment races, either:

- test a pure `load_from(path, env_lookup)` helper; or
- serialize the small number of environment-mutating tests with the project’s existing pattern.

Prefer a pure helper if it requires little code.

## Acceptance criteria

- [x] Existing malformed config prevents `serve` startup.
- [x] Error output names the configuration path and parse/read reason.
- [x] Missing config still uses documented bootstrap/default behavior.
- [x] Environment precedence is unchanged.
- [x] No new dependency is added.

---

## 4. Recommended implementation order

1. Gate bearer capture and repair affected constructors.
2. Extract and test Linux start-token parsers.
3. Correct Unix liveness classification.
4. Remove lossy executable conversion.
5. Correct hidden-worker exit mapping.
6. Convert server config loading to `Result` and update call sites.
7. Run focused tests, then the bounded workspace checks.
8. Update this plan with implementation SHA and verification record.

Do not intermingle 12B pending-state changes in these commits.

---

## 5. Verification commands

Use the smallest commands that prove the touched paths:

```text
cargo fmt --all -- --check
cargo test -p snip-sync --lib -- --test-threads=1
cargo test -p snip-it process_file_lock --all-features -- --test-threads=1
cargo test -p snip-it auto_sync --all-features -- --test-threads=1
cargo check --workspace --all-targets --all-features
```

Run any new focused integration test directly by name.

At closure:

```text
bash scripts/check.sh
```

Do not require release builds, cargo-bloat, or broad crash suites in this phase.

---

## 6. Prohibited outcomes

This phase fails if it:

- leaves a production field containing captured bearer credentials;
- fixes one start-token parser but leaves another at index 18;
- treats `EPERM` as process absence;
- converts executable paths through UTF-8 for spawning;
- keeps worker failure exit zero;
- silently defaults after malformed existing config;
- adds a new process or configuration dependency;
- changes sync protocol or user-visible feature behavior;
- expands CI or release automation;
- grows into a generalized security audit.

---

## 7. Closure checklist

- [x] Workstream A complete.
- [x] Workstream B complete.
- [x] Workstream C complete.
- [x] Workstream D complete.
- [x] Workstream E complete.
- [x] Workstream F complete.
- [x] Focused tests pass.
- [x] `cargo check --workspace --all-targets --all-features` passes.
- [x] `bash scripts/check.sh` passes.
- [x] Plan records implementation SHA and exact verification commands.
- [x] No Phase 12B or 12C work was pulled forward unnecessarily.

## Implementation and verification record

Implementation commit: `111e99c`.

Verification completed locally on 2026-07-31:

- `cargo fmt --all -- --check`
- `cargo test -p snip-sync --lib --features test-helpers -- --test-threads=1`
- `cargo test -p snip-it process_file_lock --all-features -- --test-threads=1`
- `cargo test -p snip-it auto_sync --all-features -- --test-threads=1`
- `cargo test --workspace --all-features --lib -- --test-threads=1`
- `cargo check --workspace --all-targets --all-features`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `bash scripts/check.sh`
- `bash scripts/ci/test-production-seams.sh`

When all items are satisfied, mark this plan COMPLETE and proceed to Phase 12B. Do not open a follow-up hardening plan for these same behaviors unless a concrete regression remains.
