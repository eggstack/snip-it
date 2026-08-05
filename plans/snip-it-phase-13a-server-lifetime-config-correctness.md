# Phase 13A — Server Lifetime, Coordinated Shutdown, and Configuration Correctness

Status: COMPLETE

Roadmap: `plans/snip-it-phase-13-correctness-scope-reduction-roadmap.md`

Baseline: `b62d0f50078f7656eca3c9abf58e2ad290562029`

## 1. Objective

Correct the `snip-sync` server lifetime defect and make server configuration fail closed without expanding the deployment architecture.

This phase is release-blocking. The current server joins its long-lived HTTP and gRPC tasks inside a 30-second timeout. That timeout covers normal operation, so the server can return after approximately 30 seconds without a shutdown request. The same implementation also gives each service an independent Ctrl-C listener, making shutdown coordination ambiguous.

The target is a conventional two-service process:

```text
bind both listeners
start HTTP and gRPC services
wait indefinitely for one of:
  - a single process shutdown signal
  - unexpected HTTP service exit
  - unexpected gRPC service exit
request sibling shutdown
wait up to configured/fixed graceful timeout
flush optional rate-limit state
return success for requested shutdown, error for unexpected service failure
```

No daemon manager, supervisor framework, signal crate, or new runtime dependency is needed.

## 2. Scope

### In scope

- `snip-sync/src/main.rs` service lifetime and shutdown orchestration;
- a shared shutdown notification primitive using existing Tokio facilities;
- correct propagation of unexpected service/task errors;
- applying the 30-second timeout only after shutdown begins;
- preserving the server singleton lock and PID guard for the full actual server lifetime;
- strict parsing of environment overrides in `snip-sync/src/lib.rs`;
- consistent boolean parsing for relevant server environment variables;
- focused server lifetime, graceful shutdown, and config parsing tests;
- narrow documentation corrections in `snip-sync/README.md`, `architecture/server.md`, and `AGENTS.md` if their behavior descriptions change.

### Out of scope

- adding native TLS termination;
- changing default ports, bind addresses, or reverse-proxy guidance;
- replacing systemd, Docker, launchd, or cron examples;
- adding a resident watchdog or internal process supervisor;
- redesigning PID records or server locks;
- adding SIGHUP reload, hot configuration, or multiple server instances;
- adding new health endpoints, readiness probes, or metrics;
- changing authentication, database schema, rate limiting, or sync RPC behavior;
- broad refactoring of `snip-sync/src/main.rs` unrelated to service lifetime.

## 3. Current defect and required semantics

### 3.1 Lifetime timeout defect

The current pattern conceptually does:

```text
timeout(30 seconds, join(http_task, grpc_task))
```

Because both service tasks are intended to run indefinitely, the timeout expires during healthy operation. The correction must instead:

1. create one shutdown signal future or shared notification;
2. run both services with graceful-shutdown receivers derived from that source;
3. wait without a normal-operation deadline;
4. on signal or task failure, notify both services;
5. start the 30-second drain timeout only at that point;
6. report unexpected task failure rather than logging and returning success.

### 3.2 Single shutdown authority

Use one process-level signal listener. Acceptable bounded implementations include:

- `tokio::sync::watch<bool>`;
- `tokio::sync::broadcast` with one sender and two receivers;
- `tokio::sync::Notify` plus a separate reason/result channel;
- a small local helper built from existing Tokio primitives.

Do not add `tokio-util` solely for `CancellationToken` unless it is already present transitively and a direct dependency is demonstrably simpler than the standard Tokio primitives. The preferred implementation uses existing Tokio only.

### 3.3 Task result truthfulness

The orchestrator must distinguish:

- requested shutdown with both services draining successfully: success;
- requested shutdown with drain timeout: error or explicit nonzero process result;
- HTTP service unexpected return/error: notify gRPC, drain, return error;
- gRPC service unexpected return/error: notify HTTP, drain, return error;
- task panic/join error: notify sibling, drain, return error.

Do not swallow a service failure into a log-only success.

### 3.4 Rate-limit persistence ordering

If persistent rate limiting is enabled:

- service listeners stop first;
- final persistence is requested after no new requests can arrive;
- the existing bounded persistence wait remains;
- persistence failure is reported but must not hang shutdown indefinitely.

No new persistence mechanism is required.

## 4. Configuration parsing workstream

### 4.1 Problem

Environment overrides currently use `parse().ok()` and silently fall back to file/default configuration. Examples include ports, limits, database pool size, request timeout, message size, and rate-limit values.

This can cause the server to bind or operate with values different from the administrator’s explicit environment configuration.

### 4.2 Required design

Add a small typed environment parsing helper inside the existing configuration module. It should:

```rust
fn parse_env<T>(
    env: &impl Fn(&str) -> Option<String>,
    name: &'static str,
) -> Result<Option<T>, ConfigLoadError>
where
    T: FromStr,
    T::Err: Display;
```

Exact signature is flexible. Required behavior is not:

- missing variable -> `Ok(None)`;
- present and valid -> `Ok(Some(value))`;
- present and invalid -> `Err(ConfigLoadError::InvalidEnvironment { name, value, reason })`.

The error message must name the variable and supplied value. Do not include secrets in this mechanism; API keys are not parsed through this server config path.

### 4.3 Boolean parsing

Use one helper with explicit accepted values, preferably case-insensitive:

```text
true, 1, yes, on
false, 0, no, off
```

At minimum, preserve currently documented `true` and `1`. Unknown values must fail instead of silently evaluating false.

Apply consistently to server configuration booleans used in startup, including:

- `TLS_ENABLED`;
- `SNIP_SYNC_ALLOW_HTTP`;
- `CORS_ALLOW_ALL`;
- `PERSIST_RATE_LIMITS`.

If some values are intentionally startup flags rather than `Config` fields, reuse the same parser from a narrow shared module rather than duplicating semantics.

### 4.4 Range validation

Reject values that are syntactically valid but operationally nonsensical where the type alone is insufficient. Required minimums:

- ports must be nonzero unless zero is explicitly supported for tests through an internal/test path;
- database max connections must be at least 1;
- request timeout must be at least 1 second;
- gRPC max message size must be greater than a small protocol floor and fit the underlying API type;
- rate limit must be at least 1 when enabled;
- text/count limits must be nonzero.

Keep checks in one `Config::validate()` or equivalent constructor boundary. Do not add a schema library.

## 5. Likely files

Primary:

- `snip-sync/src/main.rs`
- `snip-sync/src/lib.rs`

Focused tests may live in:

- existing `snip-sync` unit test modules;
- one new integration target such as `tests/snip_sync_lifetime.rs` only if an existing target cannot host the process-level test;
- `snip-sync/src/lib.rs` config tests for injected environment closures.

Documentation only as needed:

- `snip-sync/README.md`
- `architecture/server.md`
- `AGENTS.md`

Do not modify client sync semantics, protocol definitions, transaction code, or CI topology in this phase.

## 6. Implementation order

### Workstream A — Extract service result model

1. Define a small internal reason/result enum, for example:
   - requested signal;
   - HTTP exited;
   - gRPC exited.
2. Ensure service futures return `Result<(), Error>` rather than converting all errors to log messages.
3. Preserve existing listener prebinding so partial startup still fails immediately.

### Workstream B — Add shared shutdown notification

1. Create one sender before spawning services.
2. Give each service its own receiver/future.
3. Install one Ctrl-C listener in the orchestrator.
4. Use `tokio::select!` over signal and service completion.
5. Notify both services exactly once after any terminal event.

### Workstream C — Bound only graceful drain

1. After notification, await both service tasks.
2. Wrap this drain await in the existing 30-second timeout.
3. Abort remaining tasks only after timeout.
4. Preserve singleton/PID guards until orchestration fully returns.
5. Perform final optional persistence after listener shutdown.

### Workstream D — Strict environment parsing

1. Add `InvalidEnvironment` and, if useful, `InvalidConfiguration` error variants.
2. Replace `parse().ok()` environment paths with typed helpers.
3. Add consistent boolean parsing.
4. Add range validation after env/file/default resolution.
5. Keep missing-file default behavior unchanged; malformed existing TOML must continue to fail.

### Workstream E — Documentation and closure

1. Document that the server runs until signaled or a service fails.
2. Document strict environment parsing and accepted boolean forms.
3. Remove any implication that a healthy server has a fixed runtime timeout.
4. Record the implementation SHA and focused verification in this plan.

## 7. Focused tests

### 7.1 Server lifetime regression

Add one process-level test that:

1. launches `snip-sync serve` with isolated temp paths and loopback ephemeral or reserved ports;
2. waits for `/health` success;
3. waits beyond the prior failure boundary, preferably 35–45 seconds for the focused test;
4. confirms `/health` still succeeds and the child is running;
5. sends the supported termination signal;
6. confirms clean process exit within the graceful timeout.

To keep routine CI fast, the long lifetime regression may be marked ignored for normal unit runs only if a shorter deterministic test proves timeout placement and `scripts/check.sh` invokes a bounded version. Prefer a direct 35-second regression if total CI remains acceptable. Do not create a soak test suite.

### 7.2 Unexpected service exit

Use a focused internal test seam or listener failure injection to prove:

- one service failure notifies the sibling;
- the orchestrator returns an error;
- no task remains detached.

Do not add a generalized failpoint framework.

### 7.3 Configuration tests

Table-driven tests must cover:

- valid numeric override wins over file/default;
- malformed numeric override returns `InvalidEnvironment`;
- out-of-range numeric value returns a useful error;
- accepted true/false spellings;
- unknown boolean value fails;
- missing variable still permits file/default resolution;
- malformed existing TOML remains fail-closed.

## 8. Verification commands

Run only:

```text
cargo fmt --all -- --check
cargo clippy -p snip-sync --all-targets -- -D warnings
cargo test -p snip-sync
cargo test --test <focused-server-lifetime-target> -- --test-threads=1   # only if added
cargo check --workspace --all-targets
```

At phase closure:

```text
bash scripts/check.sh
```

Do not run the full crash/failpoint release suite unless this phase modifies shared process-lock or transaction code, which it should not.

## 9. Acceptance criteria

- [ ] A healthy server remains running beyond 30 seconds.
- [ ] The normal operation wait has no arbitrary lifetime timeout.
- [ ] Only graceful shutdown/drain is bounded by the shutdown timeout.
- [ ] One process-level signal source coordinates both services.
- [ ] HTTP failure stops gRPC and returns an error.
- [ ] gRPC failure stops HTTP and returns an error.
- [ ] Task panic/join failure is not reported as success.
- [ ] Listener prebinding and partial-startup failure behavior remain correct.
- [ ] Final optional rate-limit persistence occurs after request serving stops.
- [ ] Invalid numeric environment values fail with variable and value named.
- [ ] Invalid boolean environment values fail rather than silently falling back.
- [ ] Missing variables and valid file/default configuration retain current behavior.
- [ ] No new dependency, daemon, service manager, signal framework, endpoint, or CI job is introduced.
- [ ] Focused tests and `bash scripts/check.sh` pass.
- [ ] Documentation reflects the corrected lifetime and configuration behavior.

## 10. Stop conditions

Stop and amend the plan rather than expanding scope if:

- the correction appears to require a new service supervisor abstraction;
- tests require production-only process-control hooks broader than one narrow seam;
- native TLS, configuration reload, or a database migration becomes entangled;
- a proposed dependency is added solely for cancellation/shutdown;
- server lifetime correctness cannot be isolated from unrelated client changes.

Those conditions indicate implementation drift, not a need for broader architecture.

## 11. Completion record

Status: COMPLETE

Implementation commit: `7e0d064` — Phase 13A: Fix server lifetime defect and strict config parsing

Corrective commit: `5d37fa7` — Phase 13G: Fix sync batching, server shutdown, and config validation

Verification:
- `bash scripts/check.sh`: PASS

Acceptance criteria: All items satisfied. 13G corrected residual shutdown and config-validation defects.

Release-blocking: No (cleared by 13G)