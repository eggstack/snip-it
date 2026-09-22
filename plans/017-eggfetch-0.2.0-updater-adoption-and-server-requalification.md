# Plan 017: eggfetch-core 0.2.0 updater adoption and server requalification

Status: complete

Depends on: Plan 016 (complete)

## Objective

Adopt the newly published `eggfetch-core 0.2.0` release for the existing
`snp update` transport without redesigning updater behavior, broadening the
selected HTTP feature surface, or reopening already-closed redirect/timeout
policy work.

The required end state is:

```text
snp updater
    |
    +-- snip-it continues to own:
    |    +-- initial production HTTPS-only gate
    |    +-- exact release/tag/asset mapping
    |    +-- 404-only Cargo fallback classification
    |    +-- checksum/candidate/replacement policy
    |    +-- 1 MiB metadata and 256 MiB binary bounds
    |    `-- partial-file cleanup on failed streamed downloads
    |
    `-- eggfetch-core 0.2.0 continues to own:
         +-- lean standard HTTP/1.1 transport
         +-- Rustls + native roots
         +-- strict bounded redirect following
         +-- connect/read timeouts
         `-- one absolute total deadline through final response-body EOF
```

This is primarily a dependency qualification and version-resynchronization
pass. The existing `snp` adapter should not require source changes merely to
compile against 0.2.0.

As a secondary measurement-only task, re-evaluate whether the deliberately
retained `snip-sync` external-`curl` updater is still the correct footprint
tradeoff when compared against the current lean 0.2.0 profile. Do not migrate
`snip-sync` unless a fresh controlled measurement satisfies the existing
lightweight-tool size gate and all updater behavior remains equivalent.

## Research findings

### Current snip-it state

The root package currently pins:

```toml
eggfetch-core = { version = "=0.1.7", default-features = false, features = [
    "standard-http1",
    "redirects",
    "tls-rustls",
    "tls-native-roots",
] }
futures-util = { version = "0.3", default-features = false, features = ["alloc"] }
```

That is already the preferred narrow profile for this consumer.

`src/update.rs` currently relies on public 0.1.7 APIs that remain present in
0.2.0:

- `Client::builder()`;
- `ClientBuilder::user_agent()`;
- `ClientBuilder::automatic_decompression(false)`;
- `RedirectPolicy::strict(MAX_REDIRECTS)`;
- `ClientBuilder::redirect_policy()`;
- `Timeout::builder().connect/read/total()`;
- `RequestBuilder::max_decoded_body_size()`;
- `RequestBuilder::send()`;
- `Response::bytes()` / `Response::bytes_stream()`;
- `Error::DecodedBodyTooLarge`;
- `Error::Timeout { phase: TimeoutPhase::Total, .. }`.

The application-level initial URL policy, 404 classification, release asset
selection, checksum validation, candidate validation, replacement logic, and
fallback behavior are all snip-it policy and must remain unchanged.

### eggfetch-core 0.2.0 release qualification

The coordinated eggfetch release is `v0.2.0`, prepared by release commit
`8959ca890ee34f4cf456aed648315322f1e83ef7` and published on 2026-09-22.

The release explicitly states that there are no intentional breaking changes
relative to 0.1.7 across the public Rust/Python/C/CLI/HTTPX surfaces, feature
graph/defaults, MSRV, or dependency policy.

The release principally resynchronizes versions across crates.io, PyPI, and
GitHub while including fixes prepared after crates.io 0.1.7:

1. streaming decompression chunk-boundary corruption (issue #24);
2. Windows PyPI wheel `RUSTFLAGS=-D warnings` cleanup;
3. API-preserving internal decomposition/maintenance work.

For the `snp` updater, issue #24 is not a reason to enable compression.
The updater intentionally calls `automatic_decompression(false)` and does not
select any `compression-*` feature so release binaries remain byte-for-byte
identical to published assets.

A direct manifest comparison of `eggfetch-core` 0.1.7 and 0.2.0 found only
these relevant manifest deltas:

- package version `0.1.7 -> 0.2.0`;
- the disabled `compression-deflate` feature additionally selects zlib support;
- optional `eggfetch-http-connect` version `0.1.7 -> 0.2.0`.

Neither non-version delta participates in snip-it's selected
`standard-http1 + redirects + tls-rustls + tls-native-roots` profile.

### Prior size evidence

Plan 016's controlled release builds recorded:

```text
0.1.5 full-http1 baseline:     6,973,056 bytes
0.1.7 full-http1 control:      6,973,056 bytes
0.1.7 lean+redirects final:    6,776,320 bytes
lean delta vs full:             -196,736 bytes / -2.82%
```

Those are historical reference values only. Plan 017 must take a fresh
same-host/same-toolchain/same-profile baseline before changing the dependency.

Plan 014's original `snip-sync` trial used the older 0.1.5 full `http1`
profile and recorded:

```text
curl baseline:                 3,833,152 bytes
eggfetch 0.1.5 trial:          5,407,400 bytes
delta:                        +1,574,248 bytes / +41.07%
decision: revert to curl
```

That result remains authoritative for the implementation that was actually
tested, but it predates the lean `standard-http1` profile. A current
measurement is therefore useful before permanently treating the 0.1.5 result
as representative of 0.2.0.

The measurement is not permission to migrate. `snip-sync` remains on curl
unless the fresh lean trial satisfies the gate below.

## Scope and constraints

### Required work

1. Bump only the root `snp` dependency from exact `=0.1.7` to exact
   `=0.2.0`.
2. Refresh `Cargo.lock` narrowly.
3. Preserve the existing eggfetch feature list exactly unless compilation
   proves a feature is no longer valid.
4. Prove the current updater behavior still works against 0.2.0.
5. Take controlled release-binary size and feature-tree measurements.
6. Update the changelog/plan state with the actual results.
7. Optionally perform the explicitly bounded `snip-sync` measurement trial
   described below, then either keep curl or migrate only if the gate passes.

### Explicit non-goals

Do not use this plan to:

- redesign `snp update`;
- change release/tag/asset naming;
- replace Tonic gRPC sync traffic;
- replace Axum/Tonic server networking;
- add a generic HTTP abstraction;
- add a shared updater crate;
- enable HTTP/2 or HTTP/3;
- enable compression;
- enable JSON helpers;
- enable proxy/environment-proxy behavior;
- enable logical retries;
- enable Basic auth;
- enable cookies, multipart, tracing, or test utilities;
- change TLS trust semantics;
- add async filesystem I/O;
- alter Cargo/Homebrew fallback behavior;
- change installer or release-workflow architecture;
- broaden the project's runtime model;
- make `snip-sync` migration mandatory.

Keep this a lightweight consumer adoption pass.

## Part A — freeze the baseline

Before editing manifests, record:

```sh
git rev-parse HEAD
rustc --version --verbose
cargo --version
cargo build --release -p snip-it --bin snp
```

Record the exact `snp` release-binary byte count using the host-appropriate
tool, for example:

```sh
wc -c target/release/snp
```

Also capture:

```sh
cargo tree -p snip-it -e features
cargo tree -p snip-it -e features -i eggfetch-core
```

The baseline must be built from the current checked-in 0.1.7 lockfile using
the same toolchain, target, linker, release profile, and environment that will
be used for the 0.2.0 comparison.

Do not substitute Plan 016's historical size for this fresh baseline.

## Part B — adopt eggfetch-core 0.2.0 in snp

Change the root manifest to:

```toml
eggfetch-core = { version = "=0.2.0", default-features = false, features = [
    "standard-http1",
    "redirects",
    "tls-rustls",
    "tls-native-roots",
] }
```

Keep the existing `futures-util` dependency because
`stream_binary_to_file` still consumes `bytes_stream()` via
`StreamExt::next()`.

Refresh the lockfile narrowly:

```sh
cargo update -p eggfetch-core --precise 0.2.0
```

If Cargo requires the coordinated optional helper package to move as part of
normal resolution, allow that only where required by the selected graph.
Do not run an unconstrained whole-workspace dependency update as part of this
plan.

After the lockfile refresh, inspect the `eggfetch-core` package entry and
feature tree. The expected selected capability set remains:

```text
standard-http1
transport-http1
standard-route
high-level-url
redirects
tls-rustls
tls-native-roots
```

The following eggfetch capabilities must remain absent from this consumer
selection unless they appear transitively for an unrelated pre-existing
workspace dependency rather than through eggfetch:

```text
advanced-routing
logical-retry
basic-auth
proxy
http2
http3
json
compression-gzip
compression-brotli
compression-deflate
compression-zstd
cookies
multipart
tracing
test-util
```

Do not switch back to the broad `http1` alias.

## Part C — preserve the existing updater adapter

First attempt to compile and test with no changes to `src/update.rs`.

That is the preferred outcome.

Do not proactively rewrite API calls merely because the dependency crossed
from 0.1.x to 0.2.x. The release contract says the relevant public surface is
preserved.

The following behavior must remain exactly as implemented after Plan 016:

### C1. Initial production URL gate

Production initial URLs remain HTTPS-only before network I/O.

The existing `check_url_scheme` / `scheme_allowed` application policy stays
in place. Eggfetch's strict redirect policy governs redirect hops; it is not a
replacement for the initial URL allow-list.

### C2. Strict redirect handling

Continue using:

```rust
eggfetch_core::RedirectPolicy::strict(MAX_REDIRECTS)
```

with the existing maximum redirect count.

Do not reintroduce the deleted local redirect state machine.

### C3. Native total deadline

Continue using request-level:

```rust
eggfetch_core::Timeout::builder()
    .total(FETCH_OVERALL_TIMEOUT)
    .build()
```

while retaining the client-level connect/read phase limits.

Do not reintroduce an outer `tokio::time::timeout` unless a demonstrated
0.2.0 regression invalidates eggfetch's native total-deadline contract. If
that occurs, stop and document it as an upstream regression rather than
silently layering policy back into snip-it.

### C4. Body limits and streaming

Preserve:

- 1 MiB metadata bound;
- 256 MiB release-binary bound;
- streamed binary writes through `Response::bytes_stream()`;
- cleanup of partial candidate files on stream/timeout/write failure.

Do not buffer the whole release binary merely to simplify the bump.

### C5. Automatic decompression stays off

Keep:

```rust
.automatic_decompression(false)
```

Do not enable any compression feature in response to the issue #24 fix.
Published binary bytes and checksums must remain canonical.

### C6. Error/fallback classification

Preserve:

- HTTP 404 on the release binary as the only HTTP condition eligible for the
  existing Cargo asset fallback;
- timeout, DNS, TLS, connection, body-limit, stream, 401/403, 5xx, checksum,
  malformed metadata, candidate identity, and candidate version failures as
  hard failures.

Do not collapse typed eggfetch failures into fallback behavior.

## Part D — focused consumer qualification

Run the existing updater-focused tests first:

```sh
cargo test -p snip-it --features test-support --bin snp update:: -- --test-threads=1
```

The existing suite should continue to cover, at minimum:

- production HTTP initial URL rejected before network I/O;
- test-only loopback HTTP fixture acceptance;
- metadata fetch;
- binary streaming;
- metadata/body limits;
- 404-only fallback distinction;
- 401/403/5xx hard failures;
- strict redirect policy selection;
- relative/absolute redirect traversal;
- redirect-depth failure;
- slow-drip metadata total deadline;
- slow-drip binary total deadline;
- cleanup of timed-out/failed partial candidate files.

Do not add duplicate tests for eggfetch-internal invariants already covered by
eggfetch. Add a snip-it test only if the dependency bump exposes a consumer
contract not already represented.

If all focused tests pass without source changes, record that explicitly.

## Part E — controlled snp size/feature comparison

Using the exact environment captured in Part A:

```sh
cargo build --release -p snip-it --bin snp
wc -c target/release/snp
cargo tree -p snip-it -e features
cargo tree -p snip-it -e features -i eggfetch-core
```

Record:

- 0.1.7 baseline bytes;
- 0.2.0 bytes;
- absolute byte delta;
- percentage delta;
- any selected-feature differences.

Interpretation:

- byte-identical or smaller: accept;
- growth <= 128 KiB and <= 2%: accept after verifying no accidental features;
- growth above either threshold: inspect `cargo tree -e features` and the
  lockfile before proceeding;
- unexplained material growth caused by the dependency itself: document it
  and stop before closing the plan rather than normalizing it away.

These are investigation thresholds, not an invitation to remove required
security/behavior features.

## Part F — optional snip-sync lean-profile requalification

This part is a bounded experiment and may end with no production code change.

### F1. Fresh curl baseline

Build the current server exactly as shipped:

```sh
cargo build --release -p snip-sync --bin snip-sync
wc -c target/release/snip-sync
```

Record the same environment information used for the trial.

### F2. Temporary 0.2.0 lean trial

On the implementation branch/worktree, temporarily port only the updater HTTP
adapter to the same narrow profile used by `snp`:

```toml
eggfetch-core = { version = "=0.2.0", default-features = false, features = [
    "standard-http1",
    "redirects",
    "tls-rustls",
    "tls-native-roots",
] }
futures-util = { version = "0.3", default-features = false, features = ["alloc"] }
```

Reuse the already-qualified `snp` updater pattern rather than designing a
second transport abstraction:

- one in-process eggfetch client;
- strict redirects;
- production HTTPS-only initial gate;
- native total deadline;
- metadata/body bounds;
- streamed binary candidate;
- identical 404/error classification.

Do not change the rest of `snip-sync` to async-main architecture. The update
command may use the same narrow one-shot runtime/block-on pattern already
described by Plan 014 if required.

### F3. Decision gate

Measure the release binary after the temporary trial.

The production migration is allowed only if all of the following are true:

1. release binary growth is <=10% relative to the fresh current curl baseline;
2. there is no accidental broad eggfetch feature activation;
3. updater-focused behavior tests pass;
4. the migration deletes the external curl transport path cleanly;
5. no new shared crate/framework/runtime architecture is introduced.

If the trial exceeds 10%, revert the trial completely and retain curl.

A result above the threshold is not unfinished work. Record it as renewed
evidence that the current split remains intentional.

If the trial is <=10%, keep the migration only if the maintenance benefit is
real and the implementation remains small. Do not keep eggfetch merely to make
both binaries look symmetrical.

### F4. Required record even when reverted

Record:

```text
snip-sync curl baseline bytes:
snip-sync eggfetch 0.2.0 lean trial bytes:
delta bytes:
delta percent:
selected eggfetch features:
decision: KEEP / REVERT
reason:
```

If reverted, confirm a rebuild returns to the original baseline within normal
deterministic-build expectations and leave the existing module comment updated
to reference the 0.2.0 requalification result.

## Part G — full repository verification

After the final production decision is made, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
cargo build --release -p snip-it --bin snp
```

If `snip-sync` is actually migrated, also run its updater-specific tests and
a final release build explicitly.

Run the repository's ordinary GitHub Actions through the pushed implementation
commit. Require the existing Linux correctness, macOS platform smoke, and
Windows platform smoke coverage to remain green.

Do not add a new CI workflow solely for this version bump.

## Part H — documentation/state cleanup

Update only documentation that becomes stale because of the adoption.

At minimum:

1. add a concise `CHANGELOG.md` Unreleased entry stating that `snp update`
   now uses `eggfetch-core 0.2.0` on the unchanged lean profile;
2. if `snip-sync` remains on curl, update its intentional-split module comment
   with the new 0.2.0 measurement so future agents do not retry the old
   question using only Plan 014's 0.1.5 result;
3. if `snip-sync` migrates, remove statements that say it intentionally
   requires curl and note the measured accepted cost;
4. mark this plan complete and update `plans/README.md` in the same closure
   commit with final measurements and decisions.

Do not rewrite Plans 014-016 historical completion notes. They describe the
state and evidence at the time they were executed.

## Recommended execution order for a smaller implementation model

1. Read this plan, Plan 016 completion notes, root `Cargo.toml`, and
   `src/update.rs`.
2. Record current HEAD/toolchain and fresh `snp` 0.1.7 size/feature baseline.
3. Change only the root eggfetch version to exact `=0.2.0`.
4. Run the targeted Cargo lockfile update.
5. Inspect the resulting eggfetch feature tree.
6. Compile `snp` before changing any updater source.
7. Run the focused updater test suite.
8. If green, do not refactor `src/update.rs`.
9. Measure final `snp` release size and record the A/B result.
10. Run the optional `snip-sync` measurement trial in isolation.
11. Apply the <=10% gate mechanically; revert the server trial if it fails.
12. Run full workspace/repository verification on the final production tree.
13. Push implementation changes.
14. Confirm GitHub Actions.
15. Fill in this plan's completion notes, mark `Status: complete`, and update
    the plan index.

Do not interleave unrelated dependency upgrades or refactors with this work.

## Acceptance criteria

Plan 017 is complete when all applicable statements are true:

1. Root `snp` pins `eggfetch-core =0.2.0`.
2. The root feature profile remains
   `standard-http1,redirects,tls-rustls,tls-native-roots`.
3. No broad `http1` alias is reintroduced.
4. No advanced routing, logical retry, Basic auth, proxy, HTTP2/3, JSON,
   compression, cookies, multipart, tracing, or test-util capability is
   accidentally selected through the root eggfetch dependency.
5. The 0.2.0 lockfile update is narrow and unrelated packages are not
   gratuitously refreshed.
6. `src/update.rs` remains source-compatible without redesign, unless a
   concrete 0.2.0 compatibility defect requires a minimal correction.
7. Initial production URLs remain HTTPS-only before network I/O.
8. Redirects remain delegated to `RedirectPolicy::strict(MAX_REDIRECTS)`.
9. Native `Timeout.total` remains the sole logical request/body deadline.
10. No outer Tokio timeout state machine is reintroduced.
11. 1 MiB metadata and 256 MiB binary bounds remain intact.
12. Release binary bodies remain streamed and automatic decompression remains
    disabled.
13. HTTP 404-only Cargo fallback semantics remain unchanged.
14. Timeout/TLS/DNS/body-limit/stream/HTTP error failures remain hard failures.
15. Focused updater tests pass.
16. Full workspace tests/checks/platform CI pass.
17. A controlled current 0.1.7 -> 0.2.0 `snp` size comparison is recorded.
18. Any material size change is explained by the feature/dependency graph.
19. `snip-sync` is either:
    - unchanged on curl with a fresh 0.2.0 lean-profile trial documenting a
      >10% size cost; or
    - migrated only after a <=10% controlled size result and equivalent
      updater behavior proof.
20. No new HTTP abstraction/shared updater crate/runtime architecture is added.
21. Changelog/module comments/plan index describe the actual final state.
22. This plan contains final completion evidence and is marked complete.

## Completion notes

Fill this block during implementation; do not mark the plan complete with
placeholders remaining.

```text
Planning baseline HEAD: 5ddb57cea86bd1e9859f0e2a46cbb17aeaed3c69
Implementation commit: final Plan 017 closure commit on `main` (see `git log`)
eggfetch release/tag verified: `eggstack/eggfetch` `v0.2.0` peeled to
    8959ca890ee34f4cf456aed648315322f1e83ef7
eggfetch-core version: =0.2.0
final root features: standard-http1, redirects, tls-rustls, tls-native-roots
    (`default-features = false`)

snp baseline toolchain/target/linker: rustc 1.94.1 (aarch64-unknown-linux-gnu),
    Cargo 1.94.1, release profile (`opt-level=z`, LTO, one codegen unit,
    stripped, abort panic); default host linker
snp 0.1.7 baseline bytes: 6,776,320
snp 0.2.0 final bytes: 6,776,320
snp delta bytes / percent: 0 / 0.00%
snp feature-tree observations: selected standard-http1, transport-http1,
    standard-route, high-level-url, redirects, tls-rustls (+hyper-rustls),
    tls-native-roots; no forbidden eggfetch capabilities selected
src/update.rs behavioral source changes required: no (comment version aligned)
focused updater tests: pass, 21 passed (`cargo test -p snip-it --features
    test-support --bin snp update:: -- --test-threads=1`)
workspace tests: pass under the resource-safe serial workspace run; snip-it's
    1,164 library tests, 67 binary tests, architecture tests, and the
    snip-sync all-features suite passed. The repository's separate platform
    smoke invocation also passed through `scripts/check.sh`.
scripts/check.sh: pass (`=== All checks passed ===`; installer contract,
    format, clippy, unit suites, platform smoke, permissions, auto-sync
    closure/concurrency, and multi-batch sync all passed)
production seams: pass (`scripts/ci/test-production-seams.sh`; all five
    production-only test seams passed)
GitHub Actions: pending push and remote verification

snip-sync requalification performed: yes
snip-sync curl baseline bytes: 3,833,152
snip-sync eggfetch 0.2.0 trial bytes: 5,145,224
snip-sync delta bytes / percent: +1,312,072 / +34.23%
snip-sync trial feature tree: standard-http1, transport-http1,
    standard-route, high-level-url, redirects, tls-rustls, tls-native-roots
snip-sync decision: REVERT
snip-sync decision rationale: the fresh lean 0.2.0 trial exceeded the 10%
    server-size gate; production source and manifest remain on external curl.

docs/changelog updated: yes (`CHANGELOG.md`, `README.md`, `AGENTS.md`,
    `.skills/server-module.md`, architecture updater references, and plan index)
unexpected deviations: none; the temporary server trial compiled and was
    removed completely after measurement.
```
