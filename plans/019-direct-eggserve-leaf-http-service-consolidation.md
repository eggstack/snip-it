# Plan 019: direct EggServe leaf HTTP service consolidation

Status: blocked on Plan 018

Depends on: Plan 018

## Objective

After Plan 018 has produced a measured EggServe compatibility result, test the
smaller direct EggServe leaf-runtime architecture for snip-sync.

This plan answers a separate question from Plan 018: if EggServe owns the HTTP
runtime, is it cleaner and lighter to remove the Axum/Tower compatibility layer
entirely for a server that exposes only health and metrics endpoints?

The target experiment is:

~~~text
Tonic gRPC listener
    |
    +-- unchanged

HTTP listener
    |
    v
eggserve-server 0.2.1
    |
    v
small snip-sync native HTTP Service
    |
    +-- /health
    +-- /metrics
~~~

The direct path must use the published leaf crates:

~~~toml
eggserve-server = { version = "=0.2.1", default-features = false }
eggserve-primitives = { version = "=0.2.0", default-features = false }
~~~

It must not depend on eggserve-core.

This is a consolidation experiment, not a mandate to remove Axum. Keep the
native path only when it preserves the established HTTP contract and produces
a meaningful dependency, footprint, or maintenance improvement. Otherwise
retain the winner selected by Plan 018.

## Preconditions and decision inputs from Plan 018

Do not start this plan until Plan 018 records:

- fresh original Axum-runtime baseline bytes (A);
- EggServe plus Axum compatibility bytes (B);
- KEEP or REVERT decision for B;
- exact wire behavior for the health/metrics surface;
- final lifecycle/orchestration shape after typed EggServe supervision.

Possible starting states:

### Plan 018 kept B

Compare:

~~~text
A = original axum::serve implementation
B = EggServe + TowerToEggserve + Axum
C = EggServe direct leaf service
~~~

C may replace B only when it is a better lightweight endpoint.

### Plan 018 reverted B

Compare C directly with A. Do not reintroduce the core/Tower adapter merely to
implement this plan.

## Research findings

### Why a direct path is plausible

snip-sync currently uses Axum for a very small application surface:

- GET /health;
- GET /metrics;
- Basic authentication on metrics;
- CORS;
- three security response headers.

There are no application uploads, REST resources, nested routers, WebSockets,
SSE endpoints, multipart handlers, or JSON request extractors on the HTTP
listener.

EggServe's direct Service API already provides the primitives needed for this
surface:

- canonical request method;
- canonical path/query target;
- ordered request headers;
- response builder;
- byte or empty response bodies;
- explicit request-body policy;
- runtime response normalization;
- HEAD body suppression/framing;
- typed runtime supervision.

A custom router framework would be overengineering. A single concrete
snip-sync HTTP service/module is sufficient.

### Published leaf versions

Do not infer leaf versions from the EggServe workspace's synchronized 0.2.2
source version.

The registry-qualified direct graph is:

- eggserve-server 0.2.1;
- eggserve-primitives 0.2.0.

EggServe Plan 272 proved this pair from a fresh crates.io consumer with a
pre-bound listener, total-lifetime opt-out, keep-alive, split
control/completion, external shutdown, and clean terminal observation.

### Why the graph may be materially smaller

The Plan 018 compatibility path requires eggserve-core with the tower feature.
Core unconditionally composes eggserve-static and enables the HTTP/Tower
adapter edge.

The direct path should make these unnecessary for snip-sync production code:

- eggserve-core;
- eggserve-static;
- the EggServe Tower adapter;
- Axum, if no other snip-sync source uses it;
- tower-http, if CORS is implemented by the tiny native service;
- the direct tower dependency, if no remaining source uses it.

Some transitive packages may remain through Tonic or other workspace members.
Measure the actual snip-sync package graph rather than claiming that every
Tower/Hyper package disappears.

## Scope

### In scope

1. Capture the final Plan 018 dependency and release-size baseline.
2. Add/use the direct published EggServe leaf crates.
3. Move health/metrics HTTP application logic out of main.rs into one small
   concrete module if that improves clarity.
4. Implement exact route/method behavior without a generic router.
5. Preserve metrics Basic auth and constant-time credential comparison.
6. Preserve CORS behavior actually observed from the current server.
7. Preserve the three security headers.
8. Preserve health JSON and metrics output.
9. Preserve typed EggServe lifecycle supervision from Plan 018.
10. Remove compatibility/framework dependencies only when no longer used.
11. Measure release size and dependency graph.
12. Keep or revert C according to the gates below.

### Explicit non-goals

Do not:

- change the gRPC service;
- move gRPC onto EggServe;
- add a new router crate;
- create a generic middleware framework;
- create generic request/response abstractions around EggServe;
- enable TLS/H2/H3;
- add new HTTP endpoints;
- expose database or mutation APIs over HTTP;
- change Prometheus metric definitions;
- weaken Basic auth comparison;
- broaden CORS;
- add a new workspace crate;
- add a new CI workflow;
- redesign daemon/process control;
- alter the updater.

The direct HTTP code should remain obviously specific to snip-sync.

## Part A — capture the comparison baseline

Record the final Plan 018 state:

~~~sh
git rev-parse HEAD
rustc --version --verbose
cargo --version
cargo build --release -p snip-sync --bin snip-sync
wc -c target/release/snip-sync
cargo tree -p snip-sync
cargo tree -p snip-sync -e features
~~~

Copy Plan 018's A and B measurements into this plan's completion record.

If Plan 018 reverted B, the current tree should match A.

## Part B — preserve the wire contract before deleting Axum

Before removing the existing Router, ensure tests capture the behavior the
native service must preserve.

At minimum establish expected behavior for:

### Routing and methods

- GET /health;
- HEAD /health;
- GET /metrics;
- HEAD /metrics;
- query strings do not change route identity, for example /health?probe=1;
- unknown path -> 404;
- unsupported method on a known GET route -> the currently observed status
  and Allow header behavior;
- CORS preflight OPTIONS -> current observed status/headers.

Do not guess Axum/Tower-HTTP behavior. Capture it from the current Plan 018
implementation or the original baseline and make that test the migration
contract.

### Health

Healthy response preserves:

- HTTP 200;
- JSON content type;
- current package version;
- status = "healthy".

Database ping failure preserves:

- HTTP 503;
- JSON response;
- status = "unhealthy".

Do not expose database error details.

### Metrics

Preserve all existing states:

- neither credential configured -> 404;
- only one credential configured -> endpoint disabled/404;
- both credentials configured + no auth -> 401;
- malformed Basic encoding -> 401;
- wrong credentials -> 401;
- correct credentials -> 200 and Prometheus text;
- comparison remains constant-time with length equality.

Do not move metrics authentication into a generic reusable auth layer.

### Common response policy

Preserve:

~~~text
x-content-type-options: nosniff
x-frame-options: DENY
cache-control: no-store
~~~

for the same endpoint/error classes that receive them today.

### CORS

The current behavior is application policy, not an EggServe default.

Preserve:

- CORS_ALLOW_ALL=true only when the HTTP bind is loopback;
- non-loopback ignores/refuses permissive allow-all exactly as the current
  startup path does;
- configured origins remain an allow-list;
- configured mode allows GET and the existing Content-Type/Authorization
  headers;
- preflight output matches the captured baseline;
- absent CORS configuration does not become permissive.

Do not implement a general CORS library. The service only needs the policy
already exposed by snip-sync configuration.

## Part C — implement one concrete native HTTP service

Preferred organization:

~~~text
snip-sync/src/http.rs
~~~

with one small concrete service/state wrapper.

Keep main.rs responsible for:

- database/bootstrap startup;
- listener binding;
- starting Tonic;
- starting EggServe;
- process signal selection;
- top-level orchestration.

Keep the HTTP module responsible for:

- route/method dispatch;
- health response;
- metrics response/auth;
- CORS response handling;
- common security headers.

Do not create multiple layers, traits, or modules merely to imitate Axum.

The service may implement eggserve_server::Service directly when that is
clearer than a deeply captured service_fn closure.

Use canonical request data through the public request/head accessors for:

~~~text
method
target path
headers
~~~

Use canonical response values:

~~~text
eggserve_primitives::Response
eggserve_primitives::ResponseBody
eggserve_primitives::StatusCode
~~~

Let EggServe own final HTTP framing and normalization.

Do not write directly to sockets.

## Part D — request-body and HEAD policy

This control surface does not consume request bodies.

Use the native service default or explicit:

~~~text
RequestBodyPolicy::Reject
~~~

and keep the Plan 018 consumer test that proves the resulting wire behavior.

For HEAD, produce the same logical representation as GET and rely on
EggServe's canonical HEAD normalization to suppress wire body bytes while
retaining appropriate representation length. Do not maintain a second HEAD
body-stripping implementation unless a regression proves it is required.

## Part E — preserve lifecycle work from Plan 018

Do not redesign orchestration again.

The direct server exposes the same ServerControl and ServerCompletion types
qualified in Plan 018. Reuse the final Plan 018 supervisor unchanged except
for imports/dependency ownership needed after removing core.

The same invariants remain mandatory:

- both listeners pre-bound before service startup;
- total connection lifetime disabled;
- Ctrl-C/SIGTERM shuts down both services;
- unexpected gRPC completion shuts down HTTP;
- unexpected HTTP completion shuts down gRPC;
- typed EggServe terminal error remains visible;
- requested shutdown drains cleanly;
- no detached HTTP task remains.

## Part F — dependency deletion

After the native service passes focused tests, inspect usage before deleting
dependencies.

Expected candidates from snip-sync/Cargo.toml:

~~~text
eggserve-core        remove
axum                 remove if no remaining source use
tower-http           remove if no remaining source use
tower                remove as a direct dependency if no remaining source use
~~~

Keep:

~~~text
eggserve-server = =0.2.1
eggserve-primitives = =0.2.0
~~~

Do not remove unrelated dependencies opportunistically in this plan.

Use compilation and source search to prove a dependency is dead before
removing it. Do not infer removal merely because a transitive copy still
appears in cargo tree.

## Part G — controlled C measurement

Build with the same environment used by Plan 018:

~~~sh
cargo build --release -p snip-sync --bin snip-sync
wc -c target/release/snip-sync
cargo tree -p snip-sync
cargo tree -p snip-sync -e features
~~~

Record:

~~~text
A original Axum-runtime bytes:
B EggServe+Axum bytes:
C direct EggServe bytes:
C vs A bytes / percent:
C vs B bytes / percent:
direct dependencies removed:
transitive packages removed:
remaining Tower/Hyper sources:
native HTTP module production LOC:
~~~

Do not use production LOC as an absolute quality score. Record it only to make
obvious if a supposedly tiny two-route replacement turned into a framework.

## Part H — keep/revert gate

C must first satisfy the hard server footprint gate:

~~~text
C <= A + 10%
~~~

If C exceeds the original current-server baseline by more than 10%, revert C.

When Plan 018 kept B, prefer C over B only when at least one of these is true:

1. C is materially smaller, approximately at least 128 KiB or 2%; or
2. C removes the core/static/Axum/Tower-HTTP direct dependency surface while
   remaining essentially size-neutral, no more than 128 KiB and 2% larger
   than B, and the native HTTP code remains a single straightforward
   application-specific module.

When Plan 018 reverted B, C may still be kept when:

- C passes the <=10% vs A hard gate;
- the direct dependency graph is acceptably small;
- the native implementation is simpler to own than the compatibility graph;
- all HTTP/lifecycle contracts pass.

Revert C if it requires recreating a framework, substantial generic middleware
machinery, or brittle manual HTTP semantics merely to remove Axum.

Final winner must be one of:

~~~text
A: original Axum runtime
B: EggServe runtime + Axum application
C: direct EggServe runtime + native two-endpoint service
~~~

Do not keep parallel production implementations or a runtime feature flag.

## Part I — focused verification

Run HTTP contract tests first. They must exercise the real socket.

Then run:

~~~sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
bash scripts/check.sh
cargo build --release -p snip-sync --bin snip-sync
~~~

Confirm existing GitHub Actions Linux correctness and macOS/Windows
platform-smoke jobs on the final pushed implementation.

If direct dependency removal changes Windows compilation, fix only the
consumer seam. Do not add platform-specific HTTP architectures.

## Part J — documentation and closure

If C wins:

- add/update the concise CHANGELOG.md Unreleased note;
- keep user-facing snip-sync/README.md behavior unchanged unless an
  implementation detail is explicitly documented there;
- record exact A/B/C measurements and dependency deletions;
- mark Plans 018 and 019 state consistently in plans/README.md.

If C loses:

- fully revert the native production experiment;
- retain B if Plan 018 kept B, otherwise return to A;
- record the rejected C measurement as completed evidence;
- do not leave dead native modules/dependencies.

## Recommended execution order for a smaller implementation model

1. Read Plan 018 completion notes and this plan.
2. Record the final Plan 018 release-size/dependency baseline.
3. Ensure wire tests cover method routing, CORS preflight, health, metrics,
   common headers, and HEAD.
4. Add direct eggserve-primitives =0.2.0 if not already direct.
5. Create one concrete snip-sync/src/http.rs service.
6. Route only health, metrics, preflight, and fallback behavior.
7. Preserve Basic auth exactly.
8. Preserve CORS exactly from the captured tests.
9. Run focused HTTP tests before deleting Axum.
10. Remove core/Axum/Tower-HTTP/direct-Tower only where now unused.
11. Compile and inspect the dependency tree.
12. Run full focused and repository tests.
13. Build C with the same release environment.
14. Apply the hard <=10% vs A gate.
15. Compare C against B using the meaningful-improvement criteria.
16. Keep exactly one production architecture.
17. Push, confirm CI, and fill completion evidence.

## Acceptance criteria

Plan 019 is complete when all applicable statements are true:

1. Plan 018 is complete before implementation begins.
2. A/B baseline evidence is copied forward.
3. Direct runtime pins use published eggserve-server =0.2.1 and
   eggserve-primitives =0.2.0.
4. Production C has no eggserve-core dependency.
5. No EggServe TLS/H2/H3 feature is enabled.
6. Tonic/gRPC remains unchanged.
7. HTTP application logic is one small concrete snip-sync module, not a
   generic framework.
8. Health status/body/content type remains compatible.
9. Metrics disabled/authenticated behavior remains compatible.
10. Basic credential comparison remains constant-time.
11. CORS allow-list/allow-all/preflight behavior remains compatible.
12. Security headers remain compatible.
13. GET/HEAD/unknown/unsupported-method behavior is covered by wire tests.
14. Non-empty request bodies follow the explicit Reject policy.
15. EggServe owns final HTTP framing and HEAD body suppression.
16. Plan 018 lifecycle/supervision behavior remains intact.
17. Axum/Tower-HTTP/core dependencies are removed only after proving they are
    unused.
18. A/B/C release-size and dependency-tree evidence is recorded.
19. C exceeding A by more than 10% is reverted.
20. C replaces B only when it gives a meaningful size/dependency/maintenance
    improvement under the stated gate.
21. Exactly one HTTP production architecture remains.
22. Ordinary repository checks and hosted CI remain green.
23. Completion notes state the final A/B/C winner and why.

## Completion notes

Fill during implementation:

~~~text
Plan 018 decision:
A original Axum-runtime bytes:
B EggServe+Axum bytes:
C direct EggServe bytes:
C vs A bytes / percent:
C vs B bytes / percent:
Resolved eggserve-server:
Resolved eggserve-primitives:
Removed direct dependencies:
Removed transitive packages:
Remaining relevant transitive framework packages:
Native HTTP production LOC:
Wire HTTP tests:
Orchestration tests:
scripts/check.sh:
GitHub Actions:
Final winner: A / B / C
Reason:
~~~
