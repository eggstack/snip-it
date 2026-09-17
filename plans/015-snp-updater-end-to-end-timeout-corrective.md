# Plan 015: snp updater end-to-end timeout corrective

Status: ready for implementation

Depends on: Plan 014 (complete)

## Objective

Correct the narrow timeout regression introduced by the Plan 014 `snp` updater transport migration.

The current eggfetch-backed updater correctly applies connect/read timeouts and a 60-second Tokio timeout around request/redirect traversal, but the final response body is consumed **after** that outer timeout has completed. As a result, a peer that continuously sends body data just before the per-read inactivity timeout can keep a metadata or binary transfer alive beyond the intended 60-second total operation budget.

Restore the Plan 014 / former `curl --max-time 60` contract:

```text
one logical fetch
    |
    +-- initial URL validation
    +-- request / connect
    +-- bounded redirect traversal
    +-- final HTTP status classification
    +-- final metadata body OR streamed binary body
    `-- complete or fail within one overall timeout budget
```

This is a focused correctness pass. Do not redesign the updater, change dependency choices, alter the Plan 014 `snp` / `snip-sync` transport split, or broaden the HTTP client abstraction.

## Current bug

In `src/update.rs`, `safe_get(...)` currently creates this outer timeout:

```rust
match tokio::time::timeout(overall_timeout, fetch).await {
    ...
}
```

The `fetch` future covers URL validation, redirect traversal, and `send().await`, then returns an `eggfetch_core::Response`.

The callers subsequently consume the final body outside that deadline:

```text
fetch_bytes_with
    safe_get(...).await        <- total timeout ends here
    response.bytes().await     <- outside total timeout

fetch_file_with
    safe_get(...).await        <- total timeout ends here
    response.bytes_stream()
    stream_binary_to_file(...) <- outside total timeout
```

The configured eggfetch read timeout still limits a **single period of read inactivity**, but it is not equivalent to a wall-clock cap on the full transfer. A slow peer can therefore keep sending small chunks often enough to avoid the read timeout while extending the operation indefinitely relative to the intended 60-second maximum.

This contradicts the existing source comment and Plan 014 acceptance contract that the 60-second bound includes redirect traversal **and final body consumption**.

## Governing constraints

1. Preserve all Plan 014 updater behavior except the incorrect placement of the overall timeout.
2. Keep `eggfetch-core =0.1.5` and the existing narrow feature profile unchanged.
3. Keep connect timeout at 10 seconds and read inactivity timeout at 60 seconds.
4. Keep the overall production fetch budget at 60 seconds.
5. Keep automatic redirects disabled and preserve HTTPS-only validation for every production redirect hop.
6. Keep metadata bounded to 1 MiB and binary downloads bounded to 256 MiB.
7. Keep binary bodies streamed to disk; do not buffer release binaries in memory.
8. Keep `FetchError::NotFound` and the 404-only Cargo fallback policy unchanged.
9. Keep `snip-sync` on its existing curl transport. Plan 015 is only for the root `snp` updater.
10. Add no dependencies, mock HTTP frameworks, runtime architecture, retry layer, proxy handling, or generic timeout abstraction.
11. Do not change release tags, asset naming, checksums, candidate validation, Homebrew/Cargo behavior, or executable replacement.
12. Keep the fix readable enough for a small-model handoff: a small internal helper is acceptable; a generalized transport framework is not.

## Part A — move the overall deadline to the complete logical fetch

### A1. Remove the wall-clock timeout boundary from `safe_get`

`safe_get` should remain responsible for:

- initial and per-hop scheme validation;
- bounded manual redirect traversal;
- request construction and body-size limits;
- returning the final non-redirect `Response`;
- mapping request/transport errors.

It should **not** define the total wall-clock lifetime of the operation if it returns before the response body has been consumed.

The simplest acceptable shape is to remove `overall_timeout` from `safe_get` and let its caller own the complete deadline.

Do not remove the eggfetch connect/read timeout configuration from `update_http_client()` / `http_client_with()`. Those remain useful phase/inactivity bounds inside the larger wall-clock budget.

### A2. Wrap `fetch_bytes_with` end to end

Build one inner async operation containing all of:

```text
safe_get
final status classification
response.bytes().await
body conversion / transport-error mapping
```

Then wrap that complete operation in:

```rust
tokio::time::timeout(overall_timeout, operation)
```

On timeout, return the existing hard-failure class with a message equivalent to:

```text
update request timed out after N seconds
```

Do not map timeouts to `NotFound` or allow Cargo fallback.

The timer must begin before the first request/redirect work and remain authoritative until the final metadata/checksum body is fully consumed.

### A3. Wrap `fetch_file_with` end to end

Likewise, the overall timeout must include:

```text
safe_get
final status classification
staging-file creation
bytes_stream acquisition
all streamed body chunks
all blocking write_all calls in the stream loop
```

A transfer that continues making progress but exceeds the total wall-clock budget must fail.

The timeout remains a transport hard failure and must never become `MissingAsset` / Cargo fallback.

## Part B — guarantee staging-file cleanup on timeout cancellation

Moving `tokio::time::timeout` around the whole streamed operation introduces an important cancellation case: when the deadline fires, Tokio drops the inner future while a staging file may already exist and contain partial bytes.

Preserve Plan 014's invariant:

> failed, oversized, transport-error, write-error, or timed-out downloads do not leave a usable partial candidate.

Implement this without introducing a cleanup framework.

A straightforward acceptable shape is:

```text
result = timeout(overall_timeout, async {
    ... classify status ...
    ... create file ...
    ... stream body into file ...
}).await

match result:
    timeout -> best-effort remove staging path; return FetchError::Failed
    inner error -> existing inner cleanup / best-effort remove as needed
    success -> keep completed file
```

It is also acceptable to centralize the best-effort `remove_file(path)` into one tiny local helper if that makes all file-failure paths clearer.

Do not use an RAII/temp-file dependency or add async filesystem features for this correction.

Status classification should still happen before file creation where practical so 404/401/403/5xx responses do not create or truncate the staging path.

## Part C — add deterministic slow-body regression coverage

The existing fixture can delay an entire response before sending it. That proves a header/request timeout, but it does not reproduce this bug because no body progress occurs before the deadline.

Extend the existing std-only loopback fixture minimally so a response can:

1. send headers immediately;
2. advertise the correct `Content-Length`;
3. send the body in multiple small chunks;
4. pause between chunks for a configurable short duration.

Avoid adding chunked-transfer parsing, TLS fixtures, mock-server crates, or a general-purpose HTTP test framework. The fixture only needs enough behavior to prove the updater's wall-clock contract.

### C1. Metadata slow-drip test

Add a test equivalent to:

```text
server sends response headers immediately
server sends small metadata chunks every ~100 ms
client read timeout is comfortably above 100 ms
client overall timeout is ~250-350 ms
full body would require materially longer than the overall timeout
```

Assert:

- the operation returns `FetchError::Failed`;
- the failure is an overall timeout rather than `NotFound`;
- the test proves continuing body progress does not reset the total budget.

The exact test timings may be adjusted to avoid CI flakiness, but preserve a wide margin between inter-chunk delay, read timeout, and overall timeout. Prefer hundreds of milliseconds rather than tiny scheduler-sensitive intervals.

### C2. Streamed binary slow-drip test

Add the same regression proof for `fetch_file_with` because the binary path has the additional partial-file cleanup requirement.

Assert:

- headers arrive and at least one body chunk is written before the deadline;
- inter-chunk progress remains inside the read inactivity timeout;
- the overall deadline still aborts the transfer;
- `FetchError::Failed` is returned;
- the staging file does not exist after the call returns.

If directly proving that at least one chunk was written makes the fixture disproportionately complicated, expose a tiny fixture-side counter/flag showing at least one body chunk was sent before timeout. Do not leave the test as only an elapsed-time assertion.

### C3. Preserve existing timeout tests

Keep the existing delayed-response/read-timeout coverage unless it becomes exactly redundant after the fixture change. The desired test matrix is:

```text
request/header stalls -> hard failure
read inactivity       -> hard failure
body keeps progressing but total time expires -> hard failure
binary total timeout -> partial staging file removed
```

No production test should sleep for 60 seconds; inject small test-specific durations through the existing helper parameters.

## Part D — source comments and Plan 014 contract consistency

After the fix, update the nearby timeout comments in `src/update.rs` only as necessary so they describe the actual ownership correctly.

The important invariant should read unambiguously as:

```text
the caller applies one total wall-clock timeout around redirect traversal and complete final-body consumption
```

Do not rewrite Plan 014's historical completion notes. Plan 015 is the corrective record for the discovered placement bug.

No user-facing README/changelog entry is required for this internal unreleased correction unless implementation lands after a release containing the buggy eggfetch updater. If that happens, add a concise bug-fix note under the appropriate release section without expanding scope.

## Expected file touch set

Primary:

```text
src/update.rs
plans/015-snp-updater-end-to-end-timeout-corrective.md
plans/README.md
```

Possible only if required by an already-released version boundary:

```text
CHANGELOG.md
```

No manifest, lockfile, `snip-sync`, installer, workflow, or dependency changes are expected.

## Suggested implementation order for a smaller model

1. Read the Plan 014 completion notes and current `src/update.rs` transport helpers.
2. Refactor `safe_get` so it no longer owns a timeout that ends before body consumption.
3. Wrap the complete `fetch_bytes_with` operation in the injected overall timeout.
4. Wrap the complete `fetch_file_with` operation in the injected overall timeout.
5. Ensure timeout cancellation best-effort removes the binary staging path.
6. Extend the existing loopback fixture with minimal slow-body/chunk support.
7. Add the metadata slow-drip total-timeout regression test.
8. Add the binary slow-drip total-timeout + cleanup regression test.
9. Run the focused updater tests first.
10. Run full workspace formatting, clippy, tests, and repository check scripts.
11. Mark this plan complete and update `plans/README.md` in the same implementation commit, recording the exact tests and verification run.

Do not touch `snip-sync/src/update.rs` during this sequence.

## Required verification

At minimum run:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p snip-it --features test-support --bin snp
cargo test --workspace --all-features -- --test-threads=1
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

Also run the ordinary GitHub Actions CI through the pushed implementation commit. Do not weaken or skip platform jobs to land this correction.

Because this plan adds no dependency or release-profile changes, no new binary-size gate is required. A material binary-size change would be unexpected and should be investigated rather than accepted as part of this plan.

## Acceptance criteria

Plan 015 is complete when all applicable statements are true:

1. One injected overall timeout covers URL validation, redirect traversal, final status handling, and complete final-body consumption for metadata/checksum fetches.
2. One injected overall timeout covers URL validation, redirect traversal, final status handling, staging-file creation, complete streamed binary consumption, and writes for binary fetches.
3. A body that continually makes progress inside the read inactivity timeout still fails once the overall wall-clock budget expires.
4. Overall timeout remains `FetchError::Failed`, never `FetchError::NotFound`.
5. A timed-out streamed binary leaves no staging file / usable partial candidate.
6. Existing connect/read timeout behavior remains intact.
7. Existing 1 MiB metadata and 256 MiB binary bounds remain intact.
8. Existing HTTPS-only production redirect validation remains intact.
9. Existing 404-only Cargo fallback behavior remains intact.
10. Existing checksum, candidate identity/version, Homebrew/Cargo, replacement, and lifecycle behavior remains unchanged.
11. Deterministic metadata and binary slow-drip tests reproduce the pre-fix bug shape and pass with the correction.
12. No new dependency, transport abstraction, async filesystem feature, retry policy, proxy behavior, or `snip-sync` change is introduced.
13. Full required verification and repository CI are green.
14. Plan status and `plans/README.md` are updated with completion evidence in the implementation commit.

## Explicit non-goals

Do not use this corrective pass to:

- migrate `snip-sync` to eggfetch;
- reconsider the Plan 014 binary-size decision;
- upgrade or relax the eggfetch version pin;
- add HTTP proxy environment support;
- add automatic retries;
- add HTTP/2 or HTTP/3;
- change TLS trust roots;
- change release/checksum policy;
- redesign `FetchError`;
- replace blocking staging-file writes with async I/O;
- add a shared updater crate or generic network layer;
- redesign the global Tokio runtime;
- change installers, release workflows, or service lifecycle behavior;
- perform unrelated updater cleanup.

## Completion notes

To be filled by the implementation pass.

Record at minimum:

```text
Implementation commit:
Timeout ownership after fix:
Metadata slow-drip regression test:
Binary slow-drip/partial-cleanup regression test:
Focused updater test result:
Workspace/all-features result:
scripts/check.sh result:
production-seams result:
GitHub Actions result:
Unexpected scope/dependency changes: none / explain
```
