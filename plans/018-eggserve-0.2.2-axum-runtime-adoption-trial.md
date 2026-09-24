# Plan 018: EggServe 0.2.2 Axum-preserving HTTP runtime adoption trial

Status: ready

Depends on: Plan 003 (complete), Plan 017 (complete)

## Objective

Attempt a narrow replacement of the current `axum::serve` HTTP runtime used by
`snip-sync` with the published EggServe runtime while preserving the existing
Axum 0.8 application/router surface.

This plan is deliberately a runtime-ownership experiment, not an HTTP API
rewrite.

The required first trial is:

~~~text
current
-------
pre-bind HTTP TcpListener
        |
        v
axum::serve(listener, Router)
        |
        v
snip-sync lifecycle orchestration

trial
-----
pre-bind HTTP TcpListener
        |
        v
eggserve-server 0.2.1
        |
        v
eggserve-core 0.2.2 TowerToEggserve
        |
        v
existing Axum 0.8 Router
~~~

Tonic continues to own the separate gRPC listener. TLS termination remains
external. The existing `/health` and `/metrics` handlers, CORS policy,
security-header middleware, database logic, metrics registry, server lock,
startup/lifecycle CLI, and updater transport remain application-owned.

The trial may be retained only when the measured production cost remains
consistent with this repository's lightweight-server policy. Otherwise revert
the EggServe implementation cleanly, record the result, and allow Plan 019 to
test the smaller direct leaf-runtime path independently.

## Research findings

### Current snip-sync server shape

At planning baseline `31b7d83e7f1584110438e31aa01ef23181229acd`:

- `snip-sync` is version 0.1.6 and Rust 1.94.
- Tonic owns the gRPC listener.
- Axum owns a separate HTTP listener.
- both listeners are bound before either service starts, so bind failure cannot
  leave a half-started daemon;
- the HTTP application contains only:
  - `GET /health`, returning version/status JSON and HTTP 503 when the
    database ping fails;
  - `GET /metrics`, hidden behind optional Basic authentication;
  - CORS middleware;
  - `x-content-type-options: nosniff`;
  - `x-frame-options: DENY`;
  - `cache-control: no-store`;
- `snip-sync` deliberately does not terminate TLS;
- the process-level orchestrator waits for Ctrl-C/SIGTERM, gRPC completion, or
  HTTP completion and then drains both services with a bounded deadline.

The existing topology is therefore already compatible with a transport-only
HTTP runtime replacement. Do not merge HTTP and gRPC onto one listener.

### What EggServe 0.2.2 actually published

The coordinated source tree reports workspace version 0.2.2, but crates.io
publication is intentionally mixed:

- `eggserve-core = 0.2.2`;
- `eggserve-server = 0.2.1`;
- `eggserve-primitives = 0.2.0`;
- `eggserve-static = 0.2.0` through the core composition crate.

EggServe release evidence in
`release/plan-275-http-tower-adapter-patch-publication-closure.md` records
that `eggserve-core 0.2.2` was published on 2026-09-24 with the repaired
HTTP/Tower adapter. A registry-only consumer with no path/git patches resolved
that exact graph and passed an Axum 0.8 runtime proof.

Do not incorrectly pin `eggserve-server = 0.2.2`; that crate version was not
published as part of the 0.2.2 patch.

### Why 0.2.2 unblocks this consumer

EggServe 0.2.1 introduced the direct downstream-embedding pieces needed by
snip-sync:

- `ServerBuilder::from_listener(TcpListener)`;
- `ServerHandle::into_parts()`;
- cloneable `ServerControl`;
- cancellation-safe borrowed `ServerCompletion::wait(&mut self)`;
- typed terminal propagation through `ServerError::Terminal`;
- `disable_connection_total_timeout()`.

EggServe core 0.2.2 then repaired the optional `http-interop` / `tower`
adapter after the canonical request body moved to the primitives crate.

The upstream registry-only 0.2.2 consumer specifically proves:

- `TowerToEggserve<axum::Router>` compiles against published crates;
- pre-bound listener adoption;
- streaming request and response handling;
- middleware execution;
- duplicate response headers;
- disconnect cancellation;
- external `ServerControl::shutdown()`;
- clean typed `ServerCompletion::wait()`.

This removes the compatibility blocker from the earlier EggServe evaluation.

### Dependency cost remains unknown for snip-sync

The compatibility route requires `eggserve-core` with feature `tower`.
`eggserve-core` also unconditionally composes `eggserve-static`, even
though snip-sync does not serve files.

LTO may eliminate most unused machine code, but the dependency/build graph is
broader than the direct leaf runtime. The repository has already rejected a
technically sound dependency migration when it grew the `snip-sync` release
binary by 34.23% in Plan 017. EggServe must pass an equivalent controlled
measurement instead of being accepted on architectural appeal alone.

## Scope

### In scope

1. Take a fresh current-server baseline.
2. Add the exact published EggServe compatibility dependency set.
3. Preserve the existing Axum Router and its handlers/middleware.
4. Replace only HTTP listener/runtime ownership.
5. Preserve the pre-bind-both-listeners startup invariant.
6. Integrate EggServe's typed control/completion with the existing two-service
   process supervisor.
7. Add focused socket-level regression coverage for the public HTTP contract.
8. Compare release size and dependency graph against the baseline.
9. Keep or revert the compatibility implementation mechanically according to
   the gate below.
10. Record enough evidence for Plan 019 to compare the direct leaf path.

### Explicit non-goals

Do not use this plan to:

- replace Tonic;
- move gRPC traffic through EggServe;
- combine the two listeners;
- enable EggServe HTTP/2 or HTTP/3;
- enable EggServe TLS;
- change reverse-proxy/TLS deployment guidance;
- change `snip-sync update` or its intentional curl transport;
- add a generic HTTP abstraction;
- add a new workspace crate;
- redesign `AppState`, database, metrics, or auth;
- rewrite `/health` or `/metrics` into native EggServe handlers;
- remove Axum or `tower-http`;
- introduce a second process supervisor;
- add new release workflows or CI jobs;
- expose additional HTTP endpoints.

The goal is to answer one question first: can EggServe own the existing HTTP
runtime at acceptable cost without disturbing application behavior?

## Part A — freeze the baseline

Before editing dependencies or server code, record:

~~~sh
git rev-parse HEAD
rustc --version --verbose
cargo --version
cargo build --release -p snip-sync --bin snip-sync
wc -c target/release/snip-sync
cargo tree -p snip-sync
cargo tree -p snip-sync -e features
~~~

Use the same host, target, toolchain, linker, release profile, Cargo target
directory policy, and environment for every A/B comparison.

Plan 017's historical reference is:

~~~text
snip-sync curl/current-server baseline: 3,833,152 bytes
~~~

That number is context only. Do not use it as the acceptance baseline when a
fresh same-environment build can be taken.

Also record direct current dependencies relevant to the HTTP stack:

~~~text
axum
tower
tower-http
tokio
tonic
~~~

No dependency cleanup belongs in this baseline phase.

## Part B — add the published compatibility graph narrowly

Use exact direct pins for the crates whose public APIs this consumer invokes:

~~~toml
eggserve-core = { version = "=0.2.2", default-features = false, features = ["tower"] }
eggserve-server = { version = "=0.2.1", default-features = false }
~~~

Retain the existing `axum = "0.8"` and `tower-http` dependencies.

The expected registry graph includes `eggserve-primitives 0.2.0` and
`eggserve-static 0.2.0` transitively through core. Do not add them as direct
dependencies in this plan unless compilation of a public API genuinely
requires it.

Refresh the lockfile narrowly. Inspect the resulting EggServe packages and
features. The consumer must not enable:

- `http2`;
- `http3`;
- `tls`;
- `python-bindings-internal`;
- any static-file application behavior in snip-sync.

The `tower` feature necessarily enables `http-interop`,
`tower-service`, and `tower-layer`; that is the intended compatibility
surface.

## Part C — preserve the existing application Router

Keep construction of the existing Axum Router behaviorally intact:

~~~text
/health
/metrics
security_headers_middleware
CorsLayer
AppState
~~~

Do not move auth, CORS, JSON formatting, Prometheus encoding, or database
health logic into EggServe in Plan 018.

Run the Router through:

~~~text
TowerToEggserve<axum::Router>
~~~

using the public 0.2.2 adapter.

Because the HTTP surface has no documented request bodies, use an explicit
request-body policy rather than silently inheriting an adapter default.
Preferred initial policy:

~~~text
RequestBodyPolicy::Reject
~~~

This intentionally rejects non-empty bodies on the health/metrics control
surface before handler execution. Treat that as a hardening of an undocumented
input shape, not as permission to change method routing. Add a regression test
so the behavior is deliberate. If Axum CORS preflight qualification proves
that `Reject` interferes with legitimate bodyless OPTIONS handling, fix the
specific composition; do not broaden body acceptance without evidence.

## Part D — construct EggServe from the already-bound listener

Preserve this ordering:

~~~text
resolve grpc/http addresses
bind grpc listener
bind http listener
only then start either service
~~~

The EggServe server must adopt the existing Tokio listener through
`ServerBuilder::from_listener(http_listener)`. It must not bind the HTTP
address a second time.

Configure only values required to preserve the daemon's actual lifecycle
contract.

Required:

- bind metadata consistent with the resolved HTTP address;
- `disable_connection_total_timeout()` so healthy keep-alive connections are
  not killed after EggServe's 60-second default total lifetime;
- graceful shutdown bounded consistently with the existing snip-sync drain
  budget;
- request-body hard ceiling compatible with the selected Reject policy.

Do not enable TLS/H2/H3.

Do not mechanically expose every EggServe runtime knob in snip-sync config.
A migration is not a reason to add configuration surface.

EggServe's remaining bounded H1 defaults (header read, handler, body,
keep-alive idle, response-write, parser/header limits, and admission limits)
may remain runtime-owned unless a current snip-sync contract demonstrably
requires an override. Record any behavior change introduced by those bounds;
do not silently add user-facing knobs for them.

## Part E — integrate typed supervision without adding an abstraction framework

Do not wrap EggServe in a second daemon manager.

Update the existing `snip-sync::orchestration` module so it can supervise:

~~~text
gRPC:
    JoinHandle<Result<(), tonic::transport::Error>>

HTTP:
    eggserve_server::ServerControl
    eggserve_server::ServerCompletion
~~~

Use EggServe's borrowed, cancellation-safe `ServerCompletion::wait(&mut self)`
directly in the first-terminal-event select.

Required lifecycle behavior:

1. Ctrl-C/SIGTERM:
   - broadcast shutdown to Tonic;
   - call `ServerControl::shutdown()`;
   - drain both services.
2. unexpected gRPC completion:
   - classify it;
   - call EggServe shutdown;
   - drain HTTP;
   - return a non-clean process outcome.
3. unexpected EggServe completion:
   - preserve the typed EggServe result/error detail;
   - broadcast Tonic shutdown;
   - drain gRPC;
   - return a non-clean process outcome.
4. clean requested shutdown:
   - both services complete cleanly;
   - no detached HTTP runtime task remains.
5. HTTP terminal panic/cancellation surfaced by EggServe:
   - classify as a failure;
   - do not convert it to successful shutdown.

Prefer a few concrete helper functions for result classification over a
generic service-supervisor trait.

The current orchestration tests are valuable. Adapt them instead of deleting
them.

## Part F — add wire-level HTTP contract tests

The current integration helpers primarily exercise gRPC. Add a small HTTP
fixture that starts the actual Router through EggServe on
`127.0.0.1:0`.

Cover at minimum:

1. `GET /health` healthy:
   - 200;
   - JSON contains current version and `"healthy"`.
2. unhealthy database path/fixture if deterministic support already exists:
   - 503;
   - `"unhealthy"`.
3. `GET /metrics` with credentials absent:
   - 404.
4. metrics with credentials configured:
   - missing/wrong Basic auth -> 401;
   - correct Basic auth -> 200 and Prometheus text.
5. security headers on success and error responses:
   - `x-content-type-options: nosniff`;
   - `x-frame-options: DENY`;
   - `cache-control: no-store`.
6. configured CORS origin behavior.
7. loopback-only `CORS_ALLOW_ALL` behavior.
8. HEAD behavior for GET routes.
9. unknown route remains 404.
10. body-bearing health/metrics request is rejected according to the selected
    explicit body policy.
11. one keep-alive connection remains usable beyond a short comparison
    lifetime, proving total-lifetime opt-out.
12. HTTP shutdown drains without a detached task.

Use raw TCP or a tiny existing test client where wire behavior matters. Do not
add reqwest or another large HTTP client only for tests.

Do not duplicate EggServe's own parser/fuzz/security suite. Test the
snip-sync-to-EggServe boundary.

## Part G — controlled A/B release measurement

After the compatibility implementation passes focused tests:

~~~sh
cargo build --release -p snip-sync --bin snip-sync
wc -c target/release/snip-sync
cargo tree -p snip-sync
cargo tree -p snip-sync -e features
~~~

Record:

~~~text
A current Axum-runtime bytes:
B EggServe+Axum bytes:
delta bytes:
delta percent:
new direct dependencies:
new transitive packages:
EggServe features selected:
~~~

Decision gate:

- growth <= 10%: compatibility implementation may be retained if behavior is
  equivalent and the orchestration is no more complex than the current path;
- growth > 10%: revert the Plan 018 production implementation completely,
  retain only tests/documentation that remain useful without EggServe if
  appropriate, and record the result;
- any unexplained feature activation or TLS/H2/H3 activation: stop and correct
  before measuring;
- a passing size result does not justify keeping unnecessary wrapper code.

The 10% threshold matches the server-side dependency discipline established by
Plans 014/017.

Plan 019 remains useful after either result because the direct leaf graph is
materially smaller than `eggserve-core[tower]`.

## Part H — repository verification

Run the focused tests first, then the ordinary project gates:

~~~sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
bash scripts/check.sh
cargo build --release -p snip-sync --bin snip-sync
~~~

Also run the adapted orchestration tests explicitly if the ordinary commands
do not make their execution obvious in logs.

On the pushed implementation commit, require the existing GitHub Actions
Linux correctness and macOS/Windows platform-smoke lanes to pass.

Do not add a new workflow solely for EggServe.

## Part I — documentation and closure

If the compatibility path is retained:

- add a concise `CHANGELOG.md` Unreleased note;
- update `snip-sync/README.md` only where the internal HTTP runtime
  description is relevant; user-facing commands/ports/TLS instructions should
  not change;
- record exact resolved EggServe versions and size evidence in this plan;
- mark Plan 018 complete and update `plans/README.md`.

If the compatibility path is reverted:

- leave user documentation unchanged;
- record the measured rejection as a completed experiment, not unfinished
  work;
- mark Plan 018 complete;
- update the plan registry so Plan 019 can proceed from the original Axum
  baseline.

## Recommended execution order for a smaller implementation model

1. Read this plan, Plan 003, Plan 017 completion notes, `snip-sync/Cargo.toml`,
   `snip-sync/src/main.rs`, and `snip-sync/src/orchestration.rs`.
2. Record fresh A baseline bytes and dependency/features tree.
3. Add only `eggserve-core =0.2.2[tower]` and
   `eggserve-server =0.2.1`.
4. Refresh the lockfile narrowly and inspect selected EggServe features.
5. Compile before changing server logic.
6. Replace only `axum::serve` runtime ownership.
7. Keep the Router and middleware unchanged.
8. Adapt the existing lifecycle orchestrator to typed EggServe
   control/completion.
9. Add focused wire-level HTTP boundary tests.
10. Run focused tests.
11. Build and measure B.
12. Apply the <=10% gate mechanically.
13. Revert the production migration if the gate fails.
14. Run full project checks on the chosen final tree.
15. Push implementation, confirm CI, and fill completion evidence.

## Acceptance criteria

Plan 018 is complete when all applicable statements are true:

1. A fresh current `snip-sync` release-size baseline is recorded.
2. The trial uses published registry crates only.
3. Direct compatibility pins are `eggserve-core =0.2.2` with only
   `tower`, and `eggserve-server =0.2.1`.
4. No EggServe TLS, HTTP/2, or HTTP/3 feature is enabled.
5. Tonic remains the gRPC runtime.
6. The gRPC and HTTP listeners are both bound before either service starts.
7. EggServe adopts the already-bound HTTP listener; it does not rebind.
8. The existing Axum Router, handlers, CORS, auth, metrics, and security-header
   policy remain application-owned.
9. Total connection lifetime is explicitly disabled for the daemon HTTP
   runtime.
10. The request-body policy is explicit and covered by a consumer test.
11. Ctrl-C/SIGTERM still requests shutdown of both services.
12. Unexpected gRPC completion shuts down HTTP and returns failure.
13. Unexpected EggServe completion shuts down gRPC and returns failure.
14. EggServe terminal errors are not hidden behind a generic successful join.
15. Requested shutdown leaves no detached HTTP runtime task.
16. `/health`, `/metrics`, security headers, CORS, HEAD, 404, and
   keep-alive contracts pass through the real EggServe socket.
17. A controlled A/B release-size comparison is recorded.
18. Growth >10% causes a full production revert.
19. Growth <=10% is retained only when the code remains simpler or at least no
   more complex than the current runtime ownership.
20. No new HTTP abstraction, workspace crate, daemon manager, or release
   workflow is introduced.
21. Ordinary Linux checks and macOS/Windows platform smoke remain green.
22. Completion notes state KEEP or REVERT and why.

## Completion notes

Fill during implementation:

~~~text
Planning baseline HEAD: 31b7d83e7f1584110438e31aa01ef23181229acd
Implementation commit:
Toolchain/target/linker:
A current Axum-runtime bytes:
B EggServe+Axum bytes:
delta bytes / percent:
Resolved eggserve-core:
Resolved eggserve-server:
Resolved eggserve-primitives:
Resolved eggserve-static:
Selected EggServe features:
Focused HTTP tests:
Orchestration tests:
scripts/check.sh:
GitHub Actions:
Decision: KEEP / REVERT
Reason:
~~~
