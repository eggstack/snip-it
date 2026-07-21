# Phase 05A: Deterministic End-to-End Test Architecture

## Authority and baseline

This plan supersedes:

```text
plans/snip-it-correctness-05-end-to-end-test-architecture.md
```

Implement after Phase 04A command/recovery semantics are stable. Harness scaffolding may begin earlier. Baseline implementation commit: `ff506f5934957c4fd989224a6f0e0cf10f907567`.

## Purpose

Build a reusable, deterministic test system that proves `snip-it` behavior across real CLI processes, detached workers, supervised executors, the real sync protocol, filesystem transactions, credential boundaries, and platform-specific process semantics.

The current suite contains substantial unit and integration coverage, but this phase raises the evidence standard: critical guarantees must be proven through observable effects and exact counts, not source scanning, marker disappearance, broad timing windows, or acceptance of contradictory outcomes.

## Required outcomes

1. A real-binary mutation can be traced through worker, executor, real server request, remote state effect, local result, status update, and pending clear.
2. A no-op executor returning success fails mandatory CI.
3. Exact debounce, follow-up, backoff, and scheduling attempt counts are testable.
4. Mutual exclusion is proven across every sync entry-point pair.
5. Timeout, termination, reap, and lock lifetime are proven on Unix and Windows.
6. Crash windows have explicit expected recovery states.
7. Test environments never use the developer’s real config, libraries, keychain, server, or ports.
8. Required correctness tests are deterministic and not ignored.
9. Failure artifacts are useful, bounded, and secret-scanned.

## Non-goals

Do not create:

- a production test-control server;
- production fault-injection CLI flags;
- a second release binary;
- a permanent local daemon;
- broad UI snapshot testing unrelated to correctness;
- high-iteration soak tests in ordinary PR CI.

Long-running soak/fuzz jobs may be separate, but required contracts must run in normal CI.

---

## Workstream A — Unified isolated test environment

Create a shared test-support module or unpublished crate, recommended shape:

```text
tests/support/environment.rs
crates/snip-test-support/   # only if workspace separation is justified
```

Recommended builder:

```rust
let env = TestEnvironment::builder()
    .with_isolated_home()
    .with_fake_credentials()
    .with_recording_server()
    .with_auto_sync_policy(TestPolicy::fast())
    .build()?;
```

The environment must own:

- temporary HOME and XDG/config/data/cache/state roots;
- Windows equivalents for application data paths;
- isolated library/index/config files;
- deterministic initial clocks/timestamps where possible;
- a credential provider that never touches the real OS keyring;
- dynamically allocated loopback ports;
- real built `snp` binary path;
- real or in-process `snip-sync` service using production protocol handlers;
- SQLite/database state isolated per test;
- worker/executor event and log sinks;
- process registry for cleanup;
- unique test identity and sentinel secret;
- platform-safe path quoting;
- cleanup verification that no child remains alive.

### Environment isolation assertions

Every process invocation must explicitly set all path variables required by the application. Add a guard test that fails if a test resolves the normal user config directory.

The harness must not:

- rely on test ordering;
- share fixed ports;
- share global config files;
- store credentials in the user keychain;
- leave detached workers after test completion;
- rely on the current working directory unless explicitly part of the contract.

---

## Workstream B — Credential abstraction for tests

The existing product uses keychain/plaintext fallback behavior. Add an internal credential-provider boundary suitable for production and tests.

Recommended trait:

```rust
pub trait CredentialProvider {
    fn load_api_key(&self) -> Result<Option<SecretString>, CredentialError>;
    fn store_api_key(&self, value: &SecretString) -> Result<(), CredentialError>;
    fn delete_api_key(&self) -> Result<(), CredentialError>;
    fn revision(&self) -> Result<Option<String>, CredentialError>;
}
```

Production adapters may use the current keychain behavior. Tests use a private file or in-memory provider inherited safely by child processes.

Requirements:

- no raw key in argv;
- child processes resolve the same isolated provider;
- provider can inject unavailable/locked/write-failure states;
- credential revision changes can be tested;
- sentinel values are removable and scanned from artifacts;
- test provider is unavailable in ordinary release behavior unless behind a safe nondefault mechanism.

If a test-only environment variable selects the provider, production builds must reject or ignore it unless compiled with a nondefault `test-support` feature.

---

## Workstream C — Controllable recording sync server

Extend `snip-sync` test helpers or create an unpublished test adapter around the real service implementation.

Required recorded events:

```text
server_started
register_started/completed
health_started/completed
list_started/completed
push_started/completed
pull_started/completed
merge_started/completed
request_failed
server_stopped
```

Each event should include:

- monotonic sequence number;
- operation kind;
- client/device identity;
- library identity where applicable;
- request start/end timestamp;
- result class;
- remote revision before/after;
- no plaintext snippet content or credentials.

Required controls:

- bind port 0;
- block at named barriers;
- release barrier from test process;
- return authentication failure;
- return configuration/protocol rejection;
- close connection/reset;
- delay until released;
- hang until process timeout;
- return conflict;
- return partial result;
- return malformed/truncated response where protocol layer permits;
- restart with persistent database;
- inspect remote revision/ciphertext count;
- measure maximum concurrent canonical sync operations.

Production server artifacts must not expose these controls.

---

## Workstream D — Cross-process event sink

Add a bounded test-only event channel for worker/executor lifecycle evidence.

Preferred simple design:

- private append-only JSON-lines file in the isolated state directory;
- opened with restrictive permissions;
- each write is one bounded event record;
- enabled only by test-support configuration;
- event writes are best-effort and must not alter production correctness behavior.

Event schema:

```json
{
  "schema": 1,
  "seq": 1,
  "component": "worker",
  "event": "execution_lock_acquired",
  "pid": 123,
  "generation": 42,
  "at_unix_ms": 0
}
```

Required lifecycle events:

- parent mutation committed;
- pending generation recorded;
- schedule decision;
- worker spawned;
- execution lock acquisition result;
- debounce started/restarted/completed/max-delay;
- preflight result;
- executor spawned;
- canonical sync entered;
- executor exit code/failure class;
- termination requested;
- force kill requested;
- executor reaped;
- status write result;
- conditional clear result;
- worker exit.

Do not use this channel as the sole proof of server effect. It supplements server and filesystem evidence.

---

## Workstream E — Barrier and failpoint framework

Provide named test-only barriers/failpoints at correctness-critical boundaries:

```text
after_local_commit
before_pending_record
after_pending_record
before_schedule_decision
after_worker_spawn
before_execution_lock
execution_lock_acquired
before_debounce
after_debounce
before_preflight
after_preflight
before_executor_spawn
after_executor_spawn
before_canonical_sync
after_remote_success
before_local_persist
after_local_persist
before_executor_exit
after_executor_exit
before_status_write
after_status_write
before_pending_clear
after_pending_clear
before_worker_exit
```

Controls must permit:

- wait until reached;
- release;
- force process exit/crash at the point;
- inject a typed failure;
- inspect current local/remote/control state before release.

Failpoints must be absent or inert in production release builds.

---

## Workstream F — Headline real-effect regression

Replace or strengthen the existing closure test so it proves this exact sequence:

1. Start an isolated real protocol server with recorded remote revision `R0`.
2. Register/configure a real isolated `snp` client.
3. Enable auto-sync with deterministic policy.
4. Perform a real local mutation through the `snp` binary.
5. Observe one pending generation `G`.
6. Observe one worker and one executor lifecycle.
7. Observe the server receive the required operation.
8. Observe remote revision change from `R0` to `R1` or equivalent server-side state effect.
9. Observe executor success.
10. Observe status success for generation `G`.
11. Observe conditional pending clear for generation `G`.

Assertions:

- remote effect occurs before pending clear;
- exactly one attempt occurs for the single mutation;
- pending clear is impossible if server recording is disabled or executor is replaced with a no-op test fixture;
- status-file existence alone is insufficient;
- marker absence alone is insufficient;
- every success branch validates status content and server effect.

Add a mutation test fixture or compile-time test mode that deliberately substitutes a no-op executor. The headline test must fail under that mode. Do not ship this mode in release artifacts.

---

## Workstream G — Exact debounce and scheduling matrix

Required scenarios with exact counts:

1. Debounce zero: one mutation -> exactly one server attempt.
2. Positive debounce: no attempt before deadline; exactly one after.
3. Twenty mutations inside quiet window -> twenty generation increments, one worker spawn decision when applicable, exactly one server attempt after final quiet period.
4. Mutation in preflight window -> restart quiet period, no stale attempt.
5. Continuous mutations until max delay -> one forced attempt for latest generation.
6. Marker removed during debounce -> zero attempts.
7. Mutation during active sync -> first attempt completes, exactly one follow-up attempt for newer generation.
8. Backoff active -> mutations increment generation, zero worker spawns.
9. Backoff expires -> one recovery request creates one spawn and one attempt.
10. Attention required -> mutations increment pending, zero automatic spawns.
11. Relevant config/credential revision change -> one released scheduling attempt.
12. Startup recovery -> no generation increment and exact spawn decision.

Use an injected spawn counter and recording server. Do not accept `>= 1`.

---

## Workstream H — Failure-class contract matrix

For every `FailureClass`, inject the failure through the closest real boundary and assert:

- executor exit code;
- worker reconstructed class;
- retry disposition;
- status class and attempted generation;
- next-attempt timestamp or attention flag;
- pending preservation;
- parent local mutation success;
- foreground retry exit behavior;
- no secret leakage.

Required classes:

```text
deferred_disabled
deferred_not_configured
transient_network
transient_timeout
authentication
configuration
conflict
partial
local_persistence
credential_store
internal
```

Also test unknown/signal exit and status-write failure.

Internal failures must escalate according to the bounded retry budget exactly.

---

## Workstream I — Sync direction and client contract

With two isolated clients and one real server, prove:

- registration;
- initial push;
- initial pull;
- bidirectional merge;
- already-current no-op;
- multiple libraries;
- stable snippet/library identities;
- configured Push performs no pull;
- configured Pull performs no upload;
- Bidirectional performs required phases;
- foreground overrides use the same resolver;
- detached and foreground semantics match;
- local-only fields remain local;
- encryption leaves server payload opaque;
- server restart preserves expected state;
- corrupted remote payload fails safely;
- delete semantics are explicit and tested.

Success must inspect both remote state and resulting local files.

---

## Workstream J — Mutual exclusion matrix

Barrier-drive every meaningful pair:

```text
worker vs worker
worker vs foreground sync
worker vs cron
worker vs run --sync
worker vs clip/search/select explicit sync paths
foreground vs foreground
foreground vs cron
cron vs cron
recovery worker vs mutation worker
```

For each:

1. First operation enters canonical sync and blocks.
2. Start second operation.
3. Assert maximum concurrent canonical sync count is one.
4. Assert second operation follows documented wait/defer/error behavior.
5. Release first.
6. Verify order, status, and pending state.

Tests must use real concurrent processes, not sequential command construction.

---

## Workstream K — Timeout and process lifecycle

### Unix requirements

- worker lock acquired;
- executor child PID recorded;
- timeout reached through injected hanging server;
- SIGTERM sent;
- configured grace observed;
- SIGKILL sent if child ignores SIGTERM;
- direct child reaped;
- no zombie remains;
- execution lock remains until reap;
- pending remains;
- later retry succeeds.

### Windows requirements

- worker detaches without unwanted console;
- executor remains supervised;
- process handle liveness is accurate;
- graceful/force termination behavior matches implementation;
- child is confirmed exited;
- stale lock is reclaimable after forced worker death;
- file sharing semantics do not produce permissive skips;
- pending remains and later recovery succeeds.

If process descendants are possible, test group/job termination. Otherwise prove and document that executor sync does not spawn descendants.

---

## Workstream L — Crash-window recovery table

For each failpoint, commit an expected-state table containing:

- local library state;
- pending generation;
- status state;
- remote revision;
- lock artifacts;
- recovery action;
- expected next outcome.

At minimum test crashes:

- after local commit before pending record;
- after pending record before worker spawn;
- after lock acquisition;
- after remote success before local persistence;
- after local persistence before executor success;
- after executor success before status write;
- after status success before pending clear;
- during pending clear;
- after pending clear before worker exit.

Where a window cannot be made fully durable without a broader transaction, document the residual behavior and ensure recovery is conservative.

---

## Workstream M — Local command contract suite

Use real binaries to protect non-sync behavior before Phase 06–08 refactoring:

- positional and stdin command ingestion;
- empty/NUL/invalid UTF-8/size limits;
- editor workflows and exit handling;
- canonical/legacy TOML loading;
- variable/default/choice parsing;
- list/filter/sort stability;
- TUI cancel and terminal restoration;
- import/export/Pet compatibility;
- library create/delete/primary selection;
- usage tracking;
- shell integration output;
- help/version/update dry paths;
- stdout/stderr purity for existing machine modes.

These tests must run without a sync server.

---

## Workstream N — Package and installation evidence

Test packaged artifacts, not only workspace binaries:

```bash
cargo package -p snip-it
cargo package -p snip-sync
```

Unpack/build/install and verify:

- package compiles independently;
- expected assets included;
- no source-tree-only runtime dependency;
- `cargo install --path` smoke test;
- hidden worker/executor subcommands available to current-exe re-exec;
- process detachment works from installed path;
- shell completions generate;
- Homebrew/release archive layout tests where feasible;
- no test-support fault controls in release package.

---

## Workstream O — CI partitioning and artifact policy

Required jobs:

```text
fmt
clippy-all-targets-features
unit
local-integration
sync-real-server
auto-sync-process
crash-recovery
package-install
secret-sentinel
linux
macos
windows
```

Rules:

- job-level timeouts;
- bounded waits in every process test;
- no blanket retry wrapper;
- preserve first-failure evidence;
- upload only sanitized logs/events/databases on failure;
- scan artifacts for sentinel credentials and snippet payloads before upload;
- difficult Windows tests receive native assertions, not unconditional skip;
- ignored tests allowed only for optional soak, never required invariants.

---

## Test-quality audit

Remove or rewrite tests that:

- assert only no panic;
- accept marker present or absent without proving equivalent effects;
- infer sync from status-file existence;
- infer remote work from executor exit zero;
- accept `>= 1` attempts;
- use elapsed-time thresholds as sole correctness evidence;
- do not actually run processes concurrently;
- skip Windows without replacement coverage;
- inspect source text when behavioral evidence is available;
- rely on the same mock for both implementation and assertion.

Structural tests may remain for architecture boundaries, but they cannot substitute for behavioral tests.

---

## Recommended implementation sequence

1. Add isolated environment and credential provider.
2. Add recording server and event sink.
3. Add barrier/failpoint framework.
4. Replace headline executor regression with server-effect proof.
5. Add exact debounce/scheduling/failure matrices.
6. Add direction/client and mutual-exclusion suites.
7. Add timeout/process and crash-window suites.
8. Consolidate local command contract tests.
9. Add package/install verification.
10. Partition CI and audit permissive tests.
11. Document harness and write `plans/snip-it-correctness-05a-status.md`.

## Required verification

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --release --workspace
cargo package -p snip-it
cargo package -p snip-sync
```

## Exit criteria

Phase 05A is complete only when:

- real remote effect is proven before pending clear;
- a no-op executor fails mandatory CI;
- debounce, follow-up, backoff, and spawn counts are exact;
- all failure classes preserve intent according to contract;
- every sync entry-point pair is serialized;
- timeout termination/reap and stale-lock recovery are proven on Unix and Windows;
- crash windows have committed recovery evidence;
- package/install behavior is tested;
- no required test is ignored or permissive;
- failure artifacts are bounded and secret-scanned;
- Linux, macOS, and Windows jobs pass;
- no production daemon, helper service, or exposed fault-injection API was introduced.