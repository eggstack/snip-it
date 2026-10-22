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

This doc covers the **server side** only. The wire schema lives in
[proto.md](proto.md); the client, merge algorithm, and retry ladder live in
[sync.md](sync.md); the client-side encryption contract lives in
[encryption.md](encryption.md); and the client-side persistence rules live
in [persistence.md](persistence.md) and [library.md](library.md). The
server's own updater transport is cross-referenced from
[update.md](update.md) but documented here, because the server keeps a
*different* transport than `snp update` on purpose.

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
│   │  │ (sqlx /  │  │ (in-memory,      │     │               │
│   │  │ SQLite)  │  │ peer-keyed,      │     │               │
│   │  │          │  │ 120/min default) │     │               │
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
`snip-proto` stubs), `eggserve-server =0.4.0` +
`eggserve-primitives =0.2.2` with default features off, `sqlx 0.9`
(`sqlite`, `runtime-tokio`, `chrono`), `argon2 0.6`, `sha2 0.11`,
`subtle 2`, `prometheus 0.14`, `base64 0.23`, `toml 1.1`, `tokio`
(`macros`, `rt-multi-thread`, `signal`), `tokio-stream`, `clap 4.5`,
`dirs 7.0`, `uuid 1`, `semver`, `libc 0.2` / `windows-sys 0.61`.
No Axum, no Tower-HTTP, no generic router/middleware.

## Table of Contents

- [Overview](#overview)
- [Module map (19 files)](#module-map-19-files)
- [Process lifecycle](#process-lifecycle)
- [Database](#database)
- [gRPC service](#grpc-service)
- [HTTP server (EggServe)](#http-server-eggserve)
- [Rate limiting](#rate-limiting)
- [Metrics](#metrics)
- [TLS and certificate management](#tls-and-certificate-management)
- [Configuration](#configuration)
- [Self-update and the Plan 017 size gate](#self-update-and-the-plan-017-size-gate)
- [Key Files](#key-files)
- [Security properties](#security-properties)
- [Invariants and Gotchas](#invariants-and-gotchas)

## Module map (19 files)

gRPC handler code lives in `lib.rs`. `main.rs` is CLI dispatch plus
`serve()` / `serve_inner()` wiring only. `update.rs` is binary-private
(`mod update` in `main.rs`), not a library module.

| File (`snip-sync/src/`) | Responsibility |
|--------------------------|----------------|
| `lib.rs` | `SnipSyncService` + all 11 gRPC handlers, `Config` load/validate, `extract_api_key`, `validate_snippet`, `peer_rate_limit_key`, `UpsertOutcome` dispatch, metrics helpers |
| `main.rs` | CLI dispatch, `serve()` gate (TLS policy, layout, lock), `serve_inner()` listener bind + service spawn, `stop`/`restart`/`croncheck`/`paths` |
| `http.rs` | Concrete two-route EggServe leaf service (`/health`, `/metrics`), `CorsPolicy`, Basic-auth check, security headers |
| `orchestration.rs` | Production `run_eggserve_services_until_shutdown`, `ServiceShutdownOutcome`, `ServiceResult`, `ensure_clean_requested_shutdown` |
| `startup.rs` | Fail-closed transport policy, supervisor records (systemd/launchd/cron/Task Scheduler), health probe, managed restart, update lifecycle |
| `update.rs` | Binary-first self-update via external `curl` (Plan 017 size gate, see below); checksums, identity check, Windows self-replace helper |
| `db.rs` | SQLite schema, Argon2id auth, snippet/library CRUD, plaintext migration, `UpsertOutcome` classification |
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

## Process lifecycle

`serve` holds the singleton kernel lock for its full runtime. `stop` and
`restart` prefer that lock and use legacy PID files only as a fallback.

### Startup gate

`serve()` refuses to start unless the transport policy is satisfied
(`snip-sync/src/main.rs:59-91`):

1. `TLS_ENABLED=true` acknowledges that an upstream reverse proxy
   terminates TLS. `snip-sync` never terminates TLS itself.
2. Otherwise `SNIP_SYNC_ALLOW_HTTP=true` is required, and
   `startup::validate_transport_policy()` (`snip-sync/src/startup.rs:274-293`)
   additionally requires that *both* the gRPC and HTTP bind addresses are
   loopback. A non-loopback plaintext bind is refused, not warned about.
3. `bootstrap::ensure_layout()` + `ensure_config_file()` create the
   directory layout and the default `config.toml` if it is missing.
4. `Config::load()` (`snip-sync/src/lib.rs:359`) is **fail-closed**: a
   present-but-malformed or unreadable `config.toml` aborts startup with
   the path in the error; compiled defaults apply only when the file is
   absent (`snip-sync/src/lib.rs:368-395`). Every resolved value is then
   range-checked by `Config::validate()` (`snip-sync/src/lib.rs:499-599`),
   which rejects zero ports, zero DB connections, zero timeouts, zero
   message size, zero field limits, and a zero rate limit.

### The kernel-backed singleton lock

- `ServerLock::try_acquire(&state_dir)` at startup
  (`snip-sync/src/main.rs:97`). The Unix path uses `flock(LOCK_EX |
  LOCK_NB)` (`snip-sync/src/server_lock.rs:241`); Windows uses
  `LockFileEx` with `LOCKFILE_EXCLUSIVE_LOCK |
  LOCKFILE_FAIL_IMMEDIATELY` (`snip-sync/src/server_lock.rs:261-270`).
- The kernel owns exclusion; the lock file (`snip-sync.server.lock`) and its
  owner record are **diagnostic metadata only**. `Drop` releases without
  unlinking, so crash leftovers are harmless — the kernel already freed the
  lock (`snip-sync/src/server_lock.rs:121-129`).
- The owner record (pid, start token, nonce, timestamp) is published to a
  **sidecar file** beside the lock — `snip-sync.server.lock.owner`
  (`snip-sync/src/server_lock.rs:177-184`), written through a temp file and
  an atomic rename (`snip-sync/src/server_lock.rs:345-396`). The lock file
  itself is truncated and rewritten in place, so a concurrent reader could
  observe it empty and lose the ability to identify the running server; the
  sidecar removes that window. `read_owner` prefers the sidecar and falls
  back to the in-place record (`snip-sync/src/server_lock.rs:398-406`).
- The sidecar is deliberately *not* the lock file: renaming a new inode over
  `snip-sync.server.lock` would move the kernel lock to the old, unlinked
  inode and let a second server flock the new inode successfully. The `flock`
  therefore stays on the canonical inode.
- `Busy { owner }` reports the holding `pid`; `UnsupportedPlatform`
  refuses to run (`snip-sync/src/main.rs:99-113`).
- The guard is held for the whole `serve()` lifetime
  (`snip-sync/src/main.rs:97-119`). Current servers never write a PID
  file — the kernel lock *is* the exclusion record, and the sidecar is the
  identity record beside it.

### Legacy PID fallback

- `process::parse_pid_file()` (`snip-sync/src/process.rs:35-59`)
  understands `Structured` records, bare `LegacyPid`, `Empty`, and
  `Malformed`.
- `stop`/`restart` read it only when the kernel lock is free, verify
  liveness (`kill(pid,0)`: only `ESRCH` proves absence), start token
  (`/proc/<pid>/stat` field 22 on Linux), and process name (refused
  unless `--force`), then `remove_pid_if_unchanged()`
  (`snip-sync/src/process.rs:64`) only after re-acquiring the lock and
  rereading the identical record.

### Pre-bound listeners

`serve_inner()` binds the gRPC *and* HTTP `TcpListener`s **before**
spawning either service (`snip-sync/src/main.rs:149-150`), so a port or
address error fails the command immediately instead of leaving a
half-started server running with a misleading successful exit.

The startup signal is the bind-time log pair at
`snip-sync/src/main.rs:152-153` (`gRPC server listening on …` /
`HTTP server listening on …`). The EggServe side logs its own
`HTTP server listening on http://…` after `into_parts()` has handed the
control/completion pair to the orchestrator
(`snip-sync/src/main.rs:276`) — still before the serve expression. There
is deliberately **no** second "listening" log after
`serve_with_incoming_shutdown()` — that expression only returns once the
server has already terminated, so a log there would print a startup line
at shutdown time on every run
(`snip-sync/src/main.rs:283-287`).

### Signals and coordinated drain

- Unix registers both `ctrl_c()` and `SIGTERM`
  (`snip-sync/src/main.rs:311-338`), so `snip-sync stop` (SIGTERM) and
  Ctrl-C share the graceful path. Windows terminates the recorded process
  and re-checks the lock.
- One `broadcast::channel(())` feeds both services
  (`snip-sync/src/main.rs:280-282`). gRPC runs
  `serve_with_incoming_shutdown()` over a `TcpListenerStream`
  (`snip-sync/src/main.rs:288-303`); EggServe drains via
  `ServerControl` / `ServerCompletion`.
- `run_eggserve_services_until_shutdown()` selects over the process
  signal, the gRPC `JoinHandle`, and the borrowed cancellation-safe
  EggServe completion future, then broadcasts, requests EggServe
  shutdown, and drains both under `request_timeout_secs`
  (`snip-sync/src/orchestration.rs:9-78`).
- **Outcome**: `ServiceShutdownOutcome { requested, forced, grpc_result,
  http_result }` with `ServiceResult::{Clean, ServiceError, Panic,
  Cancelled}`. `serve_inner` calls `ensure_clean_requested_shutdown()`
  (`snip-sync/src/main.rs:354`,
  `snip-sync/src/orchestration.rs:124-134`): `Ok(())` only for
  requested + unforced + dual-clean; anything else (forced drain,
  unexpected exit, error/panic) returns a diagnostic naming both
  classifications. `server_exits_cleanly_on_signal` proves a SIGTERM
  exit code 0 plus replacement bind on the same ports
  (`tests/snip_sync_lifetime.rs:797-840`).

### CLI surface

`snip-sync/src/cli.rs:16-102` defines the subcommands:

| Subcommand | Purpose |
|------------|---------|
| `serve` | Start the server (the default when no subcommand is given) |
| `init` | Initialize config, directories, and dev certificates (`--force-cert`, `--skip-cert`) |
| `cert` | Generate self-signed dev certificates (`--force`, `--out-dir`) |
| `edit` | Open the config file in `$EDITOR` |
| `stop` | Stop a running server (`--force`) |
| `restart` | Stop + serve (`--force`) |
| `startup install` | Install boot-time startup registration (`--method auto\|systemd\|launchd\|cron\|task-scheduler`) |
| `startup instructions` | Print exact manual startup commands and rendered configuration |
| `startup uninstall` | Remove snip-sync-owned startup registration |
| `update` | Check for and install an update from crates.io (`--dry-run`, `--locked`) |
| `croncheck` | Health check; start the server if unhealthy/down (`--verbose`) |
| `paths` | Print resolved paths (`--json`) |
| `completions` | Generate shell completions |
| `version` | Print version |
| `__self-replace` | Hidden internal Windows self-replacement helper |

`startup.rs` owns the surrounding lifecycle: `check_health()` raw-socket
probe (`snip-sync/src/startup.rs:868`), `croncheck` (lock-guarded, spawns
`serve` detached, 5 s health gate), `install`/`instructions`/`uninstall`
for systemd / launchd / cron / Task Scheduler (ownership markers, exact
quoting, root policy), and `restart_after_update()` /
`restart_if_managed()` so updates reuse the installed manager.
`update --dry-run` probes liveness read-only without publishing lock
metadata.

## Database

**File**: `snip-sync/src/db.rs`. SQLite via `sqlx`; file path in
production, `sqlite::memory:` in tests. Schema is created with
`CREATE TABLE IF NOT EXISTS` plus explicit indexes.

### Connection setup

`Database::connect()` (`snip-sync/src/db.rs:253`) builds the pool with
`SqlitePoolOptions::new().max_connections(max_connections)` and — the
part that matters — an `after_connect` hook:

```rust
let pool = SqlitePoolOptions::new()
    .max_connections(max_connections)
    .after_connect(|conn, _meta| {
        Box::pin(async move {
            sqlx::query("PRAGMA foreign_keys=ON").execute(conn).await?;
            Ok(())
        })
    })
    .connect_with(options)
    .await?;
```

`snip-sync/src/db.rs:287-296`

`foreign_keys` is a **per-connection** SQLite setting (unlike
`journal_mode`, which SQLite persists in the database file header). Running
the pragma once against the pool set it on whichever single connection it
happened to check out — at most 1 of `max_connections` (default 5) —
leaving the schema's declared foreign keys unenforced on the rest.
`after_connect` runs for every connection the pool opens, so FK
enforcement is now uniform across the pool. `PRAGMA journal_mode=WAL` is
applied once to the pool (`snip-sync/src/db.rs:298-300`) because it *is*
persisted in the file header.

### Tables

| Table | Columns / constraints |
|-------|-----------------------|
| `users` | `id` (PK), `api_key` (UNIQUE, Argon2id PHC hash), `api_key_prefix` (nullable, indexed), `created_at`, `updated_at` |
| `libraries` | `id` (PK), `user_id` (FK → `users`), `name`, `created_at`, `deleted_at` (soft delete), `UNIQUE(user_id, name)` |
| `snippets` | `id` (**global** PK), `user_id` (FK), `library_id` (FK), `description`, `command`, `tags` (JSON text, default `'[]'`), `created_at`, `updated_at`, `device_id`, `deleted` (0/1), `encrypted` (0/1) |

Schema DDL: `snip-sync/src/db.rs:304-356`.

Indexes: `idx_users_api_key_prefix`, `idx_snippets_user`,
`idx_snippets_library`, `idx_snippets_updated`,
`idx_snippets_user_library_updated (user_id, library_id, updated_at,
deleted)`, `idx_libraries_user`.

### API-key hashing and lookup

- **Hash**: Argon2id (`Algorithm::Argon2id`, `V0x13`), 16 MiB memory
  (`1 << 14` KiB), 3 iterations, parallelism 4, fresh 16-byte salt per
  key; PHC string stored in `users.api_key`
  (`snip-sync/src/db.rs:92-109`).
- **Indexed lookup**: `api_key_prefix` = first 8 chars of
  `base64(SHA-256(api_key))` (`STANDARD_NO_PAD`). Auth queries
  `WHERE api_key_prefix = ?` then Argon2-verifies only that bucket
  (`snip-sync/src/db.rs:426-446`).
- **Legacy backfill**: `migrate_plaintext_api_keys()` runs at startup
  before listeners spawn; failure halts startup so a half-migrated DB
  cannot lock users out (`snip-sync/src/main.rs:131-141`). The old
  NULL-prefix O(N)-verify fallback was deliberately removed (DoS
  amplifier); `derive_prefix_from_hash()` only backfills index metadata,
  it cannot recover keys from hashes.

### The global `snippets.id` primary key

`snippets.id` is `TEXT PRIMARY KEY` — **global**, not scoped to
`(user_id, library_id)` (`snip-sync/src/db.rs:339`). Per-query scoping
(`WHERE user_id = ? AND library_id = ?`) still applies to every read
(`snip-sync/src/db.rs:165-210`), but the *write* path can hit an id that
another scope already owns.

The upsert uses a last-write-wins `ON CONFLICT(id) DO UPDATE … WHERE`
whose `WHERE` clause requires the incoming row to match the stored row's
`user_id` **and** `library_id`
(`snip-sync/src/db.rs:718-733`). SQLite reports a false `WHERE` on
`DO UPDATE` as a silent no-op — **0 rows affected, no error**. A push
whose id already belongs to another user or library therefore wrote
zero rows while looking successful.

The server's merge rule is **not** the client's. Inside the same `WHERE`
clause the server applies a lexicographic last-write-wins over
`(updated_at, device_id, command, description)` — plain string comparison,
**no SHA-256 fingerprint** (`snip-sync/src/db.rs:729-733`). `deleted` is an
ordinary column (`deleted = excluded.deleted`), so a newer *live* row
overwrites a tombstone: the server does **not** guarantee no resurrection.
The no-resurrection guarantee belongs to the client's `choose_version`
(`src/sync_commands.rs:1199`, see [sync.md](sync.md)).

`upsert_snippet_in_tx()` now classifies the outcome explicitly
(`snip-sync/src/db.rs:35-53`):

```rust
pub enum UpsertOutcome {
    /// The row was inserted or updated.
    Written,
    /// A newer version of the same record already exists under the same user
    /// and library; last-write-wins correctly kept it.
    Stale,
    /// A row with the same id already belongs to a different user and/or
    /// library. The write was discarded — reporting this as `Stale` would
    /// silently lose the snippet.
    ForeignScope,
}
```

Zero rows affected triggers a scope probe — a `SELECT user_id,
library_id FROM snippets WHERE id = ? AND (user_id != ? OR library_id != ?)`
— which distinguishes the two cases
(`snip-sync/src/db.rs:749-770`).

This matters in practice because the client's snippet ids are
**deterministic** (`legacy-<sha256 hex>` from content + occurrence index;
see [persistence.md](persistence.md) and `docs/IDENTITY_CONTRACT.md`).
Identical content in two libraries of one install produces identical ids,
so a legitimate second library can collide with the first.

### Key operations

| Operation | Notes |
|-----------|-------|
| `create_user(api_key)` | Hash + prefix, insert user + `default` library in one transaction; returns user id |
| `get_user_by_api_key(key)` | Prefix-bucket select, Argon2 verify, `Ok(None)` on miss |
| `upsert_snippet_in_tx(tx, snippet, user_id, library_id)` | Last-write-wins upsert inside a caller-owned transaction; returns `UpsertOutcome` |
| `upsert_snippet(...)` | Test-only convenience wrapper (`#[cfg(test)]`, `snip-sync/src/db.rs:656`) |
| `get_snippets(user, lib, since, limit, offset, include_deleted)` | Paginated delta read; total count + `has_more` computed by caller |
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
| `Register` | New UUID API key + `create_user`; peer-address rate limited |
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

- **Field limits** (configurable, defaults in
  `snip-sync/src/lib.rs:95-113`): `max_command_length` 1024,
  `max_encrypted_payload_length` 1 MiB, `max_description_length` 1024,
  `max_tags` 50, `max_tag_length` 100, `max_id_length` 128,
  `max_device_id_length` 128, `max_api_key_length` 512,
  `rate_limit_per_minute` 120, `db_max_connections` 5,
  `grpc_max_message_size` 4 MiB, `request_timeout_secs` 30.
  Plaintext snippets require a non-blank command and are capped by
  `max_command_length`. Encrypted snippets require a non-blank payload but
  are bounded by `max_encrypted_payload_length` instead, measured on the
  ciphertext: `command` then holds the whole
  `{description, command, tags}` blob, and anything larger than the gRPC
  ceiling could be stored but never downloaded again
  (`snip-sync/src/lib.rs:890-912`).
- **Batch/page bounds**: at most 10,000 snippets per `Push`/`Sync`
  request (`DEFAULT_MAX_SYNC_SNIPPETS`, `snip-sync/src/lib.rs:109`,
  enforced at `snip-sync/src/lib.rs:1239` and `snip-sync/src/lib.rs:1391`
  — a compile-time constant, not configurable); every `limit` is clamped
  to `MAX_REQUEST_LIMIT = 1000` (`snip-sync/src/lib.rs:117`). Omitted
  limits default to 1000 (`sync`, `snip-sync/src/lib.rs:1537-1541`),
  100 (`get_snippets`, `snip-sync/src/lib.rs:1128-1132`), 50
  (`list_libraries`, `snip-sync/src/lib.rs:1718-1722`). Download pages are
  *also* bounded by encoded bytes: `fit_page_to_byte_ceiling()` trims
  trailing rows so the encoded page stays under
  `grpc_max_message_size - 64 KiB` envelope reserve (minus `skipped_ids`
  bytes), sets `has_more`, and always keeps at least one row
  (`snip-sync/src/lib.rs:140-165`, applied at
  `snip-sync/src/lib.rs:1586-1594`). Without this, row-count paging alone
  could produce a page tonic cannot encode, which returns `OUT_OF_RANGE`
  and makes the library permanently undownloadable.
- **Transport bounds**: `grpc_max_message_size` defaults to 4 MiB and is
  applied symmetrically (`max_decoding_message_size` +
  `max_encoding_message_size`, `snip-sync/src/main.rs:290-296`);
  per-request `timeout()` defaults to 30 s (`request_timeout_secs`).
- **Sanitized errors**: handlers log the DB detail with a UUID
  `request_id` but return `Status::internal("Internal error")`.
  Ownership failures surface as `not_found` (no existence oracle).
- **Clock skew**: `validate_snippet()` samples `Utc::now()` once per
  call (no micro-boundary split). Future timestamps beyond +300 s fail
  with `CLOCK_SKEW`; zero/negative timestamps fail (zero = unset client
  clock, invisible to delta queries); `created_at > updated_at` fails
  (`snip-sync/src/lib.rs:869-977`).
- **Auth + rate limit**: `extract_api_key()` prefers the
  `authorization: Bearer` metadata, falls back to the body field. Every
  scoped RPC runs `authenticate_and_rate_limit_with_duration()`
  (`snip-sync/src/lib.rs:810-867`): empty key → unauthenticated,
  overlong → invalid_argument, over-limit → resource_exhausted with
  `rate_limit_hits` inc, unknown key → unauthenticated with
  `auth_failures` inc.

### Response contract: accepted, rejected, skipped

The server distinguishes three per-record outcomes, and the client folds
them differently:

| Server signal | Meaning | Client treatment |
|----------------|---------|------------------|
| `PushSnippetsResponse.accepted_count` | Row written, or a genuine last-write-wins dedupe (`Written` / `Stale`) | Accepted |
| `PushSnippetsResponse.rejected_count` | Validation refusal **or** a cross-scope id collision (`ForeignScope`) | Folded into `skipped_count` (counts only — `PushSnippetsResponse` carries no per-record ids) |
| `SyncResponse.skipped_ids` | Per-record ids that failed validation, upserted into a foreign scope, or errored | Surfaced as ids, and counted in `skipped_count` |

- `PushSnippets` rejects a cross-scope collision and reports it in
  `rejected_count` (`snip-sync/src/lib.rs:1307-1327`). `success` is
  `rejected == 0` (`snip-sync/src/lib.rs:1365-1370`), so a partial
  refusal is a `success: false` response with a non-zero
  `rejected_count`, not a transport error.
- `Sync` reports the same collision in `skipped_ids`
  (`snip-sync/src/lib.rs:1486-1526`) and sets
  `message = "Sync completed, N snippets skipped"`.
- A **genuine** last-write-wins dedupe (a newer row already exists under
  the same user and library) is *not* a rejection: it counts as accepted
  in `PushSnippets` (`snip-sync/src/lib.rs:1312`).
- On the client, `PushSnippets` refusals are folded into `skipped_count`
  only (`src/sync.rs:860-890`); the server's `SyncResponse.skipped_ids`
  are surfaced verbatim (`src/sync.rs:886`).

### Client retry contract (cross-reference)

The client retries `ResourceExhausted` (the server's rate-limit code) but
never `OutOfRange` — tonic raises `OUT_OF_RANGE` when a message cannot be
encoded or decoded within the size limit, which is a deterministic verdict
about the payload rather than a transient condition
(`src/sync.rs:109-131`). `InvalidArgument`, `NotFound`, `AlreadyExists`,
`PermissionDenied`, and `Unauthenticated` are also non-retryable.

Two RPCs are deliberately **not** retried at all, because they are not
idempotent and the server offers no way to deduplicate them:

- `Register` mints a fresh account and a fresh API key on every call and
  discards the request body, so there is no idempotency key. A retry after a
  lost response would leave a second, live, untracked credential on the
  server (`src/sync.rs:1070-1080`).
- `CreateLibrary` mints a fresh library id on every call, but the server
  enforces `UNIQUE(user_id, name)`, so a retry comes back as `ALREADY_EXISTS`.
  The client resolves that conflict by **adopting** the existing server
  library instead of failing, so the local library is never left permanently
  unlinked (`src/sync.rs:1163-1215`).

## HTTP server (EggServe)

**File**: `snip-sync/src/http.rs` (230 lines). One concrete two-route
leaf service built with `eggserve_server::service_fn_head`; only
`eggserve-server =0.4.0` and `eggserve-primitives =0.2.2` are used
(`snip-sync/Cargo.toml:39-40`).

| Endpoint | Methods | Auth | Body |
|----------|---------|------|------|
| `/health` | GET, HEAD | none | JSON `{version, status}` (`503` when `ping()` fails) |
| `/metrics` | GET, HEAD | Basic (both username + password required) | Prometheus text, or `404 "Not found"` when disabled |

### EggServe specifics

- **Pre-bound listener**: `serve_inner` binds the gRPC *and* HTTP
  `TcpListener`s before spawning either service, then hands the std
  listener to `Server::builder().from_listener()`
  (`snip-sync/src/main.rs:149-150`, `snip-sync/src/main.rs:264-275`).
  Bind failures fail fast with no half-started server.
- **Explicit body rejection**: bodies are rejected at the EggServe
  boundary before `handle_request` runs (a `Content-Length: 3` GET never
  reaches the handler; `tests/snip_sync_lifetime.rs:304-315`).
- **No total lifetime ceiling**: the runtime is built with
  `disable_connection_total_timeout()` and only a graceful-shutdown
  timeout (`snip-sync/src/main.rs:264-268`), so healthy keep-alive
  connections survive (proven by the 2-second keep-alive socket test,
  `tests/snip_sync_lifetime.rs:335-350`).
- **Typed shutdown completion**: `start_with_service()` yields
  `(ServerControl, ServerCompletion)` via `into_parts()`; the
  orchestrator calls `control.shutdown()`, awaits `completion.wait()`,
  and preserves the terminal error in `ServiceResult`.
- **External TLS**: the server only serves plaintext. `TLS_ENABLED=true`
  acknowledges an upstream terminating proxy; otherwise
  `SNIP_SYNC_ALLOW_HTTP=true` is required and, by
  `validate_transport_policy()`, allowed only when *both* bind addresses
  are loopback. `CORS_ALLOW_ALL=true` is likewise ignored with a warning
  on non-loopback binds (`snip-sync/src/main.rs:228-242`).

### Basic authentication

- The scheme token is matched **case-insensitively** per RFC 9110 §11.1:
  `scheme.eq_ignore_ascii_case("basic")`
  (`snip-sync/src/http.rs:158-165`).
- The expected `user:pass` is compared with `subtle::ct_eq` over a
  length-padded buffer *and* an exact length check; malformed Base64 →
  `401` (`snip-sync/src/http.rs:166-175`).
- A `401` **must** carry a `WWW-Authenticate` challenge. Without it a
  client cannot learn the scheme and browsers will not prompt for
  credentials — so a password-protected `/metrics` was unreachable
  through standard HTTP auth flows. The header is now sent as
  `Basic realm="metrics", charset="UTF-8"`
  (`snip-sync/src/http.rs:117-126`).
- HTTP auth failures increment the same
  `snip_sync_auth_failures_total` counter as the gRPC path, so a
  password-protected `/metrics` is observable under brute force
  (`snip-sync/src/http.rs:176-187`).
- Disabled metrics is a `404` from the application, not the router
  (`snip-sync/src/http.rs:150-156`).

### CORS and security headers

- **Configured CORS**: `CorsPolicy { allow_all, allowed_origins }` from
  `CORS_ALLOW_ALL` / `CORS_ALLOWED_ORIGINS` (or `[server.cors]`).
  Preflight answers `Access-Control-Allow-Methods/Headers` and echoes a
  matching `Origin`; allow-all answers `*`/`*`/`*`
  (`snip-sync/src/http.rs:54-84`, `snip-sync/src/http.rs:127-138`).
- **Three security headers** on every ordinary response:
  `x-content-type-options: nosniff`, `x-frame-options: DENY`,
  `cache-control: no-store` (`snip-sync/src/http.rs:106-110`).

### Wire-parity contract

Locked by socket tests in `tests/snip_sync_lifetime.rs`:

- Router 404/405 are empty (`content-length: 0`, no `content-type`).
  The metrics-*disabled* 404 is the exception: `text/plain; charset=utf-8`
  `"Not found"` (`tests/snip_sync_lifetime.rs:689-745`).
- Known routes answer preflight *and* unsupported methods with
  `Allow: GET, HEAD`; unknown-path preflight short-circuits to `200`
  with no `Allow` (`tests/snip_sync_lifetime.rs:747-795`).
- Every non-allow-all response carries `Vary: origin` (ordinary,
  fallback, and preflight alike); allow-all responses omit `Vary`
  (`tests/snip_sync_lifetime.rs:591-687`).
- Preflight responses carry CORS metadata but never the three security
  headers; all ordinary responses (including 404/405) carry them.
- A `401` on `/metrics` carries `WWW-Authenticate: basic realm="metrics"`
  (case-insensitively compared), and the authorized response body contains
  `snip_sync_auth_failures_total`
  (`tests/snip_sync_lifetime.rs:357-439`).

## Rate limiting

**File**: `snip-sync/src/rate_limiter.rs`. Default 120 requests/minute
(`DEFAULT_RATE_LIMIT_PER_MINUTE`, `snip-sync/src/lib.rs:111`).

### Keying: the peer address, never the API key

`peer_rate_limit_key()` (`snip-sync/src/lib.rs:781-806`) derives the
bucket identity from the peer address that tonic populates into
`request.extensions()` as a `tonic::transport::server::TcpConnectInfo`
(`remote_addr()`).

The limiter used to key on the **raw, request-supplied API key**, before
validation. Because the key is not validated until `db.get_user_by_api_key`
runs, every attacker-chosen key got a fresh budget of
`rate_limit_per_minute`, and the limiter imposed **no aggregate request
ceiling** on the authenticated gRPC surface. The peer address is bounded
by the number of real callers, so keying on it restores an aggregate
ceiling. Every RPC now uses the peer-derived key: `register`
(`snip-sync/src/lib.rs:1058`), `get_snippets`, `push_snippets`, `sync`,
`create_library`, `list_libraries`, `delete_library`,
`list_premade_libraries`, `get_premade_library`, `search_premade_libraries`.

The extension type matters: tonic inserts a `TcpConnectInfo`, never a bare
`std::net::SocketAddr`, so a lookup for the latter yields `None` for every
request and collapses every caller onto one global `"unknown"` bucket.

### Two buckets: pre-authentication and authenticated

The limiter runs in two stages so that an unauthenticated caller cannot
spend the budget of a legitimate one:

| Bucket | Key | Limit | Enforced |
|--------|-----|-------|----------|
| pre-auth | `pre-auth:<peer>` | `rate_limit_per_minute` × `PRE_AUTH_RATE_LIMIT_MULTIPLIER` (10) | before the API-key lookup |
| authenticated | `<peer>` | `rate_limit_per_minute` | after a **valid** API key was accepted |
| registration | `register:<peer>` | `rate_limit_per_minute` | `register` only (unauthenticated by nature) |

The pre-auth bucket only has to bound the cost of the key lookup, so it is
generous by design. The authenticated bucket is the per-caller ceiling: only
a request that presented a valid key can spend it, so an invalid key can
never lock out a legitimate caller behind the same peer address.
`register` gets its own bucket for the same reason — it is unauthenticated
and must not consume the authenticated budget.

Over-limit calls return `Status::resource_exhausted` and increment
`snip_sync_rate_limit_hits_total`.

### Trusted proxies and `x-forwarded-for`

`x-forwarded-for` is honoured **only** when the peer address is listed in
`trusted_proxies` (`snip-sync/src/lib.rs:797-806`). The trusted list
comes from the `TRUSTED_PROXIES` environment variable (comma-separated,
entries that parse as `IpAddr` are kept) or `[server.rate_limit]
trusted_proxies` (`snip-sync/src/lib.rs:470-478`). The first
comma-separated element of the header is used, with any `:port` suffix
stripped (`strip_port`, `snip-sync/src/lib.rs:79`). `strip_port` never
panics on malformed input — it is attacker-influenced behind a trusted
proxy and is only ever used as a bucket key. An untrusted peer
that sends the header is ignored, and a request with no peer address
falls back to the literal key `"unknown"`.

### Window mechanics

- Epoch-second sliding windows: the window resets when
  `now - window_start >= window_secs`, and the counter restarts at zero
  (`snip-sync/src/rate_limiter.rs:46-102`).
- The map is bounded at `MAX_ENTRIES = 100_000`
  (`snip-sync/src/rate_limiter.rs:8`); the oldest entry is evicted when a
  new key arrives at the cap.
- The limiter is **process-local**: state is in-memory only and resets on
  restart (`snip-sync/src/rate_limiter.rs:24-34`).

## Metrics

**File**: `snip-sync/src/metrics.rs`. A `prometheus::Registry` with six
instruments:

| Metric | Type | Labels | Incremented at |
|--------|------|--------|----------------|
| `snip_sync_requests_total` | CounterVec | `method` | `SnipSyncService::record_request` (`snip-sync/src/lib.rs:649-653`) |
| `snip_sync_request_duration_seconds` | HistogramVec | `method` | `record_request_duration` (`snip-sync/src/lib.rs:710-717`) |
| `snip_sync_sync_operations_total` | CounterVec | `direction` | `record_sync` (`snip-sync/src/lib.rs:735-740`) |
| `snip_sync_library_operations_total` | CounterVec | `operation` | `record_library_op` (`snip-sync/src/lib.rs:742-747`) |
| `snip_sync_rate_limit_hits_total` | Counter | — | `record_rate_limit` (`snip-sync/src/lib.rs:719-721`) |
| `snip_sync_auth_failures_total` | Counter | — | `record_auth_failure` (`snip-sync/src/lib.rs:723-733`) **and** the HTTP Basic-auth failure path (`snip-sync/src/http.rs:181`) |

`Metrics::new()` failure falls back to `Metrics::fallback()`, an
isolated registry with placeholder instruments so the server keeps serving
(`snip-sync/src/metrics.rs:103-166`, wired at `snip-sync/src/main.rs:170-179`).

The `/metrics` endpoint is only enabled when **both** `METRICS_USERNAME`
and `METRICS_PASSWORD` are configured; a half-configured pair stays
disabled and logs a warning (`snip-sync/src/main.rs:181-189`,
`snip-sync/src/http.rs:150-156`).

## TLS and certificate management

`snip-sync` never terminates TLS. It serves plaintext gRPC and HTTP and
expects an upstream reverse proxy to terminate TLS; `TLS_ENABLED=true`
records that acknowledgement (`snip-sync/src/main.rs:59-79`).

`snip-sync cert` / `snip-sync init` generate a **development** self-signed
pair via external `openssl req -x509 -newkey rsa:4096 -nodes -days 365
-subj /CN=localhost -addext subjectAltName=DNS:localhost,IP:127.0.0.1`
(`snip-sync/src/cert.rs:35-53`). The files (`key.pem`, `cert.pem`) are
for a TLS-terminating proxy to consume; the server does not read them.

### Regeneration: stage, then restore on any failure

Regeneration with `--force` previously deleted the old pair first and
removed the newly-written key when the second rename failed — leaving the
directory with *neither* key nor cert, strictly worse than the working
material the flow is meant to protect, and contradicting the comment
promising to keep it.

The current flow stages the old pair as sibling backups and restores it
on any failure (`snip-sync/src/cert.rs:93-143`):

1. Rename `key.pem` → `key.pem.backup.<uuid>`; on failure, restore and
   abort.
2. Rename `cert.pem` → `cert.pem.backup.<uuid>`; on failure, restore the
   key backup and abort.
3. Rename the new temp key into place; on failure, restore both backups
   and abort.
4. Rename the new temp cert into place; on failure, remove the new key,
   restore both backups, and abort.
5. Only when both files are in place are the backups removed.

Both renames stay inside the certificate directory, so staging through
sibling backup names is cheap.

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
max_encrypted_payload_length = 1048576   # or MAX_ENCRYPTED_PAYLOAD_LENGTH (default 1 MiB)
max_description_length = 1024
max_tags = 50
max_tag_length = 100
max_id_length = 128
max_device_id_length = 128
max_api_key_length = 512
request_timeout_secs = 30
grpc_max_message_size = 4194304

[server.rate_limit]
requests_per_minute = 120
# trusted_proxies = ["127.0.0.1"]        # or TRUSTED_PROXIES

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
  rejected with `InvalidRange` (`snip-sync/src/lib.rs:499-599`).
- **Fail-closed**: a present-but-malformed/unreadable `config.toml`
  makes `serve` and `croncheck` error identifying the path; compiled
  defaults apply only when the file is absent. `ensure_config_file()`
  creates it with `create_new` (no clobber) and `0o600` on Unix; a
  world-readable file holding metrics credentials logs a warning
  (`snip-sync/src/lib.rs:336-356`).
- `CORS_ALLOW_ALL` is environment-only (no TOML key) and is ignored on a
  non-loopback HTTP bind (`snip-sync/src/main.rs:228-242`).
- `max_sync_snippets` is **not** configurable; the 10,000-record batch
  ceiling is the compile-time constant `DEFAULT_MAX_SYNC_SNIPPETS`.

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
gate (`snip-sync/src/update.rs:8-13`). Do not consolidate without
re-running that measurement. `tests/architecture.rs` pins the split; see
[update.md](update.md) for the client-side transport.

## Key Files

Line counts at time of writing (`wc -l`):

| File | Role | Lines |
|------|------|-------|
| `snip-sync/src/lib.rs` | Service, config, validation, 11 RPCs, `UpsertOutcome` dispatch | 3064 |
| `snip-sync/src/startup.rs` | Supervisors, transport policy, restart | 1202 |
| `snip-sync/src/db.rs` | Schema, auth, CRUD, migration, upsert classification | 1357 |
| `snip-sync/src/orchestration.rs` | Typed shutdown + tests | 894 |
| `snip-sync/src/update.rs` | curl updater, Plan 017 gate note | 778 |
| `snip-sync/src/main.rs` | Dispatch, serve wiring, stop/restart | 719 |
| `snip-sync/src/premade.rs` | Library scan and safe fetch | 535 |
| `snip-sync/src/process.rs` | PID fallback, identity, signaling | 479 |
| `snip-sync/src/server_lock.rs` | Kernel singleton lock | 460 |
| `snip-sync/src/test_observer.rs` | Sanitized telemetry (test-only) | 413 |
| `snip-sync/src/http.rs` | Two-route EggServe service | 230 |
| `snip-sync/src/cli.rs` | Subcommands and flags | 206 |
| `snip-sync/src/cert.rs` | Dev certs via `openssl`, backup/restore regeneration | 187 |
| `snip-sync/src/editor.rs` | `$EDITOR` handling for `edit` | 166 |
| `snip-sync/src/rate_limiter.rs` | Sliding-window limiter | 159 |
| `snip-sync/src/paths.rs` | XDG-ish path resolution | 144 |
| `snip-sync/src/metrics.rs` | Prometheus instruments | 128 |
| `snip-sync/src/bootstrap.rs` | Layout + default config | 119 |
| `snip-sync/src/test_helpers.rs` | In-process test server | 110 |
| `snip-sync/Cargo.toml` | Pinned EggServe/tonic/sqlx deps | 75 |
| `snip-proto/proto/sync.proto` | Wire contract (see [proto.md](proto.md)) | 222 |
| `tests/snip_sync_lifetime.rs` | Socket wire-parity + lifetime gates | 840 |

## Security properties

- Argon2id (16 MiB, 3 iterations, parallelism 4) with per-key salt; only
  the 8-char SHA-256 prefix is indexed; plaintext migration is atomic at
  startup and fail-closed.
- Rate limiting: 120 requests/minute default, keyed on the **peer
  address** for every RPC (never on the request-supplied API key);
  `x-forwarded-for` honoured only from a configured trusted proxy;
  sliding epoch-second windows, bounded at 100,000 entries, process-local
  (resets on restart).
- Auth metadata is never retained in production state. Bearer capture
  (`captured_auth_header`), the sanitized `test_observer`, and push-fault
  injection exist only under `test-helpers` / `#[cfg(test)]`.
- Metrics endpoint requires both username and password over HTTP Basic
  auth with constant-time comparison and a case-insensitive scheme match;
  half-configured metrics stay disabled (`404`), never half-open. A `401`
  carries a `WWW-Authenticate` challenge and increments
  `snip_sync_auth_failures_total`.
- Plaintext serve is loopback-only unless a proxy terminates TLS
  (`TLS_ENABLED=true`); config files default to `0o600` with a
  world-readable warning when they hold credentials.
- Premade filenames are allowlisted (`alphanumeric/-/_/.`, ≤64 chars,
  no `..`/`/` `\`) at the handler layer
  (`snip-sync/src/lib.rs:1896-1918`) and resolved via
  canonicalize-plus-prefix check in the manager
  (`snip-sync/src/premade.rs:268-283`); error paths never leak DB
  internals to clients.
- Foreign keys are enforced on **every** pooled connection via
  `after_connect`, not just one.
- Certificate regeneration restores the previous working pair on any
  failure instead of leaving the directory empty.

## Invariants and Gotchas

- **`snippets.id` is global.** Two libraries of one install that hold
  identical content produce identical deterministic ids, so the second
  push collides with the first. The collision is now surfaced
  (`rejected_count` in `PushSnippets`, `skipped_ids` in `Sync`), never
  silently dropped. Do not "fix" this by treating a zero-row upsert as
  success — that is the exact bug the `UpsertOutcome` split exists to
  prevent.
- **`ON CONFLICT … DO UPDATE … WHERE` that evaluates false is a silent
  no-op** in SQLite: 0 rows affected, no error. Any code path that
  infers success from "no error" is wrong here; infer it from
  `rows_affected()` plus a scope probe.
- **The server's merge rule is not the client's.** The upsert tie-break is
  a lexicographic last-write-wins on `(updated_at, device_id, command,
  description)` with no SHA-256, and `deleted` is an ordinary column, so a
  newer live row overwrites a tombstone. Do not assume the server
  guarantees no resurrection — that guarantee is the client's
  (`choose_version`, `src/sync_commands.rs:1199`).
- **`PRAGMA foreign_keys` is per-connection.** Setting it once against a
  pool sets it on one connection. Use `after_connect`.
- **Do not add a second "listening" log after
  `serve_with_incoming_shutdown()`.** The bind-time log is the startup
  signal; anything after the serve expression runs at shutdown time.
- **Rate-limit buckets are per peer address.** Keying them on anything
  request-supplied (API key, device id) removes the aggregate ceiling
  entirely.
- **Download paging must be bounded by encoded bytes, not row count.**
  A page that fits the row limit but not the message ceiling returns
  `OUT_OF_RANGE`, which the client does not retry — the library becomes
  permanently undownloadable. `fit_page_to_byte_ceiling()` keeps at
  least one row and sets `has_more` so the watermark is not advanced.
- **The `skipped_ids` watermark rule**: while `has_more` is true the
  server returns the *request's* `last_sync_timestamp`, not the newest
  row's timestamp, so the client can fetch the remaining pages.
- **`snip-sync` must not terminate TLS**, and must not grow an in-process
  HTTP client. The `curl` transport is deliberate; the size gate is
  pinned by `tests/architecture.rs`.
- **No Axum, no Tower-HTTP, no generic router/middleware.** The HTTP
  surface is one concrete two-route EggServe leaf on its own listener,
  separate from the Tonic listener. Any change to status codes, `Allow` /
  `Vary`, or the security headers must update `tests/snip_sync_lifetime.rs`
  in the same change.
