# Security Assurance M004 — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/security-assurance/004-server-abuse-resistance-and-local-file-hygiene.md`

Source subsystem roadmap:

- `plans/subsystems/security-assurance-roadmap.md#milestone-004--server-abuse-resistance-and-local-file-hygiene`

Repository baseline reviewed: `0f3bc98` (`chore/prune-comments-and-plans`);
implementation is uncommitted.

Implementation commits or pull requests:

- Uncommitted working tree — `snip-sync/src/{lib,http,main,db,rate_limiter,test_helpers}.rs`,
  `snip-sync/config.toml`, `snip-sync/Cargo.toml` (0.1.8 → 0.2.0),
  `tests/snip_sync_lifetime.rs`, `tests/sync_multibatch.rs`, `Cargo.toml`,
  `Cargo.lock`, `architecture/server.md`, `SECURITY.md`.

Primary class: invariant. No proto change; process-local bounds and file modes.

## 1. Executive finding

M004 is complete. `/metrics` authentication now has a failure-only global
budget that returns `429` with `Retry-After` and no challenge, registration has
a global budget distinct from the per-peer budget, and the server's data
directory and files are owner-only on Unix. The gRPC service and the HTTP leaf
share one `Arc<RateLimiter>`, so the bounded entry map stays a single
process-wide limit.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| Metrics authentication bounded by a documented limit | `metrics_auth_failures_per_minute`, default 10, `snip-sync/config.toml` + `architecture/server.md` | pass | Global because the HTTP leaf's `RequestHead` carries no peer address |
| Valid credentials never blocked by that budget | integration test: valid scrape returns `200` after exhaustion | pass | Budget is consulted only on the failure path |
| `429` carries `Retry-After` and no `WWW-Authenticate` | integration test | pass | Challenging a throttled caller would invite the retry being bounded |
| First N failures keep returning the `401` challenge | integration test with budget 2 | pass | Wire parity for the ordinary case preserved |
| Registration has a global budget distinct from the per-peer budget | `register-global` key; unit test asserts key isolation | pass | Both budgets remain |
| Rejected registration writes no rows | unit test counts `users` before/after | pass | Rejection happens before `create_user`; no transaction is opened |
| Rejected registration is a typed resource-exhausted status | unit test asserts `Code::ResourceExhausted` and a distinguishable message | pass | |
| Data directory `0o700`, files `0o600` on Unix | `#[cfg(unix)]` tests | pass | Created with the mode at creation; pre-existing broad modes tightened |
| Broad pre-existing mode is tightened or warned with its mode | `preexisting_broad_directory_is_tightened` | pass | Failure path logs path + observed mode, does not abort startup |
| New config keys documented, defaulted, validated | `config.toml` comments; `config_defaults_new_rate_limit_keys_when_absent`; `config_validate_rejects_zero_*` | pass | Zero is a hard error, not "unlimited" |
| Existing configs still load | defaults test with only `requests_per_minute` present | pass | |
| `snip-proto` unchanged | `snip-proto/` has no diff | pass | |
| `snip-sync` takes a semver-appropriate bump | 0.1.8 → 0.2.0 in `snip-sync/Cargo.toml`, `Cargo.toml`, `Cargo.lock` | pass | Public `AppState`/`Config` gained fields |
| Windows behaviour stated as out of scope | `SECURITY.md` § Server Deployment and § Known Limitations | pass | ACL enforcement not implemented |

## 3. Production implementation evidence

- `Config` gains `register_per_minute` and `metrics_auth_failures_per_minute`,
  read from `[server.rate_limit]` or `REGISTER_PER_MINUTE` /
  `METRICS_AUTH_FAILURES_PER_MINUTE`, defaulted, and validated `> 0`.
- `AppState` gains `rate_limiter: Arc<RateLimiter>`; `main.rs` passes the same
  instance the gRPC service already owns.
- `http.rs::metrics_response` is now async: it computes validity first, records
  `auth_failures`, then consults a `metrics-auth` failure budget and returns
  `429` on exhaustion. `handle_request` adds `Retry-After: 60` for `429` and
  keeps `WWW-Authenticate` on `401` only.
- `lib.rs::register` keeps `register:<peer>` and adds `register-global` with its
  own message and metric increment.
- `db.rs` gains `prepare_data_directory` (creation mode `0o700`, tighten
  pre-existing, warn on failure) and `restrict_database_files` (`0o600` on the
  database and any existing `-wal`/`-shm`), the latter called after the schema
  writes so the siblings exist. Both are Unix-only with no-op counterparts.

## 4. Verification executed

```bash
cargo test -p snip-sync --lib                                  # 197 passed
cargo test --test snip_sync_lifetime -- --test-threads=1        # 7 passed, 2 ignored
cargo test --test sync_multibatch --features test-support -- --test-threads=1  # 6 passed
cargo check --workspace --all-targets                           # pass
```

Results: all pass. New unit tests: `register_global_budget_rejects_without_writing_rows`,
`server_budgets_do_not_consume_each_others_keys`,
`config_defaults_new_rate_limit_keys_when_absent`,
`config_reads_new_rate_limit_keys_from_file_and_env`,
`config_validate_rejects_zero_register_per_minute`,
`config_validate_rejects_zero_metrics_auth_failures_per_minute`,
`data_directory_is_created_owner_only`,
`database_file_is_restricted_to_owner_only`,
`preexisting_broad_directory_is_tightened`. New integration test:
`http_metrics_auth_failures_are_rate_limited`. The two ignored lifetime tests
are the pre-existing long-running ones and are unchanged.

## 5. Invariant review

- `snip-proto` and the gRPC message contract are unchanged.
- Below the budget, auth semantics are unchanged: missing header `401`, wrong
  password `401`, valid credentials `200`.
- Metrics stays `404` when unconfigured; enabling still requires both
  credentials.
- Successful scrapes never consume the failure budget.
- Registration still creates exactly one user plus one default library per
  successful call.
- The per-peer registration budget remains; the global budget is additive.
- File-mode changes are Unix-only and never change data content.

## 6. Failure and recovery review

- Limiter windows are process-local and reset on restart; a restart cannot
  leave a permanent lockout. Documented in `config.toml` and the deep dive.
- Concurrent requests race on the async limiter mutex; the bounded map and its
  eviction are unchanged and the new keys cannot exceed `MAX_ENTRIES`.
- A rejected registration returns before `begin`, so no rollback path exists.
- A chmod failure is a warning, not a startup abort; a newly created directory
  is created `0o700` as part of the creation call rather than chmodded after.
- `Retry-After` is advisory; an earlier retry still receives `429`.

## 7. Migration and compatibility review

Two new optional keys; absent keys take defaults, and an existing
`config.toml` keeps loading (proved by test). Behaviour change at release time:
more than ten registrations per minute globally now fail, and more than ten
failed metrics authentications per minute return `429`. Both are documented in
`config.toml` and must appear in `CHANGELOG.md` when the version is released —
this milestone deliberately does not edit the changelog. No storage schema
change. `snip-it` is unaffected by the `snip-sync` bump.

## 8. Security review

- The budget is failure-only, so it cannot be used to deny the operator their
  own scrape.
- The global registration budget is separate from the per-peer budget so a
  legitimate caller is not charged for an attacker's traffic on another address;
  it caps total account creation, which is the actual unbounded resource.
- The constant-time credential comparison is unchanged.
- `AAD`-free concerns do not apply here; no secret is introduced.
- Unix file modes protect metadata and Argon2id hashes, not only ciphertext;
  Windows ACLs remain the operator's responsibility and are documented as such.

## 9. Documentation and operations

- `snip-sync/config.toml` — both keys, defaults, environment overrides, the
  behaviour change, and the correction of the stale `persist = false` comment.
- `architecture/server.md` — config defaults, the `/metrics` budget, and the
  five-row bucket table including the two global keys and the shared limiter.
- `SECURITY.md` § Server Deployment and § Known Limitations (coordinated with
  M003, which owns those sections).
- `docs/SUPPLY_CHAIN_POLICY.md`, `architecture/proto.md`,
  `architecture/test-infrastructure.md` — version references updated to 0.2.0.

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| low | Per-IP metrics budgets are not implemented | A distributed guesser is bounded only globally | Would need a peer address in the EggServe HTTP leaf; explicitly deferred by the plan |
| low | Windows ACLs are not enforced | Server data may be readable by other local accounts on Windows | Operator responsibility, documented; a future milestone could add ACL handling |
| low | `register_per_minute = 10` is a proposed default | A busy legitimate deployment could hit it | Documented; operators can raise it; the plan authorised proposing 10 |

## 11. Roadmap disposition

Milestone closed. M005 may proceed; nothing depends on this milestone.

## 12. Registry updates

`plans/registry.md` and the roadmap § 12 table record M004 as `closed`.
