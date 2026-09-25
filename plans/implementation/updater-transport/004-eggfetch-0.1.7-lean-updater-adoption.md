> **Planning-convention migration note.** This file was promoted via `git mv` from pre-convention flat plan `016-eggfetch-0.1.7-lean-updater-adoption.md` (history preserved). Its body is unchanged historical evidence.
> New-convention identity: Updater Transport M004 — eggfetch 0.1.7 lean updater adoption.
> Source roadmap: `plans/subsystems/updater-transport-roadmap.md` (M004).
> Long-term requirements: `plans/000-long-term-specification.md`, `plans/001-terminology-and-domain-model.md`, `plans/002-long-term-roadmap.md`.
> Primary class: infrastructure. Dependencies: hard M002, M003.
> Status vocabulary mapping: pre-convention `Status: complete` means `closed` with closure record at `plans/closure/updater-transport/004-status.md`.
> Pre-convention archive pointer: `plans/archive/flat-016-eggfetch-0.1.7-lean-updater-adoption.md`.

---

# Plan 016: eggfetch-core 0.1.7 lean updater adoption

Status: complete

Depends on: Plan 014 (complete), Plan 015 (complete)

## Objective

Upgrade the `snp` self-update transport from the currently pinned
`eggfetch-core =0.1.5` integration to `eggfetch-core =0.1.7`, and adopt the
new 0.1.7-era capabilities that directly reduce snip-it-owned transport policy
and linked footprint.

The intended end state is:

```text
snp updater policy owned by snip-it
    |
    +-- exact release/tag/asset mapping
    +-- initial production URL must be HTTPS
    +-- 404-only Cargo fallback classification
    +-- checksum/candidate/replacement policy
    +-- 1 MiB metadata / 256 MiB binary limits
    +-- staging-file cleanup on failed binary download
    |
    `-- HTTP mechanics delegated to eggfetch-core 0.1.7
         +-- lean standard HTTP/1.1 route
         +-- Rustls + native roots
         +-- strict bounded redirect following
         +-- connect/read timeouts
         `-- one absolute total deadline through response-body EOF
```

This is not a new updater redesign. It is a dependency qualification and
local-policy deletion pass made possible by eggfetch 0.1.7.

## Research findings

### Current snip-it state

At the start of this plan, root `Cargo.toml` pins:

```toml
eggfetch-core = { version = "=0.1.5", default-features = false, features = [
    "http1",
    "tls-rustls",
    "tls-native-roots",
] }
futures-util = { version = "0.3", default-features = false, features = ["alloc"] }
```

The current `http1` compatibility feature pulls more capability than the
updater needs.

Plan 014 intentionally used manual redirect traversal because eggfetch 0.1.5
could follow redirects but could not reject HTTPS -> HTTP downgrade before the
second request. Plan 015 then wrapped the whole logical fetch in
`tokio::time::timeout` because 0.1.5's native `Timeout.total` did not remain
authoritative through final response-body consumption.

Those local workarounds are correct for 0.1.5, but they should not survive a
0.1.7 adoption if the new native contracts qualify successfully.

Plan 014's controlled Linux release build recorded:

```text
snp before eggfetch: 6,248,720 bytes
snp with eggfetch 0.1.5/full http1 profile: 6,970,568 bytes
delta: +721,848 / +11.55%
```

Use a fresh same-toolchain baseline during implementation; these numbers are
historical context, not a substitute for a controlled current comparison.

### eggfetch 0.1.7 release qualification

The coordinated 0.1.7 release source is commit
`43c3b312f2def887d0f0b7ce539faa626adf2cc8`.

Relative to the 0.1.5 release preparation commit
`0f720b4fd9efea80d180ce5942fce5e3453d8003`, the 0.1.7 release contains the
following changes that matter to this consumer.

#### 1. Lean standard-route profiles

0.1.7 exposes transport/routing/policy boundaries instead of requiring the
full `http1` compatibility alias.

Relevant feature definitions:

```text
standard-http1 = transport-http1 + standard-route + high-level-url
http1          = native-http1 + high-level-url + logical-retry
                 + redirects + basic-auth
native-http1   = transport-http1 + standard-route + advanced-routing
```

For snip-it, the updater does not need:

- custom `Dialer`;
- direct/pinned resolved targets;
- SNI override routing;
- local-address/socket options;
- UDS;
- logical retries;
- Basic authentication.

It does need:

- high-level URL GET requests;
- ordinary DNS -> TCP/TLS routing;
- HTTP/1.1;
- redirects;
- Rustls;
- native trust roots.

The preferred profile is therefore:

```toml
eggfetch-core = { version = "=0.1.7", default-features = false, features = [
    "standard-http1",
    "redirects",
    "tls-rustls",
    "tls-native-roots",
] }
```

Do not retain `http1` merely for compatibility convenience. That would
re-enable `advanced-routing`, `logical-retry`, and `basic-auth`, defeating
the principal footprint improvement in this release.

Eggfetch's own controlled x86_64 embedded measurement showed
`standard-http1,tls-rustls` reducing a full compatibility client by roughly
558 KiB stripped / 15.2% in that isolated fixture. That measurement is
directional only for snip-it because `snp` already links overlapping
Tokio/Hyper/Rustls machinery through Tonic. Measure the actual binary.

#### 2. Strict redirect transport policy

The 0.1.6 changes included in the 0.1.7 upgrade add:

```rust
RedirectDowngradePolicy::{Allow, Deny}
RedirectPolicy::strict(max_redirects)
ClientBuilder::redirect_policy(...)
ClientBuilder::redirect_downgrade_policy(...)
```

`RedirectPolicy::strict(n)` follows redirects while rejecting an
HTTPS -> HTTP downgrade before second-hop I/O. Relative redirect targets are
resolved normally, and redirect URL validation independently rejects
unsupported schemes.

This is the capability Plan 014 lacked.

For production, snip-it should continue to reject an initial non-HTTPS URL
before any request. Once the initial URL passes that application-level check,
eggfetch's strict redirect policy can own hop resolution, redirect-depth
limits, and HTTPS downgrade rejection.

The test-only endpoint seam may continue allowing an initial HTTP fixture.
Strict downgrade policy only rejects HTTPS -> HTTP; an all-HTTP loopback
fixture remains usable.

#### 3. Native total deadline now covers response-body lifecycle

0.1.7 fixes `Timeout.total` to be one absolute deadline from logical request
start through final response-body EOF/trailers.

The release contract now explicitly covers:

- `Response::bytes()`;
- `text()` / `json()`;
- `bytes_stream()`;
- raw/decoded streaming;
- redirects using the remaining original budget;
- an already-expired body deadline failing before accepting a ready chunk;
- `TimeoutPhase::Total` winning ties with `Read`.

The total deadline does not reset when a body chunk arrives.

That is the exact behavior Plan 015 had to implement around eggfetch 0.1.5
with an outer Tokio timeout.

After qualification, snip-it should use eggfetch's native `Timeout.total`
and remove the duplicate outer `tokio::time::timeout` ownership.

#### 4. Resolved-target connection reuse is not relevant here

0.1.7 also improves reuse for identical `resolved_addresses()` routes.
That belongs to `advanced-routing`, which this updater should intentionally
omit. Do not enable advanced routing to obtain a feature snip-it does not use.

#### 5. Environment proxies remain opt-in and out of scope

The 0.1.6 changes included in this upgrade also expose explicit
`ProxyEnvironment` handling.

Do not enable the `proxy` feature or begin reading
`HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY` in this plan. Proxy environment
behavior is still not part of snip-it's documented updater contract and would
materially widen both behavior and dependency surface.

## Governing decisions

1. Pin exactly `eggfetch-core =0.1.7` for this qualification.
2. Prefer the lean `standard-http1 + redirects` profile over the full
   `http1` compatibility alias.
3. Keep `tls-rustls + tls-native-roots`; do not change trust semantics.
4. Let eggfetch own redirect traversal using `RedirectPolicy::strict(10)`.
5. Keep snip-it's initial URL scheme guard because strict redirect policy is a
   hop policy, not an initial-URL allow-list.
6. Let eggfetch `Timeout.total` own the network request/body wall-clock
   deadline.
7. Preserve Plan 015's partial staging-file cleanup when a streamed body
   returns an error, including `TimeoutPhase::Total`.
8. Do not add retries, proxy behavior, Basic auth, advanced routing, HTTP/2,
   HTTP/3, compression, cookies, JSON, multipart, tracing, or test-util.
9. Keep the existing direct `futures-util` dependency unless the final code
   no longer needs `StreamExt`; do not change it merely because eggfetch
   also depends on futures-util.
10. Do not migrate `snip-sync` in this plan.

### Why `snip-sync` remains out of scope

Plan 014 measured the server at:

```text
curl baseline:                 3,833,152 bytes
eggfetch 0.1.5 trial:          5,407,400 bytes
growth:                        +1,574,248 / +41.07%
```

Eggfetch's new lean standard profile is a meaningful improvement, but its
isolated full-to-lean saving does not itself demonstrate that the server would
fall anywhere near Plan 014's 10% gate. Cross-binary and cross-target size
deltas must not be extrapolated as exact values.

This plan therefore improves the already-adopted `snp` transport only.
Reopening `snip-sync` should be a separate measurement-driven task if later
evidence specifically suggests the server can meet its footprint gate.

## Part A — dependency/profile migration

### A1. Record the current baseline first

Before changing the dependency, record:

```bash
cargo build --release -p snip-it --bin snp
cargo tree -p snip-it -e features
```

Record the stripped release binary size and the eggfetch-relevant feature
closure.

The current baseline must remain buildable from the pre-change revision so the
comparison is same toolchain / same host / same profile.

### A2. Upgrade and narrow the feature set

Change the root dependency to:

```toml
eggfetch-core = { version = "=0.1.7", default-features = false, features = [
    "standard-http1",
    "redirects",
    "tls-rustls",
    "tls-native-roots",
] }
```

Regenerate `Cargo.lock` normally.

The final `cargo tree -e features` must show the intended eggfetch shape:

```text
present:
  standard-http1
  transport-http1
  standard-route
  high-level-url
  redirects
  tls-rustls
  tls-native-roots

absent from eggfetch selection:
  advanced-routing
  logical-retry
  basic-auth
  proxy
  http2
  http3
  json
  compression-*
  cookies
  multipart
  tracing
  test-util
```

Do not fail merely because another unrelated workspace dependency enables a
similarly named transitive capability elsewhere. Inspect the eggfetch feature
edges specifically.

### A3. Reassess the root Tokio `time` feature

Plan 014 added Tokio's direct `time` feature for the updater's outer
`tokio::time::timeout`.

After Part C removes that wrapper, search the root package for any remaining
direct use of `tokio::time`.

If the root package no longer directly requires the feature, remove `"time"`
from its explicit Tokio feature list. If another root module still uses it,
leave it in place. Do not force removal; Cargo feature unification may make
this a manifest-cleanliness change rather than a linked-byte change anyway.

## Part B — delegate redirect traversal to eggfetch

### B1. Configure one strict redirect policy

Configure the updater client with:

```rust
.redirect_policy(eggfetch_core::RedirectPolicy::strict(MAX_REDIRECTS))
```

Keep:

```text
MAX_REDIRECTS = 10
```

Do not separately call `follow_redirects(true)` and
`redirect_downgrade_policy(...)` unless the exact API shape requires it.
`RedirectPolicy::strict(MAX_REDIRECTS)` expresses the whole intended
contract in one place.

### B2. Keep the initial scheme guard

Before creating/sending the request:

```text
production:
    initial URL must be https

test-support:
    initial URL may be http or https
```

Keep a small `check_url_scheme` / `scheme_allowed` helper or equivalent.

Do not rely on strict redirect policy to reject an initial plaintext URL; it
specifically governs redirect downgrade behavior.

### B3. Delete the local redirect state machine

Once the strict policy is proven, remove the snip-it-owned redirect machinery
that 0.1.5 required:

```text
is_redirect_status
redirect_location
redirect_target
manual redirect for-loop in safe_get
manual relative Location resolution
manual per-hop scheme validation
```

Replace it with one request dispatch:

```text
validate initial scheme
build GET
apply per-request body limit
apply per-request total timeout
send
receive final response after eggfetch redirect handling
```

Keep the helper small. A name such as `fetch_response` is clearer than
`safe_get` once there is no local redirect loop, but renaming is optional.

### B4. Preserve status ownership in snip-it

Eggfetch should return the final response after redirects. Snip-it still owns
the updater-specific status policy:

```text
final 2xx -> consume body
final 404 -> FetchError::NotFound
other final status -> FetchError::Failed
redirect/TLS/timeout/transport error -> FetchError::Failed
```

Do not allow a redirect error or total timeout to become Cargo fallback.

## Part C — replace the Plan 015 outer timeout with native `Timeout.total`

### C1. Preserve connect/read semantics

Keep the existing client-level phase limits:

```text
connect: 10 seconds
read inactivity: 60 seconds
```

Configure them through eggfetch `Timeout` exactly as today.

### C2. Apply one native total timeout per logical request

The production total remains:

```text
FETCH_OVERALL_TIMEOUT = 60 seconds
```

Use an explicit total field:

```rust
eggfetch_core::Timeout::builder()
    .total(overall_timeout)
    .build()
```

Prefer the existing request-level helper seam so tests can continue injecting
short total deadlines while production retains the 60-second constant.

Eggfetch documents request-level timeout values as per-field overrides, so a
request-level `total` should retain client-level `connect` and `read`
defaults.

Do not use `Timeout::from_secs(60)` as a replacement: its scalar constructor
sets pool/connect/write/read but deliberately leaves `total` unset.

### C3. Remove the outer Tokio timeout wrappers

Delete the `tokio::time::timeout(overall_timeout, operation)` wrappers from
`fetch_bytes_with` and `fetch_file_with`.

After the change:

```text
metadata:
    request starts with eggfetch total deadline
    redirects consume same deadline
    final response.bytes() remains under same deadline

binary:
    request starts with eggfetch total deadline
    redirects consume same deadline
    bytes_stream() remains under same deadline
    a total timeout surfaces as a stream error
    existing local failure cleanup removes the partial file
```

The updater should have one total-deadline owner, not nested competing timers.

### C4. Preserve partial-file cleanup

Keep `remove_partial_staging_file(path)` or the equivalent small helper.

Any failure returned while streaming—including:

- decoded body limit;
- read timeout;
- native total timeout;
- transport failure;
- local write failure;

must close/drop the file and best-effort remove the staging path.

Do not add async filesystem I/O or a cleanup framework.

### C5. Timeout error mapping

A native total timeout is an ordinary hard transport failure.

If user-facing output becomes materially less clear after removing the outer
wrapper, map:

```rust
eggfetch_core::Error::Timeout {
    phase: eggfetch_core::TimeoutPhase::Total,
    ..
}
```

to a concise updater-level message indicating the overall request deadline was
exceeded.

Do not redesign `FetchError` solely for this dependency upgrade.

## Part D — requalify the updater tests against the native policies

Use the existing std-only loopback fixture. Do not add a mock HTTP framework,
TLS test framework, or second client.

### D1. Keep policy tests that remain application-owned

Retain or adapt tests proving:

1. initial production HTTP is rejected before network I/O;
2. test-support HTTP fixture endpoints remain accepted;
3. metadata 2xx succeeds;
4. metadata over 1 MiB fails;
5. binary 2xx streams to disk;
6. binary over 256 MiB fails and leaves no usable partial;
7. final binary 404 maps to the existing missing-asset fallback path;
8. checksum 404 remains a hard failure;
9. 401/403/5xx remain hard failures;
10. connect/read timeouts remain hard failures;
11. relative redirects still work;
12. redirect loop/depth overflow remains a hard failure;
13. the two Plan 015 slow-drip tests still prove an overall deadline even
    while body progress continues;
14. timed-out binary streaming removes the partial staging file.

### D2. Rework redirect-security coverage without duplicating eggfetch internals

The current downgrade test exists because snip-it implemented the redirect
state machine itself.

After delegation, do not recreate eggfetch's own strict-redirect test suite in
snip-it.

Instead:

- make the updater's redirect policy construction a tiny inspectable helper if
  useful;
- assert that it is configured with:
  - `follow = true`;
  - `max_redirects = MAX_REDIRECTS`;
  - `downgrade = RedirectDowngradePolicy::Deny`;
- keep an end-to-end relative redirect test and redirect-limit test through
  the snip-it adapter;
- rely on eggfetch 0.1.7's qualified redirect tests for the internal
  "reject HTTPS -> HTTP before second-hop I/O" implementation.

Do not add a local TLS fixture solely to retest the dependency's already
qualified strict redirect engine.

The snip-it acceptance requirement is that the updater demonstrably selects
the strict policy and never overrides it per request.

### D3. Make the slow-drip tests prove native total timeout

The Plan 015 regression tests are especially valuable for this upgrade.

Keep the same shape:

```text
headers arrive immediately
body chunks arrive every ~100 ms
read inactivity timeout is comfortably larger
total timeout is ~300 ms
full body would take ~2 seconds
```

But make them pass solely because the request carries eggfetch
`Timeout.total`.

There must be no outer `tokio::time::timeout` around the operation.

For the binary test, continue asserting:

```text
at least one body chunk was sent
operation fails as total timeout
staging path does not exist afterwards
```

This directly qualifies the 0.1.7 body-lifecycle correction in snip-it's real
usage pattern.

### D4. Preserve architecture checks

Keep the existing source-level guarantee that the root updater does not shell
out to `curl`.

Optionally tighten the updater architecture test if it is simple and stable to
pin these facts:

```text
Cargo.toml pins eggfetch-core =0.1.7
root updater does not call tokio::time::timeout
root updater does not contain the old manual redirect helpers
```

Do not add brittle source scanning for every feature or helper spelling; Cargo
tree verification and behavior tests are authoritative.

## Part E — footprint qualification

This release specifically introduces a footprint-oriented profile, so size
measurement is part of acceptance.

### E1. Build three controlled profiles

Using the same revision/toolchain/target and release settings, record:

```text
A. current baseline
   eggfetch 0.1.5:
   http1,tls-rustls,tls-native-roots

B. 0.1.7 compatibility control
   eggfetch 0.1.7:
   http1,tls-rustls,tls-native-roots

C. intended 0.1.7 lean updater
   eggfetch 0.1.7:
   standard-http1,redirects,tls-rustls,tls-native-roots
```

At minimum:

```bash
cargo build --release -p snip-it --bin snp
cargo tree -p snip-it -e features
```

Record bytes and percentage delta for B and C relative to A.

Profile B is diagnostic only. Do not keep it merely because it is the easiest
version bump.

### E2. Expected decision

The intended production choice is profile C because it removes capability the
updater does not use and allows deletion of local redirect/timeout policy.

Accept C when:

- all behavior tests pass;
- the intended feature boundary is confirmed;
- there is no material size regression relative to the current 0.1.5 build.

For this narrow upgrade, treat more than either:

```text
+128 KiB
or
+2%
```

relative to the controlled current `snp` baseline as an unexpected material
regression requiring investigation before the change is kept.

This is not permission to grow by that amount; it is the point at which the
implementer must stop and diagnose.

If profile C is unexpectedly larger:

1. inspect `cargo tree -e features` for accidental full `http1`,
   `advanced-routing`, retry, Basic auth, proxy, or other broad features;
2. measure `standard-http1,tls-rustls,tls-native-roots` without
   `redirects` as a diagnostic to isolate the built-in redirect cost;
3. do not silently revert to insecure redirect behavior;
4. if the redirect feature alone is genuinely responsible for a material
   regression, report the measurements and retain the existing manual
   redirect loop as the narrow fallback rather than enabling the full
   compatibility profile.

A measurement-only fallback does not reopen Plan 014's `snip-sync`
migration.

### E3. No new server size trial

Do not add eggfetch to `snip-sync/Cargo.toml` during this plan.

The server remains on the measured curl path from Plan 014.

## Part F — cleanup and documentation

If the preferred profile qualifies:

- remove manual redirect helpers no longer used;
- remove outer Tokio total-timeout wrappers;
- update nearby comments so eggfetch owns strict redirects and the absolute
  body-lifecycle deadline;
- update `AGENTS.md` / architecture documentation only where they currently
  describe the 0.1.5 feature profile or Plan 015 timeout ownership;
- update `CHANGELOG.md` under `Unreleased` because the dependency/profile
  and updater internals changed after the existing 1.3.9 release;
- update Plan 014/015 historical text only if a forward pointer is useful;
  do not rewrite their recorded historical measurements or rationale;
- mark this plan complete and update `plans/README.md` with actual version,
  profile, test, feature-tree, and size results.

Documentation should make the current state obvious:

```text
snp:
  eggfetch-core 0.1.7
  standard HTTP/1.1 route
  strict redirects
  native total request/body deadline
  native roots
  no external curl

snip-sync:
  external curl retained by Plan 014 size gate
```

## Expected file touch set

Primary:

```text
Cargo.toml
Cargo.lock
src/update.rs
plans/016-eggfetch-0.1.7-lean-updater-adoption.md
plans/README.md
CHANGELOG.md
```

Likely documentation touch only if stale wording exists:

```text
AGENTS.md
architecture/overview.md
```

Tests should remain inline in `src/update.rs` unless the existing fixture
becomes objectively clearer by moving; do not create a new test framework.

Do not touch:

```text
snip-sync/Cargo.toml
snip-sync/src/update.rs
release workflow
installers
service lifecycle
sync gRPC path
```

## Suggested implementation order for smaller-model handoff

1. Read Plans 014 and 015 completion notes and current `src/update.rs`.
2. Record current 0.1.5 `snp` release size and feature tree.
3. Temporarily build the 0.1.7 full-`http1` compatibility control and record
   size/tree; do not keep it as the final profile.
4. Change to the intended
   `standard-http1,redirects,tls-rustls,tls-native-roots` profile.
5. Configure `RedirectPolicy::strict(MAX_REDIRECTS)`.
6. Replace the manual redirect loop with one eggfetch request dispatch while
   retaining the initial URL scheme guard.
7. Add native request `Timeout.total` using the existing injected total
   duration.
8. Remove the outer Tokio timeout wrappers.
9. Keep binary stream-error cleanup and ensure native total errors reach it.
10. Update the redirect tests to validate snip-it configuration/integration,
    not eggfetch internals.
11. Re-run both Plan 015 slow-drip tests against native `Timeout.total`.
12. Run focused updater tests.
13. Measure final profile C size and feature tree.
14. If the size is unexpectedly worse, run only the diagnostic fallback
    profile described in Part E; do not broaden scope.
15. Run full workspace/platform verification.
16. Update docs/changelog only where current behavior changed.
17. Fill completion notes below, mark Plan 016 complete, and update
    `plans/README.md` in the implementation commit.

## Required verification

At minimum:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p snip-it --features test-support --bin snp
cargo test --workspace --all-features -- --test-threads=1
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
cargo build --release -p snip-it --bin snp
cargo tree -p snip-it -e features
```

Also run ordinary GitHub Actions through the pushed implementation commit and
require the existing Linux correctness, macOS platform smoke, and Windows
platform smoke jobs to remain green.

No new CI job is required for this dependency bump.

## Acceptance criteria

Plan 016 is complete when all applicable statements are true:

1. Root `snp` pins `eggfetch-core =0.1.7`.
2. The production feature profile uses
   `standard-http1,redirects,tls-rustls,tls-native-roots`, not the broad
   `http1` alias.
3. Eggfetch advanced routing, logical retry, Basic auth, proxy, HTTP2/3,
   JSON, compression, cookies, multipart, tracing, and test-util are not
   enabled by the updater dependency.
4. Initial production URLs remain HTTPS-only before network I/O.
5. Redirect traversal is delegated to eggfetch with
   `RedirectPolicy::strict(MAX_REDIRECTS)`.
6. The old local redirect loop/Location-resolution helpers are removed when
   the preferred profile qualifies.
7. Relative redirects and redirect-depth failures continue to behave
   correctly through the updater adapter.
8. The updater uses native `Timeout.total` with a 60-second production
   budget.
9. There is no outer `tokio::time::timeout` duplicating the logical fetch
   deadline.
10. Plan 015 slow-drip metadata and binary tests pass using eggfetch's native
    body-lifecycle total deadline.
11. Native total timeout remains a hard failure and can never trigger Cargo
    fallback.
12. Timed-out/failed binary streaming leaves no usable staging file.
13. Existing 1 MiB / 256 MiB body limits remain unchanged.
14. Existing 404-only fallback, checksum, candidate identity/version,
    Homebrew/Cargo, replacement, and lifecycle behavior remain unchanged.
15. TLS remains Rustls with native roots and packaged-root fallback semantics.
16. No proxy behavior or automatic retry is introduced.
17. `snip-sync` remains unchanged on its Plan 014 curl path.
18. Controlled A/B/C size and feature-tree measurements are recorded.
19. Any material size regression is investigated according to Part E rather
    than silently accepted.
20. Full workspace checks and GitHub Actions are green.
21. Changelog/docs and plan index describe the actual final 0.1.7 profile and
    timeout/redirect ownership.

## Explicit non-goals

Do not use this plan to:

- migrate `snip-sync` to eggfetch;
- redesign the self-update release/tag/asset policy;
- add updater retries;
- add environment proxy support;
- enable Basic auth;
- add HTTP/2 or HTTP/3;
- change trust roots;
- change checksum or candidate-verification policy;
- alter Cargo fallback semantics;
- replace Tonic sync traffic;
- change Axum/Tonic server networking;
- add async filesystem I/O;
- add a shared updater crate;
- add a generic HTTP abstraction;
- redesign the global Tokio runtime;
- modify installers or release workflows;
- optimize unrelated dependencies;
- re-test every eggfetch internal redirect/timeout invariant already covered by
  eggfetch's own 0.1.7 qualification.

## Completion notes

```text
Implementation commit: (this commit; see git log for the Plan 016 message)
eggfetch-core version: =0.1.7
final eggfetch features: standard-http1, redirects, tls-rustls,
    tls-native-roots (default-features = false)
0.1.5 current baseline bytes: 6973056 (profile A: =0.1.5 http1 profile,
    same toolchain/host/profile, `cargo build --release -p snip-it --bin snp`)
0.1.7 full-http1 control bytes: 6973056 (profile B: =0.1.7 http1 profile;
    delta 0 vs A)
0.1.7 lean+redirects final bytes: 6776320 (profile C: intended production
    profile; delta -196736 / -2.82% vs A)
final delta bytes / %: -196736 / -2.82% (shrink; far inside the +128 KiB /
    +2% investigation gate, so no fallback diagnosis was needed)
feature-tree observations: `cargo tree -e features -i eggfetch-core` shows
    standard-http1, transport-http1, standard-route, high-level-url,
    redirects, tls-rustls (+hyper-rustls), tls-native-roots. Absent from the
    eggfetch selection: advanced-routing, logical-retry, basic-auth, proxy,
    http2, http3, json, compression-*, cookies, multipart, tracing,
    test-util.
manual redirect helpers removed: yes (`is_redirect_status`,
    `redirect_location`, `redirect_target`, manual for-loop in `safe_get`
    deleted; replaced by one `fetch_response` dispatch through eggfetch's
    strict redirect handling). Initial-URL HTTPS guard (`check_url_scheme` /
    `scheme_allowed`) kept: strict policy governs hops, not the initial URL.
outer Tokio total wrapper removed: yes (both
    `tokio::time::timeout(overall_timeout, ...)` wrappers deleted from
    `fetch_bytes_with` / `fetch_file_with`). One native request
    `Timeout.total` per logical request now owns the deadline; client keeps
    connect (10 s) + read (60 s) phase limits and request-level `total`
    merges per-field without resetting them. Root Tokio `time` feature kept:
    `src/sync.rs` still uses `tokio::time` directly, so removal was not
    applicable (per Part A3).
native total slow-drip metadata result: pass
    (`metadata_slow_drip_body_still_hits_overall_timeout`: 20x100-byte
    chunks at 100 ms gaps, 300 ms total budget, fails with "timed out",
    body progress proven via chunks_sent >= 1, no outer Tokio timeout)
native total slow-drip binary/cleanup result: pass
    (`binary_slow_drip_body_hits_overall_timeout_without_partial`: same
    shape through the streamed path, stream error mapped from native Total
    timeout, staging path absent afterwards)
focused updater test result: 21 passed (`cargo test -p snip-it --features
    test-support --bin snp update::`); full bin suite 67 passed. Includes
    strict-policy config test, relative + absolute redirect integration,
    redirect-loop depth failure, 1 MiB / streamed-limit bounds, 404-only
    fallback vs checksum-404 hard failure, and production-seam HTTP
    rejection before I/O
workspace/all-features result: pass
    (`cargo test --workspace --all-features -- --test-threads=1`: every
    target ok, 0 failed; scanned for FAILED/error lines, none found)
scripts/check.sh result: pass (`=== All checks passed ===`: installers,
    fmt, clippy, lib, platform_smoke, destination_permissions,
    auto_sync_closure, auto_sync_concurrency, sync_multibatch)
production-seams result: pass
    (`bash scripts/ci/test-production-seams.sh`: all seam tests passed)
GitHub Actions result: pass (CI run 35400092370 for the implementation
    commit: Linux correctness, macOS platform smoke, and Windows platform
    smoke all success)
docs/changelog updated: CHANGELOG.md Unreleased (0.1.7 lean profile +
    -2.8% size note), AGENTS.md updater bullet, tests/architecture.rs
    (lean-profile + delegated timeout pins), this plan + plans/README.md.
    Plan 014/015 historical text untouched. README.md needed no change
    (no version-specific updater wording). No skill/architecture deep-dive
    described the 0.1.5 profile, so no pruning applied there.
unexpected deviations: none. Profile B measured byte-identical to profile A
    (6973056), so the full -2.8% saving is attributable to the lean
    standard-route boundary (dropping advanced-routing, logical-retry, and
    basic-auth). `futures-util` direct dependency retained (`StreamExt`
    still used by `stream_binary_to_file`). `snip-sync` untouched on curl.
```
