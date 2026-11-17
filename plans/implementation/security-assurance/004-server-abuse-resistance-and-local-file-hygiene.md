# Security Assurance Milestone 004 — Server abuse resistance and local file hygiene

Status: closed

Repository baseline: `0f3bc98` (`chore/prune-comments-and-plans`)

Source roadmap:

- [`plans/subsystems/security-assurance-roadmap.md` § Milestone 004](../../subsystems/security-assurance-roadmap.md#milestone-004--server-abuse-resistance-and-local-file-hygiene)

Long-term requirements:

- [`plans/000-long-term-specification.md#4-crate-and-module-ownership`](../../000-long-term-specification.md#4-crate-and-module-ownership)
- [`plans/000-long-term-specification.md#8-transport-and-runtime-policy`](../../000-long-term-specification.md#8-transport-and-runtime-policy)
- [`plans/000-long-term-specification.md#10-verification-and-evidence-rules`](../../000-long-term-specification.md#10-verification-and-evidence-rules)
- [`plans/001-terminology-and-domain-model.md#7-server-terms`](../../001-terminology-and-domain-model.md#7-server-terms)

Applicable ADRs: none. The change stays inside the existing server lifecycle
and auth contracts; it adds budgets and file modes, not a protocol.

Primary class: invariant

---

## 1. Objective

Bound the work a caller can impose on the two server surfaces that check a
credential or create an account, and make server-owned data readable only by
the service account on Unix — without changing the gRPC wire contract, the
`snip-proto` version, or the sync semantics.

## 2. Why this milestone is ready

- `snip-sync` already owns a bounded, process-local rate limiter with eviction
  (`snip-sync/src/rate_limiter.rs`), used by every authenticated gRPC path. The
  change reuses that primitive instead of inventing one.
- The two unbounded surfaces are precisely identified: metrics basic auth
  (`snip-sync/src/http.rs:146-199`) and unauthenticated registration
  (`snip-sync/src/lib.rs:1050-1122`).
- The file-mode gap is a contrast inside one crate: the config file is
  chmodded (`snip-sync/src/bootstrap.rs:54-60`), the database is not
  (`snip-sync/src/db.rs:329-344`).
- No proto change is required; the limiter and file modes are process-local.

## 3. Current implementation evidence

### 3.1 Metrics authentication has no budget

`snip-sync/src/http.rs:146-199` decodes `Authorization: Basic`, compares with
`subtle::ConstantTimeEq` and a length check (`:169-175`), and on failure
increments `snip_sync_auth_failures_total` (`:181`). There is no limiter and no
lockout: guesses are bounded only by connection handling. The endpoint exists
only when both `metrics_username` and `metrics_password` are configured;
otherwise it returns `404` (`:150-156`).

The HTTP leaf receives only `RequestHead`
(`eggserve_server::service_fn_head`), which carries no peer address, so a
per-IP budget is not available in the current handler shape. A **global**
failure budget is therefore the correct bounded design; it cannot lock out a
legitimate scraper because successful requests do not consume it.

### 3.2 Registration is bounded only per peer

`snip-sync/src/lib.rs:1050-1122`: `Register` authenticates nothing, inserts one
`users` row plus one default `libraries` row per call (`db.create_user`,
`snip-sync/src/db.rs:487-522`), and is limited only by
`register:<peer>` (`lib.rs:1064-1082`). An attacker with many addresses grows
the database without a global ceiling.

### 3.3 The database ignores the private-file policy

`snip-sync/src/db.rs:329-344` creates the parent directory with
`create_dir_all` and lets SQLite create `snippets.db`, `-wal`, and `-shm`
under the process umask. Only the config file is restricted
(`snip-sync/src/bootstrap.rs:54-60`, mode `0o600`). Contents are ciphertext, but
metadata and Argon2id hashes are exposed to any local reader.

### 3.4 The shared limiter is per service, not per process

`AppState` is `{ config, metrics, db }` (`snip-sync/src/lib.rs:627-631`);
`SnipSyncService` owns the `Arc<RateLimiter>` (`:633-638`), created once in
`snip-sync/src/main.rs:169` and passed only to the gRPC service (`:226`). The
HTTP service receives `AppState` (`:218`, `:268`).

### 3.5 Tests that must not regress

`tests/snip_sync_lifetime.rs` locks the HTTP wire behavior (status codes,
`Allow`, `Vary`, security headers), and `snip-sync/src/lib.rs` tests lock the
auth status codes. A new `429` must be added deliberately, not accidentally:
the failure budget is configurable and test server configs must raise it
unless a test is specifically exercising the budget.

## 4. Invariants that must not regress

- `snip-proto` and the gRPC message contract are unchanged.
- Existing auth semantics are unchanged below the budget: wrong password is
  `401`, missing header is `401`, valid credentials are `200`.
- The metrics endpoint stays `404` when unconfigured; enabling it still
  requires both credentials.
- Successful metrics requests never consume the failure budget; an attacker
  cannot deny metrics to the operator by burning the budget.
- Registration still creates exactly one user + default library per successful
  call; a rejected call creates no rows.
- The per-peer registration budget remains; the new global budget is additive.
- File-mode changes are Unix-only and never change data content.

## 5. Scope

### In scope

- `snip-sync/src/http.rs`, `snip-sync/src/lib.rs`, `snip-sync/src/main.rs`,
  `snip-sync/src/db.rs`, `snip-sync/src/bootstrap.rs`.
- `snip-sync/config.toml` (two new documented keys).
- `snip-sync` version bump required by the public interface change.
- Focused tests and the serial lifetime target.

### Explicitly out of scope

- Per-IP metrics budgets (no peer address in the leaf; would require changing
  the service shape or the EggServe API).
- Persistent rate-limit state across restarts (the module documents
  process-local windows as intentional).
- Removing `/health` version output.
- Changing registration to require an invite or admin approval.
- Windows ACL enforcement (documented as out of scope).
- Changing gRPC auth, Argon2 parameters, or library scoping.

## 6. Required production changes

### Runtime and concurrency

Share one `Arc<RateLimiter>` between the gRPC service and the HTTP state:

- add `rate_limiter: Arc<RateLimiter>` to `AppState` and construct both from
  the instance created in `main.rs:169` (or pass it to `http::service`; prefer
  the `AppState` field so the service signature stays stable);
- keep distinct key namespaces: peer addresses for gRPC, `metrics-auth` and
  `register-global` for the new budgets. The limiter's `MAX_ENTRIES` bound and
  eviction already cover the added keys.

### Security and authorization

**Metrics failure budget.** In `metrics_response`, compute `valid` first. If
valid, serve. If invalid:

1. `record_auth_failure()` as today;
2. `allow("metrics-auth", config.metrics_auth_failures_per_minute, 60s)`;
3. if the budget is exhausted, return `429` with the existing plain-text body
   and `Retry-After: 60`; otherwise return `401` with the existing
   `WWW-Authenticate` challenge.

Document the 12th-and-later behavior and the fact that the budget is global.
A `429` must not carry `WWW-Authenticate` (that would invite more attempts).

**Registration global budget.** Before `create_user`:

- keep the existing `register:<peer>` check;
- add `allow("register-global", config.register_per_minute, 60s)`;
- on exhaustion return `Status::resource_exhausted` with a distinct message and
  record the rate-limit metric; no rows are written.

**Configuration.** Add to `[server.rate_limit]`:

```toml
register_per_minute = 10
metrics_auth_failures_per_minute = 10
```

Environment overrides `REGISTER_PER_MINUTE` and
`METRICS_AUTH_FAILURES_PER_MINUTE`; both parsed and validated `> 0` in
`Config::validate` alongside the existing checks (`snip-sync/src/lib.rs:595-620`
pattern). Zero is a hard configuration error, not "unlimited": a zero budget
would make registration impossible and is almost certainly a mistake.

### Storage and migrations

**Private server data (Unix).**

- When creating the database parent directory, create it `0o700` and, if it
  already exists, tighten it to `0o700` when possible; a failure to tighten an
  existing directory logs a warning naming the path and the observed mode
  (mirror the config-file permission warning in `snip-sync/src/lib.rs:349-370`).
- After the pool connects, `chmod 0o600` the database file and any existing
  `-wal`/`-shm` siblings; a failure logs a warning.
- Do not fail startup on a permission failure for a pre-existing path: a
  read-only or administrator-managed deployment may legitimately be unable to
  chmod. Newly created directories must be created with the mode at creation,
  not chmodded afterwards, to avoid a window.

### Frontend or operator surface

`snip-sync/config.toml` documents both new keys, their defaults, their
environment overrides, and the behavior change: a public deployment that
registers more than ten devices per minute must raise the key.

### Versioning

The public `AppState` and `Config` gain fields. `snip-sync` is pre-1.0;
take the semver-appropriate **minor** bump (0.1.8 → 0.2.0) and record it in
the closure evidence. `snip-proto` and `snip-it` are unchanged. No publish is
part of this milestone; the version bump only prepares the release.

## 7. Ordered work packages

### Work package A — Share the limiter

Intent: one bounded structure for both listeners.

Required changes: `AppState` field, `main.rs` wiring, gRPC service construction
reuses the same `Arc`.

Acceptance evidence: a unit test constructing both services with one limiter
observes a key written by one path and read by the other.

### Work package B — Metrics failure budget

Intent: cap password guesses without giving an attacker a denial switch.

Required changes: the failure-only budget, `429` + `Retry-After`, config key,
validation, and the no-lockout property test.

Acceptance evidence: 10 failures → `401`; 11th → `429`; a success after the
budget is exhausted still returns `200` (because successes do not consult it).

### Work package C — Registration global budget

Intent: bound account creation independently of the caller's address.

Required changes: the global check, config key, validation, typed status,
metric increment.

Acceptance evidence: with a low configured ceiling, N+1 registrations from
distinct peer addresses are rejected after the ceiling; no partial rows exist
for rejected calls.

### Work package D — Private database paths

Intent: make the data directory and files owner-only on Unix.

Required changes: the creation/chmod helper, calls from `Database::connect`
(and `snip-sync init` bootstrap if it creates the directory), warning path.

Acceptance evidence: `#[cfg(unix)]` tests assert `0o700` on the directory and
`0o600` on the database; a pre-existing `0o755` directory is tightened or
warned with its mode named.

## 8. Failure, cancellation, restart, and contention semantics

- Limiter windows are process-local and reset on restart; this is documented
  and unchanged. A restart must not leave a permanent lockout.
- Concurrent requests race on the async limiter mutex; the bounded map and
  eviction are unchanged, and the added keys cannot exceed `MAX_ENTRIES`.
- A rejected registration returns before `begin`; no transaction is opened, so
  no rollback path is added.
- A chmod failure is a warning, not a startup abort; a newly created directory
  that cannot be created `0o700` is a hard error (creation mode is part of the
  creation call).
- `Retry-After` is advisory; clients may retry earlier and receive `429`.

## 9. Compatibility and migration

- Two new optional config keys; absent keys take the defaults. Existing
  `config.toml` files keep loading.
- Behavior change: more than ten registrations per minute globally now fail;
  more than ten failed metrics authentications per minute now return `429`.
  Both are documented in `config.toml` and must appear in `CHANGELOG.md` at
  release time.
- No proto, storage-schema, or gRPC-status change for authenticated callers.
- `snip-sync` minor version bump; `snip-it` unaffected.

## 10. Required tests

### Focused unit tests

- `RateLimiter` key isolation: `metrics-auth` exhaustion does not affect
  `register-global` or a peer-address key.
- Config validation rejects zero for both new keys and accepts defaults when
  absent.

### Integration tests

- `tests/snip_sync_lifetime.rs`: unchanged behavior for the first N failures;
  new `429` case added with a low configured ceiling; `Retry-After` present;
  no `WWW-Authenticate` on `429`.
- Registration: a low global ceiling rejects the N+1th call with
  `resource_exhausted` and leaves the user count unchanged.

### Restart and recovery tests

- Restarting the service clears the budget (documented process-local
  behavior); no persisted state is created.

### Contention and cancellation tests

- Concurrent registration attempts at the ceiling produce at most the
  configured number of successful inserts.

### Security and negative tests

- Successful metrics auth after exhaustion still returns `200`.
- A wrong password never returns `200`, and the constant-time comparison is
  unchanged.
- Unix permission test for the directory and database; skip on non-Unix.

### Migration and compatibility tests

- An existing config without the new keys loads with defaults.
- Existing lifetime wire-parity cases still pass unchanged.

## 11. Required verification commands

```bash
cargo test -p snip-sync --lib
cargo test --test snip_sync_lifetime --features test-helpers -- --test-threads=1
cargo test --workspace --lib
bash scripts/check.sh
bash scripts/ci/test-production-seams.sh
```

`snip_sync_lifetime` and any concurrency target stay serial.

## 12. Documentation updates

- `snip-sync/config.toml` (two keys, defaults, overrides, behavior change).
- `SECURITY.md` § Server Deployment (registration and metrics budgets;
  private data directory) — coordinate with M003, which edits the same section.
- `CHANGELOG.md` at release time, not in this milestone.
- `architecture/server.md` if it enumerates config keys or auth surfaces.

## 13. Acceptance criteria

- Metrics authentication is bounded by a documented, configurable failure
  budget; a valid request is never blocked by that budget.
- Registration has a global budget distinct from the per-peer budget; rejected
  calls write no rows.
- On Unix, the server's database directory is `0o700` and its files `0o600`
  when created; an existing broader path is tightened or warned with its mode.
- New config keys are validated, documented, and optional.
- `snip-proto` is unchanged; `snip-sync` takes a minor bump.

## 14. Stop conditions

Stop and report rather than improvise when:

- sharing the limiter requires changing the EggServe service shape (then split
  the milestone rather than widening it);
- a permission change would break a documented multi-user deployment (record
  the conflict; do not silently drop the restriction);
- the lifetime parity tests cannot accommodate a `429` without weakening their
  assertions (stop for an explicit wire-contract decision);
- the registration ceiling needs product input on the default (propose 10/min
  and record the assumption rather than choosing silently).

## 15. Closure evidence required

- The limiter-sharing diff and the isolation test.
- Status-code transcripts for `401` → `429` and the success-after-exhaustion
  case.
- Registration ceiling transcript with row counts before/after.
- Unix mode assertions (`stat` output or test output).
- Config-validation evidence and the final defaults.
- The version bump and a statement that `snip-proto` is unchanged.

## 16. Handoff notes

- `snip-sync/src/lib.rs` is large; keep changes localized to the service
  methods and the config struct. HTTP policy stays in `http.rs`.
- The HTTP leaf has no peer address today; do not invent one from
  `X-Forwarded-For` without the trusted-proxy config used by gRPC.
- Tests construct `AppState`/`Config` literally; adding fields breaks every
  literal. Use `..Default::default()` where possible or update all constructors
  in the same commit — this is the main mechanical hazard.
- `tests/snip_sync_lifetime.rs` is wire-parity sensitive; read it before
  changing status codes, `Allow`, `Vary`, or headers.
- `cargo test --workspace --all-features -- --test-threads=1` is the full
  serial suite; do not run it concurrently with a `cargo build` that replaces
  `target/debug/snp`.
