# Server Module Skill

## Purpose
Guide agents through working with the snip-sync server (`snip-sync/src/`).

## Critical Security Issues

### 1. CORS Misconfiguration
**Location**: `snip-sync/src/main.rs:998-1003`

When `cors_allowed_origins` is empty (default), `CorsLayer::new()` is used with NO allow-origin rules, which **blocks all cross-origin requests**. The config comment says "Leave empty to allow all origins" — this is wrong.

**Fix**: Either use `CorsLayer::very_permissive()` when origins is empty, or update the config comment to reflect deny-all behavior.

### 2. Registration Rate Limit Bypassable
**Location**: `snip-sync/src/main.rs:330-337`

Rate limiting uses `req.device_id` which is client-controlled. A client can rotate `device_id` to bypass rate limiting entirely.

**Fix**: Rate limit by IP address or a server-generated token, not client-provided `device_id`.

### 3. No TLS
**Location**: `snip-sync/src/main.rs:920-922`

API keys are transmitted in plaintext over gRPC. Production deployments must use a reverse proxy with TLS.

### 4. Argon2 Memory Cost Too Low
**Location**: `snip-sync/src/db.rs:12`

`ARGON2_MEMORY_KIB = 1 << 6 = 64 KiB`. OWASP minimum is 19 MiB (19456 KiB). Makes API keys easier to brute-force if database is compromised.

## Server Architecture

```
snip-sync/
├── src/main.rs         # gRPC + HTTP server entry
│   ├── SnipSyncService # gRPC service implementation
│   ├── axum routes     # /health (no auth), /metrics (Basic auth)
│   └── Config          # Loaded from config.toml, overridable by env vars
├── src/db.rs           # SQLite via sqlx
│   ├── users table     # id, api_key, api_key_prefix, created_at, updated_at
│   ├── libraries table # id, name, user_id, created_at, deleted_at
│   └── snippets table  # id, library_id, content, encrypted, created_at, updated_at, deleted
├── src/rate_limiter.rs # Per-key sliding window (120 req/min default)
├── src/metrics.rs      # Prometheus counters
└── src/premade.rs      # Premade library file scanning with path traversal prevention
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `LISTEN_ADDR` | `0.0.0.0:50051` | gRPC listen address |
| `HTTP_ADDR` | `0.0.0.0:50050` | HTTP listen address |
| `DATABASE_URL` | `snip_sync.db` | SQLite database path |
| `CORS_ALLOWED_ORIGINS` | empty (deny-all) | Comma-separated origins |
| `METRICS_USERNAME` | empty | Basic auth for /metrics |
| `METRICS_PASSWORD` | empty | Basic auth for /metrics |
| `RUST_LOG` | `info` | Log level |

## gRPC Endpoints

| RPC | Auth | Rate Limited | Description |
|-----|------|-------------|-------------|
| Health | No | No | Server health check |
| Register | No | Yes (by device_id — bypassable) | Create user + API key |
| GetSnippets | Yes | No | Fetch snippets for library |
| PushSnippets | Yes | Yes | Upload snippets to server |
| Sync | Yes | Yes | Full bidirectional sync |
| CreateLibrary | Yes | Yes | Create new library |
| ListLibraries | Yes | No | List user's libraries |
| DeleteLibrary | Yes | Yes | Soft-delete library |
| ListPremadeLibraries | Yes | Yes | Browse premade catalog |
| GetPremadeLibrary | Yes | Yes | Download premade library |

## Dead Code

- `verify_snippet_ownership` in `db.rs:374-388` — unused
- `DbError::Unauthorized` in `db.rs:22-23` — never constructed
- `record_request` `_method` parameter in `main.rs:255` — ignored

## Testing

Server tests use `sqlite::memory:` for isolation. Run with:
```bash
cargo test -p snip-sync
```

Current coverage: 15 unit tests in `db.rs` only. No tests for gRPC handlers, HTTP endpoints, rate limiter, or premade manager.
