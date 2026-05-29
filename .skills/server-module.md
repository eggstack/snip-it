# Server Module Skill

## Purpose
Guide agents through working with the snip-sync server (`snip-sync/src/`).

## Security Notes

- **TLS**: Server defaults to HTTP. Production deployments must use a reverse proxy with TLS. Documentation at startup and in `config.rs` notes this requirement.
- **CORS**: `CORS_ALLOW_ALL=true` env var enables permissive CORS. When not set and no origins configured, cross-origin requests are blocked.
- **Rate limiting**: All endpoints use `authenticate_and_rate_limit()` helper. Registration rate limits use IP address (not client-controlled device_id).
- **Argon2**: Memory cost is `1 << 14` (16 MiB) in `snip-sync/src/db.rs:12`.

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
| `CORS_ALLOW_ALL` | `false` | Set to `true` or `1` to allow all origins |
| `METRICS_USERNAME` | empty | Basic auth for /metrics |
| `METRICS_PASSWORD` | empty | Basic auth for /metrics |
| `RUST_LOG` | `info` | Log level |

## gRPC Endpoints

| RPC | Auth | Rate Limited | Description |
|-----|------|-------------|-------------|
| Health | No | No | Server health check |
| Register | No | Yes (by IP address) | Create user + API key |
| GetSnippets | Yes | Yes | Fetch snippets for library |
| PushSnippets | Yes | Yes | Upload snippets to server |
| Sync | Yes | Yes | Full bidirectional sync |
| CreateLibrary | Yes | Yes | Create new library |
| ListLibraries | Yes | Yes | List user's libraries |
| DeleteLibrary | Yes | Yes | Soft-delete library |
| ListPremadeLibraries | Yes | Yes | Browse premade catalog |
| GetPremadeLibrary | Yes | Yes | Download premade library |

## Testing

Server tests use `sqlite::memory:` for isolation. Run with:
```bash
cargo test -p snip-sync
```

Current coverage: 15 unit tests in `db.rs` only. No tests for gRPC handlers, HTTP endpoints, rate limiter, or premade manager.
