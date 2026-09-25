# Server (snip-sync)

[← Back to Overview](overview.md)

## Overview

**Directory**: `snip-sync/` (`snip-sync` binary + `snip_sync` library).

A standalone sync server: Tonic gRPC for snippet/library sync and
EggServe HTTP/1 for health/metrics. The two transports are separate
listeners owned by different runtimes; all business logic lives in one
`SnipSyncService` (defined in `lib.rs`, not `main.rs`) backed by SQLite,
an in-memory rate limiter, Prometheus metrics, and a premade-library
manager.

```
┌─────────────────────────────────────────────────────────────┐
│                         snip-sync                            │
│                                                              │
│   :50051 (default)                :50050 (default)            │
│   ┌──────────────────┐            ┌──────────────────┐       │
│   │ Tonic gRPC       │            │ EggServe HTTP/1  │       │
│   │ TcpListener →    │            │ pre-bound        │       │
│   │ serve_with_      │            │ TcpListener →    │       │
│   │ incoming_shutdown│            │ ServerControl /  │       │
│   └────────┬─────────┘            │ ServerCompletion │       │
│            │                      └────────┬─────────┘       │
│            │        ┌──────────────────────┘                 │
│            ▼        ▼                                        │
│   ┌─────────────────────────────────────────┐               │
│   │            SnipSyncService (`lib.rs`)    │               │
│   │  ┌──────────┐  ┌──────────────────┐     │               │
│   │  │ Database │  │ RateLimiter      │     │               │
│   │  │ (sqlx /  │  │ (in-memory,     │     │               │
│   │  │ SQLite)  │  │ 120/min default)│     │               │
│   │  └──────────┘  └──────────────────┘     │               │
│   │  ┌──────────┐  ┌──────────────────┐     │               │
│   │  │ Metrics  │  │ PremadeManager   │     │               │
│   │  │(Prometheus│  │ (TOML dir scan)  │     │               │
│   │  │ registry)│  │                  │     │               │
│   │  └──────────┘  └──────────────────┘     │               │
│   └─────────────────────────────────────────┘               │
│                                                              │
│   broadcast shutdown (1) ──► both services ──► drain under   │
│   `request_timeout_secs` ──► ServiceShutdownOutcome          │
└─────────────────────────────────────────────────────────────┘
```

Dependencies (`snip-sync/Cargo.toml`): `tonic 0.14` (+ `prost 0.14`,
`snip-proto` stubs), `eggserve-server =0.2.1` +
`eggserve-primitives =0.2.0` with default features off, `sqlx 0.9`
(`sqlite`, `runtime-tokio`, `chrono`), `argon2 0.6`, `sha2`, `subtle`,
`prometheus 0.14`, `tokio` (`macros`, `rt-multi-thread`, `signal`),
`tokio-stream`, `clap 4.5`, `dirs 7.0`, `semver`, `libc` /
`windows-sys`. No Axum, no Tower-HTTP, no generic router/middleware.

## Module map (19 files)

gRPC handler code lives in `lib.rs`. `main.rs` is CLI dispatch plus
`serve()` / `serve_inner()` wiring only. `update.rs` is binary-private
(`mod update` in `main.rs`), not a library module.

| File (`snip-sync/src/`) | Responsibility |
|--------------------------|----------------|
| `lib.rs` | `SnipSyncService` + all 11 gRPC handlers, `Config` load/validate, `extract_api_key`, `validate_snippet`, metrics helpers |
| `main.rs` | CLI dispatch, `serve()` gate (TLS policy, layout, lock), `serve_inner()` listener bind + service spawn, `stop`/`restart`/`croncheck`/`paths` |
| `http.rs` | Concrete two-route EggServe leaf service (`/health`, `/metrics`), `CorsPolicy`, Basic-auth check, security headers |
| `orchestration.rs` | Production `run_eggserve_services_until_shutdown`, `ServiceShutdownOutcome`, `ServiceResult`, `ensure_clean_requested_shutdown` |
| `startup.rs` | Fail-closed transport policy, supervisor records (systemd/launchd/cron/Task Scheduler), health probe, managed restart, update lifecycle |
| `update.rs` | Binary-first self-update via external `curl` (Plan 017 size gate, see below); checksums, identity check, Windows self-replace helper |
| `db.rs` | SQLite schema, Argon2id auth, snippet/library CRUD, plaintext migration |
| `bootstrap.rs` | `ensure_layout()` dirs + `ensure_config_file()` from `config.toml` template (`create_new`, `0o600` on Unix) |
| `server_lock.rs` | Kernel-backed singleton (`flock` / `LockFileEx`), diagnostic identity metadata, `Busy` vs `Io` vs `UnsupportedPlatform` |
| `process.rs` | Legacy PID-file fallback parser, `is_running` / start-token / name checks, signaled stop, croncheck file lock |
| `paths.rs` | `config_dir` / `config_path` (`CONFIG_PATH`), `data_dir`, `state_dir` (`SNIP_SYNC_STATE_DIR`), `cert_dir`, `pid_path`, `db_path`, `premade_dir` |
| `cli.rs` | `serve`, `init`, `cert`, `edit`, `stop`, `restart`, `startup`, `update`, `__self-replace`, `croncheck`, `paths`, `completions`, `version` |
| `cert.rs` | Dev CA via external `openssl req -x509` (4096-bit, `CN=localhost`, SAN `localhost`/`127.0.0.1`); proxy-consumed, server still plaintext |
| `editor.rs` | `$EDITOR` resolution (absolute / CWD-relative / `PATH`) with symlink-escape refusal; `open_in_editor()` for `edit` |
| `rate_limiter.rs` | Bounded process-local sliding windows (`MAX_ENTRIES = 100_000`), epoch-second windows, reset on restart |
| `metrics.rs` | Prometheus registry + 6 instruments, `Metrics::fallback()` isolated registry on init failure |
| `premade.rs` | `.toml` directory scan, invalid-escape repair, snippet-count metadata, traversal-safe `get()` |
| `test_helpers.rs` | `test-helpers`-gated in-process service (`sqlite::memory:`, fallback metrics, fresh auth capture) |
| `test_observer.rs` | `test-helpers`-gated sanitized telemetry (length/hash/sentinel only, `MAX_OBSERVER_RECORDS = 256`) |

## Database

**File**: `snip-sync/src/db.rs`. SQLite via `sqlx` (`PRAGMA foreign_keys=ON`);
file path in production, `sqlite::memory:` in tests. Schema is created
with `CREATE TABLE IF NOT EXISTS` plus explicit indexes.

### Tables

| Table | Columns / constraints |
|-------|-----------------------|
| `users` | `id` (PK), `api_key` (UNIQUE, Argon2id PHC hash), `api_key_prefix` (nullable, indexed), `created_at`, `updated_at` |
| `libraries` | `id` (PK), `user_id` (FK → `users`), `name`, `created_at`, `deleted_at` (soft delete), `UNIQUE(user_id, name)` |
| `snippets` | `id` (PK), `user_id` (FK), `library_id` (FK), `description`, `command`, `tags` (JSON text, default `'[]'`), `created_at`, `updated_at`, `device_id`, `deleted` (0/1), `encrypted` (0/1) |

Indexes: `idx_users_api_key_prefix`, `idx_snippets_user`,
`idx_snippets_library`, `idx_snippets_updated`,
`idx_snippets_user_library_updated (user_id, library_id, updated_at,
deleted)`, `idx_libraries_user`.

### API-key hashing and lookup

- **Hash**: Argon2id (`Algorithm::Argon2id`, `V0x13`), 16 MiB memory
  (`1 << 14` KiB), 3 iterations, parallelism 4, fresh 16-byte salt per
  key; PHC string stored in `users.api_key`.
- **Indexed lookup**: `api_key_prefix` = first 8 chars of
  `base64(SHA-256(api_key))` (`STANDARD_NO_PAD`). Auth queries
  `WHERE api_key_prefix = ?` then Argon2-verifies only that bucket.
- **Legacy backfill**: `migrate_plaintext_api_keys()` runs at startup
  before listeners spawn; failure halts startup so a half-migrated DB
  cannot lock users out. The old NULL-prefix O(N)-verify fallback was
  deliberately removed (DoS amplifier); `derive_prefix_from_hash()` only
  backfills index metadata, it cannot recover keys from hashes.

### Key operations

| Operation | Notes |
|-----------|-------|
| `create_user(api_key)` | Hash + prefix, insert user + `default` library in one transaction; returns user id |
| `get_user_by_api_key(key)` | Prefix-bucket select, Argon2 verify, `Ok(None)` on miss |
| `upsert_snippet` / `upsert_snippet_in_tx` | Insert-or-update scoped to `(user_id, library_id)`; push/sync batch in one transaction with rollback on failure |
| `get_snippets(user, lib, since, limit, offset, sync_filter)` | Paginated delta read; total count + `has_more` computed by caller |
| `get_latest_timestamp(user, lib)` | Sync watermark; held back while `has_more` to avoid skipping pages |
| `get_default_library(user)` | Resolves the per-user `default` library id |
| `create_library(user, name)` | Validates `alphanumeric/-/_`, length 1–64; `Conflict` on duplicate |
| `list_libraries(user, limit, offset)` | Paginated with snippet counts |
| `delete_library(user, lib)` | Soft delete (`deleted_at`); default library refused at handler layer |
| `verify_library_ownership(user, lib)` | Authorization gate for every scoped RPC |
| `ping()` / `pool()` | Health probe (`SELECT 1`) and pool access for transactions |

## gRPC service

**File**: `snip-sync/src/lib.rs` (`SnipSyncService: SnippetSync`). Eleven RPCs:

| RPC | Description |
|-----|-------------|
| `Health` | `db.ping()` → `healthy` + crate version; unauthenticated |
| `Register` | New UUID API key + `create_user`; rate-limited by peer IP (see below) |
| `GetSnippets` | Auth + ownership check, default page 100, delta read |
| `PushSnippets` | Auth + ownership check, transactional batch upsert, accepted/rejected counts |
| `Sync` | Bidirectional: validate + upsert locals in tx, return server delta + watermark + `skipped_ids` |
| `CreateLibrary` | Validated create; `already_exists` / `invalid_argument` mapping |
| `ListLibraries` | Default page 50, ownership-scoped |
| `DeleteLibrary` | Refuses `""`, `"default"`, and the resolved default id |
| `ListPremadeLibraries` | Authenticated directory listing |
| `GetPremadeLibrary` | Authenticated fetch with strict filename allowlist |
| `SearchPremadeLibraries` | Authenticated substring search; empty query rejected |

### Server-side validation

- **Field limits** (configurable, defaults): `max_command_length` 1024,
  `max_description_length` 1024, `max_tags` 50, `max_tag_length` 100,
  `max_id_length` 128, `max_device_id_length` 128,
  `max_api_key_length` 512. Plaintext snippets require a non-blank
  command; encrypted snippets require a non-blank payload but skip the
  length cap (ciphertext).
- **Batch/page bounds**: at most 10,000 snippets per `Push`/`Sync`
  request (`DEFAULT_MAX_SYNC_SNIPPETS`); every `limit` is clamped to
  `MAX_REQUEST_LIMIT = 1000`. Omitted limits default to 1000 (`sync`),
  100 (`get_snippets`), 50 (`list_libraries`).
- **Transport bounds**: `grpc_max_message_size` defaults to 4 MiB and is
  applied symmetrically (`max_decoding_message_size` +
  `max_encoding_message_size`); per-request `timeout()` defaults to 30 s
  (`request_timeout_secs`).
- **Sanitized errors**: handlers log the DB detail with a UUID
  `request_id` but return `Status::internal("Internal error")`.
  Ownership failures surface as `not_found` (no existence oracle).
- **Clock skew**: `validate_snippet()` samples `Utc::now()` once per
  call (no micro-boundary split). Future timestamps beyond +300 s fail
  with `CLOCK_SKEW`; zero/negative timestamps fail (zero = unset client
  clock, invisible to delta queries); `created_at > updated_at` fails.
- **Auth + rate limit**: `extract_api_key()` prefers the `authorization:
  Bearer` metadata, falls back to the body field. Every scoped RPC runs
  `authenticate_and_rate_limit_with_duration()` (empty → unauthenticated,
  overlong → invalid_argument, over-limit → resource_exhausted with
  `rate_limit_hits` inc, unknown key → unauthenticated with
  `auth_failures` inc). `Register` keys by peer `SocketAddr` IP, honoring
  `X-Forwarded-For` only for configured `trusted_proxies`.

## HTTP server (EggServe)

**File**: `snip-sync/src/http.rs` (~209 lines). One concrete two-route
leaf service built with `eggserve_server::service_fn_head`; only
`eggserve-server =0.2.1` and `eggserve-primitives =0.2.0` are used.

| Endpoint | Methods | Auth | Body |
|----------|---------|------|------|
| `/health` | GET, HEAD | none | JSON `{version, status}` (`503` when `ping()` fails) |
| `/metrics` | GET, HEAD | Basic (both username + password required) | Prometheus text, or `404 "Not found"` when disabled |

EggServe specifics:

- **Pre-bound listener**: `serve_inner` binds the gRPC *and* HTTP
  `TcpListener`s before spawning either service, then hands the std
  listener to `Server::builder().from_listener()`. Bind failures fail
  fast with no half-started server.
- **Explicit body rejection**: bodies are rejected at the EggServe
  boundary before `handle_request` runs (a `Content-Length: 3` GET never
  reaches the handler). HEAD reuses the GET logic; EggServe suppresses
  the wire body while preserving `content-length`.
- **No total lifetime ceiling**: the runtime is built with
  `disable_connection_total_timeout()` and only a graceful-shutdown
  timeout, so healthy keep-alive connections survive (proven by the
  2-second keep-alive socket test).
- **Typed shutdown completion**: `start_with_service()` yields
  `(ServerControl, ServerCompletion)` via `into_parts()`; the
  orchestrator calls `control.shutdown()`, awaits `completion.wait()`,
  and preserves the terminal error in `ServiceResult`.
- **External TLS**: the server only serves plaintext. `TLS_ENABLED=true`
  acknowledges an upstream terminating proxy; otherwise
  `SNIP_SYNC_ALLOW_HTTP=true` is required and, by
  `validate_transport_policy()`, allowed only when *both* bind addresses
  are loopback. `CORS_ALLOW_ALL=true` is likewise ignored with a warning
  on non-loopback binds.
- **Basic-auth constant-time**: expected `user:pass` is compared with
  `subtle::ct_eq` over a length-padded buffer *and* an exact length
  check; malformed Base64 → `401`. Disabled metrics is a `404` from the
  application, not the router.
- **Configured CORS**: `CorsPolicy { allow_all, allowed_origins }` from
  `CORS_ALLOW_ALL` / `CORS_ALLOWED_ORIGINS` (or `[server.cors]`).
  Preflight answers `Access-Control-Allow-Methods/Headers` and echoes a
  matching `Origin`; allow-all answers `*`/`*`/`*`.
- **Three security headers** on every ordinary response:
  `x-content-type-options: nosniff`, `x-frame-options: DENY`,
  `cache-control: no-store`.

Wire parity (locked by socket tests in `tests/snip_sync_lifetime.rs`):

- Router 404/405 are empty (`content-length: 0`, no `content-type`).
  The metrics-*disabled* 404 is the exception: `text/plain; charset=utf-8`
  `"Not found"`.
- Known routes answer preflight *and* unsupported methods with
  `Allow: GET, HEAD`; unknown-path preflight short-circuits to `200`
  with no `Allow`.
- Every non-allow-all response carries `Vary: origin` (ordinary,
  fallback, and preflight alike); allow-all responses omit `Vary`.
- Preflight responses carry CORS metadata but never the three security
  headers; all ordinary responses (including 404/405) carry them.

## Configuration

**Files**: `snip-sync/src/lib.rs` (`Config::load`), `snip-sync/config.toml`
template, `snip-sync/src/bootstrap.rs`, `snip-sync/src/paths.rs`.

```toml
[server]
grpc_host = "127.0.0.1"
grpc_port = 50051
http_host = "127.0.0.1"
http_port = 50050

[server.database]
path = "snippets.db"          # or DATABASE_URL; default: <config>/snippets.db
max_connections = 5           # or DB_MAX_CONNECTIONS

[server.premade]
# directory = "/var/lib/snip-sync/premade-libraries"  # or PREMADE_DIR

[server.limits]
max_command_length = 1024
max_description_length = 1024
max_tags = 50
max_tag_length = 100
request_timeout_secs = 30
grpc_max_message_size = 4194304

[server.rate_limit]
requests_per_minute = 120

[server.metrics]
username = "admin"
password = "secret"

[server.cors]
allowed_origins = "https://example.com"
```

- **Precedence**: environment beats TOML beats compiled defaults.
  Resolved paths come from `paths.rs` (`CONFIG_PATH`,
  `SNIP_SYNC_STATE_DIR` overrides; `snip-sync paths [--json]` prints them).
- **Strict parsing**: `parse_env()` fails on any present-but-invalid
  value (`InvalidEnvironment` names the variable and value); TOML type
  errors fail with the file path (`Parse`). Booleans accept only
  case-insensitive `true/1/yes/on` and `false/0/no/off`.
- **Range validation**: zero ports, zero DB connections, zero timeouts,
  zero message size, zero field limits, and zero rate limit are all
  rejected with `InvalidRange`.
- **Fail-closed**: a present-but-malformed/unreadable `config.toml`
  makes `serve` and `croncheck` error identifying the path; compiled
  defaults apply only when the file is absent. `ensure_config_file()`
  creates it with `create_new` (no clobber) and `0o600` on Unix; a
  world-readable file holding metrics credentials logs a warning.

## Process lifecycle

`serve` holds the singleton kernel lock for its full runtime. `stop` and
`restart` prefer that lock and use legacy PID files only as a fallback.

- **Singleton**: `ServerLock::try_acquire(&state_dir)` at startup
  (`flock` on Unix, `LockFileEx` on Windows). The kernel owns exclusion;
  the lock file (`snip-sync.server.lock`: pid, start token, nonce,
  timestamp) is diagnostic metadata. `Drop` releases without unlinking,
  so crash leftovers are harmless — the kernel already freed the lock.
  `Busy { owner }` reports `pid`, `UnsupportedPlatform` refuses to run.
- **Legacy fallback**: `process::parse_pid_file()` understands
  `Structured` records, bare `LegacyPid`, `Empty`, and `Malformed`.
  Current servers never write it. `stop`/`restart` read it only when the
  kernel lock is free, verify liveness (`kill(pid,0)`: only `ESRCH`
  proves absence), start token (`/proc/<pid>/stat` field 22 on Linux),
  and process name (refused unless `--force`), then `remove_pid_if_unchanged()`
  only after re-acquiring the lock and rereading the identical record.
- **Signals**: Unix registers both `ctrl_c()` and `SIGTERM`, so
  `snip-sync stop` (SIGTERM) and Ctrl-C share the graceful path. Windows
  terminates the recorded process and re-checks the lock.
- **Coordinated drain**: one `broadcast::channel(())` feeds both
  services. gRPC runs `serve_with_incoming_shutdown()` on a
  `TcpListenerStream`; EggServe drains via `ServerControl` /
  `ServerCompletion`. `run_eggserve_services_until_shutdown()` selects
  over the process signal, the gRPC `JoinHandle`, and the borrowed
  cancellation-safe EggServe completion future, then broadcasts, requests
  EggServe shutdown, and drains both under `request_timeout_secs`.
  Forced abort preserves classifications and the original detail.
- **Outcome**: `ServiceShutdownOutcome { requested, forced, grpc_result,
  http_result }` with `ServiceResult::{Clean, ServiceError, Panic,
  Cancelled}`. `serve_inner` calls `ensure_clean_requested_shutdown()`:
  `Ok(())` only for requested + unforced + dual-clean; anything else
  (forced drain, unexpected exit, error/panic) returns a diagnostic
  naming both classifications. `server_exits_cleanly_on_signal` proves a
  SIGTERM exit code 0 plus replacement bind on the same ports.

`startup.rs` owns the surrounding lifecycle: `check_health()` raw-socket
probe, `croncheck` (lock-guarded, spawns `serve` detached, 5 s health
gate), `install`/`instructions`/`uninstall` for systemd / launchd / cron
/ Task Scheduler (ownership markers, exact quoting, root policy), and
`restart_after_update()` / `restart_if_managed()` so updates reuse the
installed manager. `update --dry-run` probes liveness read-only without
publishing lock metadata.

## Self-update and the Plan 017 size gate

`snip-sync update` (`src/update.rs`, binary-private) is binary-first:
crates.io selects the stable version, the exact `snip-sync-vX.Y.Z` GitHub
tag supplies the asset + `.sha256` sidecar, and the candidate must print
`snip-sync X.Y.Z` before Unix atomic stage-and-rename (Windows defers to
the `__self-replace` helper with `MoveFileExW`). It deliberately shells
out to external `curl` (`--proto =https`, TLS 1.2+, 10 s connect / 60 s
total, byte caps) instead of sharing `snp`'s in-process `eggfetch-core`
transport: the Plan 017 lean-profile trial grew the server from
3,833,152 to 5,145,224 bytes (+34.23%), past the 10% material-growth
gate. Do not consolidate without re-running that measurement.

## Security properties

- Argon2id (16 MiB, 3 iterations, parallelism 4) with per-key salt; only
  the 8-char SHA-256 prefix is indexed; plaintext migration is atomic at
  startup and fail-closed.
- Rate limiting: 120 requests/minute default, per-API-key for data RPCs
  and per-peer-IP for `Register`; sliding epoch-second windows, bounded
  at 100,000 entries, process-local (resets on restart).
- Auth metadata is never retained in production state. Bearer capture
  (`captured_auth_header`), the sanitized `test_observer`, and push-fault
  injection exist only under `test-helpers` / `#[cfg(test)]`.
- Metrics endpoint requires both username and password over HTTP Basic
  auth with constant-time comparison; half-configured metrics stay
  disabled (`404`), never half-open.
- Plaintext serve is loopback-only unless a proxy terminates TLS
  (`TLS_ENABLED=true`); config files default to `0o600` with a
  world-readable warning when they hold credentials.
- Premade filenames are allowlisted (`alphanumeric/-/_/.`, ≤64 chars,
  no `..`/`/` `\`) and resolved via canonicalize-plus-prefix check;
  error paths never leak DB internals to clients.

## Key files

Line counts at time of writing (`wc -l`):

- `snip-sync/src/lib.rs` (~2599) — service, config, validation, 11 RPCs
- `snip-sync/src/startup.rs` (~1202) — supervisors, transport policy, restart
- `snip-sync/src/db.rs` (~1305) — schema, auth, CRUD, migration
- `snip-sync/src/orchestration.rs` (~894) — typed shutdown + tests
- `snip-sync/src/main.rs` (~721) — dispatch, serve wiring, stop/restart
- `snip-sync/src/update.rs` (~778) — curl updater, Plan 017 gate note
- `snip-sync/src/premade.rs` (~535) — library scan and safe fetch
- `snip-sync/src/process.rs` (~479) — PID fallback, identity, signaling
- `snip-sync/src/server_lock.rs` (~460) — kernel singleton lock
- `snip-sync/src/test_observer.rs` (~413) — sanitized telemetry (test-only)
- `snip-sync/src/http.rs` (~209) — two-route EggServe service
- `snip-sync/src/cli.rs` (~206) — subcommands and flags
- `snip-sync/src/editor.rs` (~166) — `$EDITOR` handling for `edit`
- `snip-sync/src/rate_limiter.rs` (~159) — sliding-window limiter
- `snip-sync/src/cert.rs` (~152) — dev certs via `openssl`
- `snip-sync/src/paths.rs` (~144) — XDG-ish path resolution
- `snip-sync/src/metrics.rs` (~128) — Prometheus instruments
- `snip-sync/src/bootstrap.rs` (~119) — layout + default config
- `snip-sync/src/test_helpers.rs` (~109) — in-process test server
- `snip-sync/Cargo.toml` (~75) — pinned EggServe/tonic/sqlx deps
- `tests/snip_sync_lifetime.rs` (~826) — socket wire-parity + lifetime gates
