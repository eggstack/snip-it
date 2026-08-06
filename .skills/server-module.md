# Server Module Skill

## Purpose
Guide agents through working with the snip-sync server (`snip-sync/src/`).

## Security Notes

- **TLS**: Server defaults to HTTP. Production deployments must use a reverse proxy with TLS. `TLS_ENABLED` env var available for native TLS. Documentation at startup and in `config.rs` notes this requirement.
- **CORS**: `CORS_ALLOW_ALL=true` env var enables permissive CORS. When not set and no origins configured, cross-origin requests are blocked.
- **Rate limiting**: All endpoints use `authenticate_and_rate_limit()` helper. Registration rate limits use IP address (not client-controlled device_id). `RATE_LIMIT_PER_MINUTE` controls limit.
- **Argon2**: Memory cost is `1 << 14` (16 MiB) in `snip-sync/src/db.rs`.

## Server Architecture

```
snip-sync/
├── src/main.rs         # gRPC + HTTP server entry, CLI dispatch
├── src/lib.rs          # Config loading, SnipSyncService, axum routes
├── src/db.rs           # SQLite via sqlx (18 tests)
├── src/rate_limiter.rs # Per-key sliding window (120 req/min default)
├── src/metrics.rs      # Prometheus counters
├── src/premade.rs      # Premade library file scanning with path traversal prevention
├── src/bootstrap.rs    # Server initialization and service wiring
├── src/paths.rs        # Default path resolution
├── src/cert.rs         # TLS certificate handling
├── src/cli.rs          # CLI argument parsing
├── src/editor.rs       # Editor integration
├── src/process.rs      # Process management
├── src/server_lock.rs  # Kernel-backed server singleton lock (flock/LockFileEx)
├── src/test_helpers.rs # In-process test server support
├── src/test_observer.rs# Test-only request telemetry (no secrets)
└── src/update.rs       # Server self-update
```

### Process lifecycle

The server singleton uses a kernel-backed lock held for the full `serve`
runtime. `stop` and `restart` understand structured and legacy numeric PID
records on Unix; stale legacy records are cleaned only after lock acquisition,
while live unrelated processes are refused unless `--force` is explicit.
Persistent lock-file presence is not an ownership signal.

### Shutdown Orchestration (Phase 13H + Phase 13J)

- Shutdown coordination lives in a shared `run_services_until_shutdown()` helper in `orchestration.rs`.
- `serve_inner` and deterministic tests call the same helper — production and test paths are identical.
- Both service task handles are awaited by mutable reference; a completed handle is never polled twice.
- On timeout (default 30s graceful drain), unfinished handles are explicitly aborted and awaited — they are not silently dropped.
- `serve_inner` evaluates `ServiceShutdownOutcome::ensure_clean_requested_shutdown()` after persistence cleanup. A requested shutdown returns success only when both services returned cleanly without forced abort; any drain-time service error, panic, or forced abort produces a failure that retains both classifications and the original detail.
- `state_dir()` supports `SNIP_SYNC_STATE_DIR` env var override for test isolation.

## Environment Variables

All environment variable overrides are strictly parsed. Missing variables fall
back to file/default configuration. Present but invalid values cause startup to
fail with an error naming the variable and the supplied value. Boolean
variables (`TLS_ENABLED`, `SNIP_SYNC_ALLOW_HTTP`, `CORS_ALLOW_ALL`,
`PERSIST_RATE_LIMITS`) accept case-insensitive `true`, `1`, `yes`, `on` and
`false`, `0`, `no`, `off`; unknown values fail instead of silently falling
back.

| Variable | Default | Description |
|----------|---------|-------------|
| `GRPC_HOST` | `127.0.0.1` | gRPC listen host |
| `GRPC_PORT` | `50051` | gRPC listen port (must be nonzero) |
| `HTTP_HOST` | `127.0.0.1` | HTTP listen host |
| `HTTP_PORT` | `50050` | HTTP listen port (must be nonzero) |
| `DATABASE_URL` | `snip_sync.db` | SQLite database path |
| `DB_MAX_CONNECTIONS` | 10 | Max SQLite connections (must be ≥ 1) |
| `PREMADE_DIR` | `./premade` | Premade library directory |
| `CORS_ALLOWED_ORIGINS` | empty (deny-all) | Comma-separated origins |
| `CORS_ALLOW_ALL` | `false` | Boolean: `true`/`1`/`yes`/`on` to allow all origins |
| `METRICS_USERNAME` | empty | Basic auth for /metrics |
| `METRICS_PASSWORD` | empty | Basic auth for /metrics |
| `RATE_LIMIT_PER_MINUTE` | 120 | Requests per minute per API key |
| `TRUSTED_PROXIES` | empty | Comma-separated trusted proxy IPs |
| `PERSIST_RATE_LIMITS` | `false` | Boolean: `true`/`1`/`yes`/`on` to persist rate limits to SQLite |
| `TLS_ENABLED` | `false` | Boolean: `true`/`1`/`yes`/`on` to acknowledge TLS termination |
| `SNIP_SYNC_ALLOW_HTTP` | `false` | Boolean: `true`/`1`/`yes`/`on` to allow plaintext HTTP (loopback only) |
| `RUST_LOG` | `info` | Log level (via tracing) |

## gRPC Endpoints

| RPC | Auth | Rate Limited | Description |
|-----|------|-------------|-------------|
| Health | No | No | Server health check (verifies DB connectivity) |
| Register | No | Yes (by IP address) | Create user + API key |
| GetSnippets | Yes | Yes | Fetch snippets for library |
| PushSnippets | Yes | Yes | Upload snippets to server |
| Sync | Yes | Yes | Full bidirectional sync |
| CreateLibrary | Yes | Yes | Create new library |
| ListLibraries | Yes | Yes | List user's libraries |
| DeleteLibrary | Yes | Yes | Soft-delete library |
| ListPremadeLibraries | Yes | Yes | Browse premade catalog |
| GetPremadeLibrary | Yes | Yes | Download premade library |
| SearchPremadeLibraries | Yes | Yes | Search premade libraries |

## Testing

Server tests use `sqlite::memory:` for isolation. Run with:
```bash
cargo test -p snip-sync
```

For in-process test server support, use the `test-helpers` feature:
```bash
cargo test -p snip-sync --features test-helpers
```

Current coverage: 18 tests in `db.rs` plus integration tests via `test-helpers` feature.
