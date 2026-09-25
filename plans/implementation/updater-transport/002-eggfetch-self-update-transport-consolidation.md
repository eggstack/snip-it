> **Planning-convention migration note.** This file was promoted via `git mv` from pre-convention flat plan `014-eggfetch-self-update-transport-consolidation.md` (history preserved). Its body is unchanged historical evidence.
> New-convention identity: Updater Transport M002 — eggfetch self-update transport consolidation.
> Source roadmap: `plans/subsystems/updater-transport-roadmap.md` (M002).
> Long-term requirements: `plans/000-long-term-specification.md`, `plans/001-terminology-and-domain-model.md`, `plans/002-long-term-roadmap.md`.
> Primary class: infrastructure. Dependencies: hard M001.
> Status vocabulary mapping: pre-convention `Status: complete` means `closed` with closure record at `plans/closure/updater-transport/002-status.md`.
> Pre-convention archive pointer: `plans/archive/flat-014-eggfetch-self-update-transport-consolidation.md`.

---

# Plan 014: eggfetch self-update transport consolidation

Status: complete

Depends on: Plan 004 (complete)

## Objective

Replace the duplicated external-`curl` HTTP transport used by `snp update` and `snip-sync update` with the current `eggfetch-core` 0.1.5 Rust client where doing so reduces maintenance and external-command dependence without materially bloating the binaries.

This is a transport consolidation pass, not an updater redesign.

The release/version/checksum/replacement policy implemented by Plan 004 remains authoritative. The intended outcome is:

```text
updater policy owned by snip-it
        |
        +-- crates.io version lookup
        +-- exact GitHub tag/asset mapping
        +-- 404-only Cargo fallback classification
        +-- checksum/candidate validation
        +-- executable replacement/lifecycle handling
        |
        `-- HTTP mechanics delegated to eggfetch-core
```

Do not alter the gRPC sync path, Axum server path, release workflow, installer contract, or service lifecycle architecture as part of this work.

## Current-state findings

As of 2026-09-16 on `main`:

- The root `snip-it` package is version `1.3.9` and `snip-sync` is version `0.1.6`.
- Neither manifest declares `reqwest`. There is therefore no `reqwest` dependency to remove in this repository.
- Both updater implementations contain near-duplicate `curl` subprocess adapters:
  - `src/update.rs` for `snp`;
  - `snip-sync/src/update.rs` for `snip-sync`.
- Both adapters currently provide the same important behavior:
  - production URLs must use HTTPS;
  - test features may inject HTTP fixture URLs;
  - redirects are followed;
  - production redirect protocols are constrained by `curl --proto =https`;
  - TLS 1.2 or newer is required;
  - connect timeout is 10 seconds;
  - overall curl operation timeout is 60 seconds;
  - metadata bodies are capped at 1 MiB;
  - binary bodies are capped at 256 MiB;
  - User-Agent is `snip-it-update`;
  - HTTP 404 is distinguished from other failures.
- `snp` already owns a reusable Tokio runtime in `src/main.rs`; the update command currently calls a synchronous `update::run` directly.
- `snip-sync` keeps a synchronous `main` and constructs a Tokio runtime only where needed; the update command is currently synchronous.
- Plan 004 deliberately selected `curl` because it avoided introducing another TLS client stack into `snip-sync`. That was a reasonable tradeoff before eggfetch was tightened for this use case and must remain part of the size decision.

Current public release artifacts provide useful external size sanity points, especially for the SBC-oriented Linux ARM64 path:

```text
v1.3.9 / snp-aarch64-unknown-linux-gnu             6,225,032 bytes
snip-sync-v0.1.6 / snip-sync-aarch64-unknown-linux-gnu 3,861,496 bytes
```

Use same-toolchain before/after release builds for the actual acceptance comparison; public release sizes are reference points, not a substitute for a controlled comparison.

## eggfetch 0.1.5 findings relevant to this migration

`eggfetch-core` 0.1.5 provides the needed primitives without enabling its broad optional surface:

- HTTP/1.1 can be selected independently.
- Rustls TLS can be selected independently.
- native trust roots can be enabled explicitly, with packaged WebPKI fallback during construction.
- response bodies can be buffered or streamed.
- `Response::status()`, `headers()`, `url()`, `bytes()`, and `bytes_stream()` expose everything the updater needs.
- request-level decoded-body limits apply to both buffered and streaming bodies.
- client/request timeouts support connect, read, and total phases.
- automatic decompression can be disabled.
- JSON support is optional and is unnecessary because snip-it already uses `serde_json`.
- HTTP/2, HTTP/3, compression, cookies, multipart, proxy, tracing, and eggfetch test utilities are unnecessary for this updater.

Important compatibility details:

1. Eggfetch's native redirect policy currently allows both `http` and `https`. Do **not** replace the current `curl --proto =https` behavior with unconditional `follow_redirects(true)`, because that would permit an HTTPS-to-HTTP downgrade before the caller can reject the final URL.
2. Eggfetch's native Rust API does not automatically read proxy environment variables. System `curl` commonly does. Proxy environment support is not part of snip-it's documented updater contract today; do not pull the eggfetch `proxy` feature and add environment parsing solely to reproduce an undocumented side effect unless a concrete supported deployment requires it.
3. Use the native-root TLS profile first because it is the closest match to the current system-client trust behavior. The WebPKI-only profile may be measured as a footprint experiment, but do not silently switch trust semantics merely to save bytes.

## Governing constraints

1. Keep Plan 004's exact-version release policy unchanged.
2. Cargo fallback remains allowed only for an intentionally source-only target or a definite missing binary asset (`HTTP 404`).
3. Transport, TLS, timeout, checksum, malformed response, HTTP 5xx, HTTP 401/403, candidate identity, and candidate version failures remain hard failures.
4. Do not add a self-update framework.
5. Do not add a new shared workspace/published crate solely to deduplicate the two thin updater adapters.
6. Do not put updater transport code into `snip-proto`.
7. Do not replace Tonic gRPC sync traffic with eggfetch.
8. Do not replace Axum/Tonic server networking with eggfetch.
9. Do not enable eggfetch HTTP/2, HTTP/3, JSON, compression, cookies, proxy, multipart, tracing, or test-util features for this work.
10. Do not add async file I/O solely for the updater. A one-shot updater may stream network chunks into `std::fs::File` with ordinary blocking writes; this avoids another Tokio feature expansion and does not create a long-lived service hot path.
11. Preserve the existing test-only endpoint injection seams (`test-support` for `snp`, `test-helpers` for `snip-sync`).
12. Keep this project lightweight. If the `snip-sync` binary-size cost is material after feature minimization, it is acceptable for this plan to land eggfetch only in `snp` and retain the existing curl adapter in `snip-sync` with the measurement recorded.

## Part A — add the narrow eggfetch dependency profile

### A1. Root `snp` manifest

Add the current known-good core dependency explicitly:

```toml
eggfetch-core = {
    version = "=0.1.5",
    default-features = false,
    features = ["http1", "tls-rustls", "tls-native-roots"]
}
```

Pin 0.1.5 for this first integration so updater behavior does not change underneath the migration while eggfetch is still on the pre-1.0 line. A later maintenance pass may intentionally relax/bump the pin after qualification.

For streaming release assets, add the minimum direct `futures-util` dependency needed for `StreamExt`/`next`. Prefer the same narrow feature shape already used by eggfetch where practical, for example:

```toml
futures-util = { version = "0.3", default-features = false, features = ["alloc"] }
```

If that exact feature set does not expose the required extension method under this compiler, enable only the smallest additional futures-util feature required. Do not enable unrelated async utilities by default.

### A2. `snip-sync` manifest

Use the same eggfetch/futures-util dependency profile if the server updater is included in the implementation branch.

Do not create a workspace dependency table merely for two entries unless it actually removes manifest duplication without making package publication harder to understand.

### A3. Lockfile and graph inspection

Regenerate `Cargo.lock` normally.

Record before/after output for the relevant release graphs:

```bash
cargo tree -p snip-it -e features
cargo tree -p snip-sync -e features
```

Confirm none of these eggfetch features appear accidentally:

```text
http2
http3
json
compression-*
cookies
proxy
multipart
tracing
test-util
```

Do not attempt to remove Tonic/Hyper/Tokio dependencies that are required elsewhere merely because eggfetch also uses them.

## Part B — implement one small updater HTTP adapter per binary

Keep the adapter inside the existing `update.rs` files. The two packages are independently published and the duplicated adapter should become small enough that another published helper crate would cost more than it saves.

The local adapter should own only these concerns:

```text
build configured eggfetch client
validate initial URL policy
GET with safe redirect handling
classify final HTTP status
buffer bounded metadata
stream bounded binary to a file
map eggfetch errors into existing FetchError
```

Everything above that layer remains existing updater policy.

### B1. Client configuration

Create one small helper such as `update_http_client()` with behavior equivalent to:

```text
HTTP/1.1 only by selected Cargo features
User-Agent: snip-it-update
automatic decompression: disabled
automatic redirects: disabled
connect timeout: 10 seconds
read inactivity timeout: no more than 60 seconds
```

Use an outer 60-second `tokio::time::timeout` around each complete logical fetch operation, including redirect traversal and final body consumption. This is the important parity point with curl's `--max-time 60`: a chain of redirects must not receive a fresh 60-second wall-clock budget for every hop, and a streamed binary download must not run indefinitely after response headers arrive.

An eggfetch `Timeout` with at least the connect/read phases configured is appropriate, but the outer timeout remains the authoritative full-operation bound.

Do not configure an explicit retry policy. The current updater does not intentionally retry HTTP failures and should not begin hiding transport/release defects during this consolidation.

Set `automatic_decompression(false)` even though no compression feature is selected. Release binary/checksum bytes must remain byte-for-byte transport content and the intent should be explicit.

### B2. Preserve HTTPS-only production redirects

Implement a bounded GET-only redirect loop locally instead of enabling eggfetch automatic redirects.

Suggested shape:

```text
current_url = initial URL
for hop in 0..=MAX_REDIRECTS:
    validate scheme before sending
    send GET with redirects disabled
    if status is 301/302/303/307/308:
        require Location
        resolve Location relative to response.url()
        validate resolved scheme before any next request
        current_url = resolved URL
        continue
    return final response
fail: too many redirects
```

Use a small fixed redirect bound such as 10. GitHub/crates.io update endpoints do not need deep chains; the purpose is loop/bad-endpoint containment, not a general browser policy.

Production scheme policy:

```text
allowed: https
rejected before network I/O: http and everything else
```

Test-feature scheme policy:

```text
allowed: http, https
```

This must apply to **every redirect hop**, not only the initial and final URL.

Relative `Location` values should be resolved against `response.url()`. Avoid adding a direct `url` dependency if the existing eggfetch response URL type lets the adapter call `join()` directly with type inference.

Do not weaken the rule to "follow first, inspect final URL later". By then an insecure request would already have been sent.

### B3. `fetch_bytes`

Convert the metadata/checksum fetcher to async and keep the existing `FetchError` classification.

Required behavior:

1. perform the safe GET/redirect flow;
2. apply `max_decoded_body_size(MAX_METADATA_BYTES as usize)` on every request/hop that could become the final response;
3. on final `2xx`, consume `response.bytes().await` and return the bytes;
4. on final `404`, return `FetchError::NotFound`;
5. on every other final status, return `FetchError::Failed("HTTP <status> ...")` without attempting Cargo fallback;
6. map DNS/connect/TLS/timeout/body-limit/stream errors to `FetchError::Failed`;
7. do not parse or retain a large error response body merely to improve the message.

Keep `serde_json` parsing in the updater; do not enable eggfetch's `json` feature.

### B4. `fetch_file`

Convert the binary fetcher to async and stream the final 2xx body to the staging path.

Required behavior:

1. perform the same safe GET/redirect flow;
2. apply `max_decoded_body_size(MAX_BINARY_BYTES as usize)`;
3. classify status **before** creating/truncating the destination where practical;
4. on final `404`, return `FetchError::NotFound`;
5. on final non-2xx/non-404, hard fail;
6. on final 2xx, create the staging file and consume `response.bytes_stream()` chunk by chunk;
7. write each chunk with `std::io::Write::write_all`;
8. if stream/body-limit/timeout/write failure occurs, close and best-effort remove the partial candidate before returning the error;
9. never buffer a potentially 256 MiB release asset into memory.

The existing SHA-256 verification remains the authoritative integrity check after a complete download.

### B5. Keep 404 policy at the existing caller boundary

Do not collapse all HTTP errors into strings too early.

The existing call sites rely on this distinction:

```text
binary asset 404 -> MissingAsset -> Cargo fallback allowed
checksum 404     -> hard failure
crates metadata 404 -> hard failure
anything else    -> hard failure
```

Preserve that exact behavior.

## Part C — async integration without changing command architecture

### C1. `snp`

Change the updater entry point to async:

```rust
pub async fn run(...) -> Result<(), String>
```

Propagate `.await` only through updater functions that perform HTTP operations (`latest_crates_version`, `download_candidate`, `fetch_bytes`, `fetch_file`, and direct callers as necessary).

Keep filesystem, Cargo, Homebrew, candidate execution, hashing, and executable replacement code synchronous. Do not convert unrelated updater code to async merely for stylistic consistency.

At the existing `Commands::Update` dispatch, reuse the already-created global runtime:

```text
RUNTIME.block_on(update::run(...))
```

Do not create a second runtime for `snp update`.

The internal Windows self-replacement helper remains synchronous.

### C2. `snip-sync`

Keep `main()` synchronous.

For the `Command::Update` arm, create/block on a Tokio runtime only for the update operation, using the same simple runtime construction style already present in the server binary. Do not convert the whole CLI to `#[tokio::main]` and do not make lifecycle/file commands async.

The server's existing `serve()` runtime remains separate from one-shot update execution because those commands are mutually exclusive.

The internal Windows replacement helper remains synchronous.

## Part D — preserve and extend updater tests

Use the existing test-only endpoint injection instead of introducing a mock HTTP framework.

A tiny local HTTP fixture using existing standard-library/Tokio capabilities is sufficient. Do not add `wiremock`, `httpmock`, a test web framework, or a second HTTP client.

Required deterministic coverage for both updater adapters where applicable:

1. initial production HTTP URL is rejected before network I/O;
2. test-feature HTTP fixture URL is accepted;
3. metadata 2xx body is returned correctly;
4. metadata body over 1 MiB fails;
5. binary 2xx body streams to disk correctly;
6. binary body over 256 MiB fails without leaving a usable partial candidate;
7. final binary asset 404 maps to `FetchError::NotFound` / existing `MissingAsset` fallback path;
8. checksum 404 remains a hard failure rather than Cargo fallback;
9. HTTP 401/403/5xx remain hard failures;
10. connect/read/overall timeout failure remains hard failure;
11. relative redirect resolves correctly;
12. HTTPS -> HTTPS redirect is accepted in production policy;
13. HTTPS -> HTTP redirect is rejected **before** the HTTP target is requested;
14. redirect loop/depth overflow is rejected;
15. transport works when `curl` is absent from `PATH`;
16. existing checksum, candidate identity/version, host mapping, replacement, and lifecycle tests continue to pass unchanged.

For the downgrade test, make the HTTP target increment an observable request counter or otherwise prove it was never contacted. Merely checking the returned error string is insufficient.

Do not add tests for general eggfetch behavior already covered in eggfetch itself. Test only the snip-it adapter policy and integration contract.

## Part E — controlled footprint and dependency acceptance gate

This migration is expected to reduce source duplication and external-command dependence. It is **not** automatically a binary-size reduction because the current updater shells out to system curl instead of embedding an HTTP/TLS client.

Measure before making a final keep/revert decision.

### E1. Controlled build comparison

Before the dependency change, record stripped release sizes from the same revision/toolchain/target setup used for the after build.

At minimum compare:

```bash
cargo build --release -p snip-it --bin snp
cargo build --release -p snip-sync --bin snip-sync
```

Prefer Linux x86_64 plus Linux aarch64 when the project/release environment can build both without introducing emulation solely for this measurement. Linux aarch64 is especially relevant because SBC distribution is a project goal.

Record:

```text
binary                  before bytes    after bytes    delta bytes    delta %
snp                     ...             ...            ...            ...
snip-sync               ...             ...            ...            ...
```

Also inspect feature/dependency deltas with `cargo tree -e features`. `cargo bloat` may be used if already available to the implementer, but do not add it to repository CI or dependencies just for this plan.

### E2. Size decision

Use 10% growth in a stripped representative release binary as the "material regression" trigger for this pass, not as an automatic failure of the whole line of work.

Decision policy:

- `snp` at <=10% growth: keep eggfetch if all behavioral tests pass.
- `snp` >10% growth: inspect accidental features first; test the WebPKI-only profile only as a measurement. Do not change trust policy without documenting the compatibility tradeoff. If growth remains material, report it before keeping the migration.
- `snip-sync` at <=10% growth: keep eggfetch if tests pass.
- `snip-sync` >10% growth after confirming the narrow feature profile: retain/revert to the existing curl adapter for `snip-sync` and still allow the `snp` migration to land independently.

The server binary is deliberately small and previously avoided an embedded TLS client. Do not force architectural symmetry at the expense of the lightweight-server goal.

If only `snp` migrates, add a short comment near the retained `snip-sync` curl adapter explaining that the split is intentional and measurement-driven so a future cleanup does not "fix" it blindly.

## Part F — documentation and cleanup

If both binaries migrate successfully:

- delete `curl_protocol()` and the `Command::new("curl")` paths from both updater modules;
- remove curl-specific error wording/tests;
- keep ordinary `Command` imports because Cargo, Homebrew, candidate verification, and Windows replacement still use subprocesses;
- update any user-facing statement that says self-update requires curl, if such a statement exists;
- note in the changelog/release notes that self-update HTTP is now handled in-process through eggfetch and no longer depends on an external curl executable.

If only `snp` migrates:

- perform the same cleanup only in the root updater;
- leave the `snip-sync` curl path intentionally intact;
- document the measured reason in this plan's completion notes.

Do not add a new HTTP abstraction layer above eggfetch after deleting the curl wrapper. The thin updater adapter itself is the abstraction boundary.

## Expected file touch set

Primary files:

```text
Cargo.toml
Cargo.lock
src/main.rs
src/update.rs
snip-sync/Cargo.toml
snip-sync/src/main.rs
snip-sync/src/update.rs
plans/014-eggfetch-self-update-transport-consolidation.md
plans/README.md
```

Possible focused test files may be added only if the existing inline updater tests become materially clearer by moving local fixture code out. Avoid unrelated refactors.

## Suggested implementation order for a smaller model

Execute in this order and keep each step compiling:

1. Record pre-change release binary sizes and `cargo tree -e features` output.
2. Add eggfetch/futures-util to the root `snp` package only; update lockfile.
3. Implement the root async HTTP adapter with redirects disabled in eggfetch.
4. Convert only the root updater's HTTP call chain to async and reuse `RUNTIME` at command dispatch.
5. Add/adjust root updater transport tests and get the root package green.
6. Measure root release size and inspect accidental features. Resolve root acceptance before copying the pattern.
7. Add the same dependency profile to `snip-sync`.
8. Port the already-proven adapter pattern into `snip-sync/src/update.rs` without introducing a shared crate.
9. Add the narrow runtime block for `snip-sync update`; keep the rest of `main` synchronous.
10. Run/update server updater tests.
11. Measure `snip-sync` release size and apply the 10% decision rule.
12. Remove curl-specific code only for components that passed the gate.
13. Run full workspace/platform checks.
14. Record final measurements and decisions in this plan, mark it complete, and update `plans/README.md` in the same completion commit.

Do not begin by refactoring both update modules simultaneously. Prove the adapter in `snp`, then copy the small established pattern. This reduces the chance that an async/redirect mistake is duplicated across both packages.

## Required verification

At minimum run:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --release -p snip-it --bin snp
cargo build --release -p snip-sync --bin snip-sync
```

Also run the repository's existing platform/release verification scripts that exercise updater/release behavior if they are part of the normal pre-release contract. Do not weaken Windows/platform CI to make the dependency change land.

Where cross-target release builds are supported by the current workflow/tooling, verify the same dependency profile compiles for:

```text
x86_64-unknown-linux-gnu
aarch64-unknown-linux-gnu
x86_64-apple-darwin
aarch64-apple-darwin
x86_64-pc-windows-msvc
```

No new target is required by this plan.

## Acceptance criteria

This plan is complete when all applicable statements are true:

1. `snp update` no longer shells out to curl and uses `eggfetch-core` 0.1.5 with only the required H1/Rustls/native-root feature profile.
2. `snip-sync update` also uses eggfetch **if and only if** its controlled size comparison stays within the material-growth gate or an explicitly recorded project decision accepts the measured cost.
3. Any intentionally retained `snip-sync` curl path is documented as a measured footprint tradeoff, not unfinished migration work.
4. Production initial URLs and every followed redirect hop are HTTPS-only.
5. An HTTPS-to-HTTP redirect is rejected before any plaintext target request occurs.
6. Test-only HTTP endpoint injection still works under the existing test features.
7. Metadata remains bounded to 1 MiB.
8. Release binaries remain streamed and bounded to 256 MiB without whole-file buffering.
9. Failed/oversized/timeout downloads do not leave a usable partial candidate.
10. HTTP 404 remains distinguishable from all other failures.
11. Cargo fallback remains limited to source-only targets or a definite missing binary asset.
12. Checksum/candidate/replacement/lifecycle behavior from Plan 004 is unchanged.
13. `snp` reuses its existing Tokio runtime rather than creating another one.
14. `snip-sync` remains a synchronous CLI/server entry point with async runtime use limited to commands that need it.
15. No eggfetch optional feature outside the approved set is enabled accidentally.
16. No new updater/shared workspace crate or self-update framework is introduced.
17. The updater functions successfully with no `curl` executable available for each component that migrated.
18. Controlled before/after release binary sizes and feature-tree observations are recorded in the plan's completion notes.
19. Full workspace formatting/check/test/clippy verification passes.
20. Existing release/install/platform behavior remains green.

## Explicit non-goals

Do not use this plan to:

- replace Tonic with HTTP/JSON sync;
- replace Axum/Tonic server networking;
- redesign release tags/assets;
- change installer selection/fallback policy;
- add automatic HTTP retries;
- add updater telemetry;
- add proxy auto-detection unless it becomes an explicit supported requirement;
- add HTTP/2 or HTTP/3 to update downloads;
- centralize every duplicated line between independently published binaries;
- redesign `main` around async;
- introduce background update checks;
- add signature infrastructure beyond the existing SHA-256/candidate checks;
- optimize unrelated dependencies or binary sections.

## Completion notes

Completed 2026-09-17 on `main`. The migration landed split, as the plan
allows: `snp` uses eggfetch-core in-process; `snip-sync` retains its
`curl` adapter with an intentional-split comment at the top of
`snip-sync/src/update.rs`.

```text
Baseline commit: 302287cf4c9ada01dd664bcbd4710f3daff7e0b8
Eggfetch version/profile: eggfetch-core =0.1.5, default-features = false,
    features = ["http1", "tls-rustls", "tls-native-roots"];
    futures-util 0.3, default-features = false, features = ["alloc"]
    (StreamExt::next for the streamed binary path; "alloc" suffices)

snp before bytes: 6248720
snp after bytes: 6970568
snp delta bytes / %: +721848 / +11.55%
snp decision: KEEP with explicit report (over the 10% trigger, which the
    plan defines as an inspection trigger, not an automatic failure).
    Cause is the embedded rustls/TLS stack replacing the external curl
    process; no accidental features (verified via cargo tree: http2 in the
    graph comes from pre-existing tonic, json/tracing from axum/sqlx/tower).

snip-sync before bytes: 3833152
snip-sync after bytes: 5407400 (eggfetch trial build; reverted)
snip-sync delta bytes / %: +1574248 / +41.07%
snip-sync decision: REVERT to the existing curl adapter per the plan's
    >10% rule. Rebuilt after revert reproduces 3833152 bytes exactly.
    The split is recorded in snip-sync/src/update.rs module docs so a
    future cleanup does not "fix" it blindly.

Unexpected dependency/features: none. eggfetch-core enables only
    http1/tls-rustls/tls-native-roots (+hyper-rustls); no http2, http3,
    json, compression, cookies, proxy, multipart, tracing, or test-util.
Trust/profile decision: keep tls-native-roots. A WebPKI-only trial build
    of snp produced a byte-identical 6970568 binary on Linux aarch64, so
    there is no size incentive to give up native-root trust semantics.
Proxy compatibility note: unchanged. Neither the old curl adapter (no
    proxy flags were passed) nor the documented updater contract relied on
    proxy environment parsing; no eggfetch proxy feature was enabled.
Verification commands/results:
    cargo fmt --all -- --check — pass
    cargo clippy --workspace --all-targets -- -D warnings — pass
    cargo clippy -p snip-it --features test-support --all-targets/--tests — pass
    cargo clippy -p snip-sync --features test-helpers --all-targets/--tests — pass
    cargo clippy --workspace --all-targets --all-features — pass
    cargo test --workspace --lib --all-features — 1158 + 139 pass
    cargo test -p snip-it --features test-support --bin snp — 63 pass
        (13 new transport_tests + existing updater tests)
    cargo test -p snip-sync --features test-helpers --bin snip-sync — pass
        (pre-existing updater tests; eggfetch trial suite removed with revert)
    bash scripts/check.sh — pass (installer contract, fmt, clippy, unit,
        platform smoke, destination permissions, auto-sync closure +
        concurrency, multi-batch sync)
    bash scripts/ci/test-production-seams.sh — pass
    cargo test --workspace --all-features -- --test-threads=1 — all pass
    cargo build --release -p snip-it --bin snp — 6970568 (kept)
    cargo build --release -p snip-sync --bin snip-sync — 3833152 (restored)
```

Acceptance mapping: (1) `snp update` uses eggfetch-core 0.1.5 narrow
profile, no curl — yes (`tests/architecture.rs` pins it). (2) `snip-sync`
migrates only within the gate — it did not; curl retained and documented.
(3) retained curl path documented as measured tradeoff — yes, module docs +
AGENTS.md. (4) production HTTPS-only every hop — yes, `check_url_scheme`
at loop top plus `redirect_target` pre-request validation. (5) downgrade
rejected before plaintext request — yes, counter-proven in
`https_to_http_redirect_rejected_before_target_request`. (6) test HTTP
injection works — yes, `build_allow_http` + fixture suite. (7)(8) 1 MiB /
256 MiB bounds, streaming without buffering — yes, per-request
`max_decoded_body_size` + `bytes_stream` chunk loop. (9) no usable
partial on failure — yes, classify-before-create plus remove-on-error,
both asserted. (10) 404 distinguishable — yes, `FetchError::NotFound`
preserved. (11) Cargo fallback still 404-asset-only — yes, caller
boundary unchanged and covered by
`missing_asset_falls_back_while_checksum_404_hard_fails`. (12) Plan 004
policy unchanged — yes. (13) `snp` reuses `RUNTIME` — yes. (14)
`snip-sync` stays sync-`main` — yes (reverted; the trial runtime block
was removed with it). (15) no accidental features — yes. (16) no new
crate/framework — yes. (17) works with no curl on PATH — yes by
construction for `snp` (no curl invocation remains; architecture test
pins it). (18) measurements recorded — above. (19) verification green —
above. (20) release/install/platform behavior green — check.sh green.

Explicit non-transport deviation from the suggested order: the snip-sync
eggfetch trial (steps 7–11) was implemented and measured, then reverted
rather than kept, exactly as the size gate prescribes. The trial's
transport test suite was removed with the revert; the retained curl
adapter is covered by its pre-existing tests plus the architecture-doc
record. D15 has no PATH-mutating runtime test by design: emptying PATH
process-wide is racy against parallel subprocess-spawning unit tests, so
the no-curl property is pinned by source scan instead.
