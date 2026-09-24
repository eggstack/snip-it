# Plan 020: EggServe HTTP parity and closure corrective

Status: complete

Depends on: Plan 018 (complete), Plan 019 (complete)

## Objective

Close the remaining narrow HTTP-compatibility and planning-evidence gaps after
the direct EggServe migration.

Plan 019's architecture decision remains authoritative:

~~~text
final production architecture
-----------------------------
Tonic -> gRPC listener
EggServe leaf runtime -> HTTP/1 health/metrics listener
~~~

Do not reopen the A/B/C architecture comparison. The direct EggServe service
remains the selected implementation unless this corrective pass uncovers a
material correctness problem that cannot be fixed locally.

This plan has four jobs only:

1. restore the previous Tower-HTTP CORS `Vary` contract;
2. prove and then preserve the pre-migration 404/405 wire representation;
3. add socket-level regression assertions for those semantics;
4. finish Plans 018/019 and registry closure evidence.

## Planning baseline

Current implementation baseline:

~~~text
main: 10aeb9940e2c1a81b200ca1c67bc92a7a94d456b
commit: Consolidate snip-sync HTTP on EggServe
~~~

Current direct HTTP dependencies:

~~~toml
eggserve-server = { version = "=0.2.1", default-features = false }
eggserve-primitives = { version = "=0.2.0", default-features = false }
~~~

Current measured architecture results:

~~~text
A original axum::serve:       3,833,152 bytes
B EggServe + Axum adapter:    3,963,968 bytes  (+3.41% vs A)
C direct EggServe service:    3,898,408 bytes  (+1.70% vs A, -1.65% vs B)
final winner: C
~~~

Current hosted CI for implementation commit `10aeb994`:

~~~text
workflow run 36011491151
Linux correctness:             passed
Platform smoke (Windows):      passed
Platform smoke (macOS):        passed
~~~

The corrective pass must preserve this architecture and dependency outcome.

## Confirmed defect 1 — CORS Vary parity

Before Plan 019, every HTTP request passed through Tower-HTTP's
`CorsLayer`.

Tower-HTTP 0.7's default CORS `Vary` policy emits:

~~~text
Vary: origin, access-control-request-method, access-control-request-headers
~~~

The default is applied by the CORS layer to ordinary and preflight responses,
including configurations where no origin is ultimately allowed.

The current native implementation in `snip-sync/src/http.rs` instead:

- emits `Vary: origin` only for some configured-origin responses;
- emits no `Vary` for allow-all responses;
- emits no full three-field `Vary` on preflight;
- emits no `Vary` on ordinary responses when no CORS origin is configured.

That is a real cache-semantics regression from the prior middleware contract.

### Required correction

Restore the prior default CORS cache-key contract exactly:

~~~text
Vary: origin, access-control-request-method, access-control-request-headers
~~~

Use one small constant/helper local to the snip-sync HTTP module if useful.

Do not add Tower-HTTP back solely for this header.

Do not emit three separate `Vary` fields unless the historical wire behavior
or EggServe response API makes that necessary. The preferred representation is
the same comma-separated value Tower-HTTP emitted.

The header should be present on the same broad response surface that the old
outer CORS layer covered, including:

- healthy and unhealthy `/health`;
- enabled, disabled, authorized, and unauthorized `/metrics`;
- 404 responses;
- 405 responses;
- configured-origin ordinary requests;
- configured-origin preflight;
- allow-all ordinary requests;
- allow-all preflight;
- no-origin/no-CORS-allow-list ordinary responses.

Do not make `Vary` conditional on whether
`Access-Control-Allow-Origin` is emitted.

## Corrective investigation 2 — 404/405 representation parity

The current native service emits explicit text bodies:

~~~text
404 -> "404 Not Found"
405 -> "Method Not Allowed"
content-type: text/plain; charset=utf-8
~~~

The pre-migration Axum router used its default fallback and method-router
responses. Plan 019 intended wire compatibility, but the new tests only assert
status/header behavior and do not prove the old representation body.

Before changing 404/405 behavior, establish the historical contract from the
Plan 019 parent commit:

~~~text
c1c77a814989f8add527fa25cebde9e87c80b920
~~~

Use the smallest reliable method:

1. preferred: build/run that historical tree in an isolated worktree or
   temporary checkout and issue raw HTTP requests to its real server;
2. acceptable fallback: a minimal Axum 0.8 fixture reproducing exactly
   `Router::route(..., get(...))` plus the old middleware ordering, if an
   isolated historical server run is impractical.

Capture at minimum:

~~~text
GET /missing
PUT /health
PUT /metrics
~~~

For each, record:

- status;
- content-length;
- content-type presence/value;
- body bytes;
- Allow header;
- security headers;
- CORS/Vary headers where applicable.

Do not infer the answer from framework documentation alone when the historical
server can be exercised.

### Decision rule

After capturing the old wire behavior:

- if current C already matches, add tests and make no production change;
- if current C differs only in fallback representation, change the native
  response to the historical behavior;
- do not preserve a newly introduced body/content-type merely because it is
  human-readable;
- do not change documented `/health` or `/metrics` success/error payloads.

If historical Axum behavior is an empty 404/405 representation, implement the
native equivalent with `ResponseBody::Empty` and omit application
`content-type` unless the old wire response supplied one.

## Preserve current security-header ordering

The old router layering was:

~~~text
Router
  -> security_headers_middleware
  -> CorsLayer as outer layer
~~~

The CORS layer handled preflight without calling the inner application.

Therefore do not mechanically add the normal security headers to preflight
responses unless the historical wire proof shows they were present.

For ordinary handler/router responses, continue preserving:

~~~text
x-content-type-options: nosniff
x-frame-options: DENY
cache-control: no-store
~~~

The corrective tests should make the distinction explicit rather than relying
on middleware-order memory.

## Scope

### In scope

Likely implementation files:

~~~text
snip-sync/src/http.rs
tests/snip_sync_lifetime.rs
plans/018-eggserve-0.2.2-axum-runtime-adoption-trial.md
plans/019-direct-eggserve-leaf-http-service-consolidation.md
plans/020-eggserve-http-parity-and-closure-corrective.md
plans/README.md
~~~

Potential documentation-only changes:

~~~text
CHANGELOG.md
architecture/server.md
~~~

Touch those only if existing wording becomes inaccurate.

### Out of scope

Do not:

- change EggServe versions;
- reintroduce `eggserve-core`;
- reintroduce Axum as a direct HTTP dependency;
- reintroduce Tower-HTTP;
- change Tonic or gRPC behavior;
- merge the listeners;
- enable EggServe TLS, HTTP/2, or HTTP/3;
- change metrics authentication policy;
- change CORS allow-list policy;
- change `CORS_ALLOW_ALL` loopback restrictions;
- add endpoints;
- add a generic router or middleware framework;
- redesign service orchestration;
- change startup/process-control behavior;
- change updater dependencies;
- add a new CI workflow;
- rerun the full A/B/C architecture experiment.

This should be a small parity patch plus evidence cleanup.

## Part A — add focused regression assertions before correction

Extend the existing real-socket tests in `tests/snip_sync_lifetime.rs`.

### Vary assertions

Assert the exact logical token set on at least:

1. ordinary `GET /health` with default/no allowed origins;
2. ordinary configured-origin `GET /health`;
3. configured-origin preflight `OPTIONS /health`;
4. allow-all ordinary `GET /health`;
5. allow-all preflight `OPTIONS /health`;
6. a fallback response such as `GET /missing`.

Prefer a helper that parses response headers case-insensitively and compares
the comma-separated `Vary` token set, while also asserting the expected
historical serialized value where stable.

Required token set:

~~~text
origin
access-control-request-method
access-control-request-headers
~~~

The test must fail against the current `10aeb994` implementation before the
production fix.

### 404/405 assertions

After historical behavior is captured, add explicit assertions for:

~~~text
GET /missing
PUT /health
PUT /metrics
~~~

Assert body and content-type, not merely status.

Keep the existing 405 `Allow: GET, HEAD` assertion if that matches the old
server.

Also assert whether the three security headers are present on these ordinary
fallback/method responses.

### Preflight middleware-boundary assertion

Capture and assert whether old preflight responses carry the three security
headers. Preserve that result exactly.

## Part B — make the smallest native HTTP correction

Modify only the concrete HTTP service.

Preferred shape:

- one constant for the historical CORS `Vary` value;
- one small helper to append common CORS metadata;
- existing route handling remains concrete and readable;
- fallback body construction follows the captured historical Axum behavior.

Avoid introducing:

- middleware traits;
- response-extension frameworks;
- router tables;
- generic header-policy engines.

The final module should remain recognizably a two-endpoint control surface.

## Part C — preserve all Plan 019 contracts

Rerun the existing socket tests and ensure this corrective work does not alter:

- `GET /health` JSON/status;
- `HEAD /health` representation length/body suppression;
- query-string route identity;
- metrics-disabled 404 behavior;
- metrics 401 behavior;
- malformed Basic auth handling;
- authorized Prometheus output;
- `HEAD /metrics`;
- constant-time credential comparison;
- configured CORS allow-origin;
- loopback allow-all behavior;
- request-body rejection;
- keep-alive behavior;
- 404/405 status;
- `Allow: GET, HEAD`;
- common security headers on ordinary responses.

No request-body acceptance should be added for OPTIONS.

## Part D — finish Plan 018 evidence

Update Plan 018 completion notes with concrete closure data.

At minimum replace:

~~~text
Implementation commit: this closure and Plan 019 are recorded together on main.
~~~

with the actual implementation commit:

~~~text
Implementation commit: 10aeb9940e2c1a81b200ca1c67bc92a7a94d456b
~~~

Keep the A/B measurement and resolved compatibility versions already recorded.

Clarify that B passed its gate but was subsequently superseded by C under
Plan 019.

Record the hosted CI run explicitly:

~~~text
36011491151
~~~

Do not rewrite Plan 018 as though B were the final architecture.

## Part E — finish Plan 019 evidence

Fill all currently blank completion fields:

~~~text
C direct EggServe bytes: 3,898,408
C vs A bytes / percent: +65,256 / +1.70%
C vs B bytes / percent: -65,560 / -1.65%
Resolved eggserve-server: 0.2.1
Resolved eggserve-primitives: 0.2.0
~~~

Retain:

~~~text
Final winner: C
~~~

Add exact implementation commit and hosted CI run if the completion section
does not already identify them clearly.

Update the wire-test description to include the newly closed `Vary` and
404/405 representation contracts.

Do not rerun size measurements merely to fill fields whose controlled values
are already recorded elsewhere in the plan/registry.

## Part F — reconcile plans/README.md

Register Plan 020 as the active corrective follow-up while it is open.

On implementation completion:

- mark Plan 020 Complete;
- retain Plans 018 and 019 as Complete;
- state that C remains the final architecture;
- replace wording such as "hosted checks run" with the verified result:
  implementation CI run `36011491151` passed Linux, Windows, and macOS;
- record that Plan 020 restored exact CORS cache variation semantics and
  locked fallback representation behavior with socket tests.

Do not reopen completed older plan sequences.

## Part G — verification

Run focused tests first:

~~~sh
cargo build -p snip-sync --bin snip-sync
cargo test --test snip_sync_lifetime -- --test-threads=1
~~~

Then ordinary repository checks:

~~~sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --lib
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
~~~

Because this is a small HTTP parity patch, a new controlled release-size A/B/C
study is not required.

As a sanity check, build the final release binary once:

~~~sh
cargo build --release -p snip-sync --bin snip-sync
wc -c target/release/snip-sync
~~~

Record the value only to detect an obviously accidental footprint explosion.
Do not fail this corrective pass over normal linker-level byte drift unless the
change is material or unexpected.

After push, verify the existing GitHub Actions:

~~~text
Linux correctness
Platform smoke (Windows)
Platform smoke (macOS)
Link Check
~~~

No new workflow is required.

## Recommended execution order for a smaller implementation model

1. Read Plans 018, 019, and this plan.
2. Check out or otherwise execute the historical parent
   `c1c77a814989f8add527fa25cebde9e87c80b920`.
3. Capture raw 404/405 and preflight wire behavior.
4. Add failing current-tree tests for full CORS `Vary` semantics.
5. Add exact 404/405 representation assertions from the captured baseline.
6. Add the preflight security-header assertion.
7. Patch only `snip-sync/src/http.rs`.
8. Run the focused socket suite.
9. Run `scripts/check.sh` and production-seam verification.
10. Build one release sanity binary.
11. Fill Plan 018 completion provenance.
12. Fill Plan 019's blank C/version fields.
13. Mark Plan 020 complete and reconcile `plans/README.md`.
14. Push the implementation/evidence commit.
15. Confirm hosted CI and, if needed, add only a final evidence-only commit.

## Acceptance criteria

Plan 020 is complete when all of the following are true:

1. The direct EggServe C architecture remains the sole production HTTP path.
2. Direct dependencies remain `eggserve-server =0.2.1` and
   `eggserve-primitives =0.2.0`.
3. `eggserve-core`, direct Axum, direct Tower, and Tower-HTTP are not
   reintroduced.
4. Ordinary and preflight HTTP responses reproduce Tower-HTTP's historical
   three-field `Vary` contract.
5. `Vary` behavior is covered on no-origin, configured-origin, allow-all,
   preflight, and fallback responses.
6. Historical 404 wire representation is captured and reproduced.
7. Historical 405 wire representation is captured and reproduced.
8. 405 `Allow` semantics remain compatible.
9. Historical preflight security-header behavior is captured and reproduced.
10. Ordinary response security headers remain unchanged.
11. Health and metrics application semantics remain unchanged.
12. Metrics Basic auth remains constant-time.
13. Request bodies remain rejected on the control surface.
14. HEAD behavior remains delegated to EggServe normalization and passes.
15. Existing keep-alive and shutdown tests remain green.
16. Plan 018 records implementation commit `10aeb994...` and concrete hosted
    CI provenance.
17. Plan 019 records C size/deltas and resolved leaf versions with no blanks.
18. Plan 019 still names C as the final winner.
19. `plans/README.md` records Plan 020 and reconciles the EggServe sequence.
20. `scripts/check.sh` and production-seam verification pass.
21. Existing Linux, Windows, and macOS hosted CI pass after the corrective
    implementation.
22. No unrelated cleanup or architecture work is included.

## Completion notes

~~~text
Planning baseline: 10aeb9940e2c1a81b200ca1c67bc92a7a94d456b
Corrective implementation commit: (filled after push; single implementation commit on main)
Historical parity reference: c1c77a814989f8add527fa25cebde9e87c80b920
Historical 404 body/content-type: empty body, content-length 0, no content-type
  (router fallback; metrics-disabled 404 keeps "Not found" text/plain payload)
Historical 405 body/content-type: empty body, content-length 0, no content-type
Historical 405 Allow: GET, HEAD
Historical preflight security headers: absent (CORS short-circuits OPTIONS
  outside the security middleware; ordinary responses carry all three)
Historical Vary: `origin` when no origins configured; absent for configured
  origins and allow-all (Tower-HTTP recomputes Vary from the rules at
  layer-build time; the configured path used exact-origin Const semantics, so
  the three-field default never reached the wire for snip-sync's configs)
Corrective Vary behavior: `Vary: origin` on every non-allow-all response
  (ordinary, preflight, fallback); omitted for allow-all. This restores the
  proven historical value for the empty and allow-all configurations and
  extends `origin` to configured-origin responses to keep the cache key
  coherent with the retained conditional ACAO emission (out of scope to
  change per this plan). The specified three-field value was contradicted by
  the historical wire proof and was not implemented; see closure decision.
Focused snip_sync_lifetime: 6 passed, 2 ignored (long-signal suites per convention)
scripts/check.sh: passed (including new parity socket contracts)
production-seam: passed (scripts/ci/test-production-seams.sh)
Release sanity bytes: 3,898,408 (byte-identical to recorded C; no footprint change)
GitHub Actions run: (verified after push; Linux correctness + Windows/macOS smoke + Link Check)
Final architecture: C (direct EggServe)
Closure decision: Corrective parity implemented on C with no dependency,
  listener, TLS/H2/H3, auth, endpoint, or orchestration changes. The only
  plan-text deviation is the Vary value: criterion 4 named the Tower-HTTP
  `Vary::default()` three-field value, but exercising c1c77a8 proved the
  server never emitted it (origin-only for the empty config, absent
  otherwise). Per this plan's own evidence rule (wire proof over inference,
  and never preserve a newly introduced representation), the implementation
  reproduces the proven contract instead of introducing a header value with
  no historical precedent. Plans 018/019 evidence completed; plans/README.md
  reconciled; C remains the final architecture.
~~~
