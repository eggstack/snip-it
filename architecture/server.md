# Server (snip-sync)

[← Back to Overview](overview.md)

## Overview

**Directory**: `snip-sync/`

A standalone gRPC + HTTP server that stores encrypted snippets, manages users/libraries, and serves premade libraries.

## Architecture

```
┌──────────────────────────────────────────┐
│              snip-sync                    │
│                                          │
│  ┌─────────────┐    ┌─────────────────┐  │
│  │ gRPC Server │    │ HTTP Server     │  │
│  │ (tonic)     │    │ (axum)          │  │
│  │ :50051      │    │ :50050          │  │
│  └──────┬──────┘    └───────┬─────────┘  │
│         │                   │             │
│  ┌──────┴───────────────────┴─────────┐  │
│  │        SnipSyncService             │  │
│  │  ┌──────────┐  ┌──────────────┐    │  │
│  │  │ Database │  │ Rate Limiter │    │  │
│  │  │ (SQLite) │  │ (in-memory)  │    │  │
│  │  └──────────┘  └──────────────┘    │  │
│  │  ┌──────────┐  ┌──────────────┐    │  │
│  │  │ Metrics  │  │ Premade Mgr  │    │  │
│  │  │(Prometheus│  │ (file scan)  │    │  │
│  │  └──────────┘  └──────────────┘    │  │
│  └────────────────────────────────────┘  │
└──────────────────────────────────────────┘
```

## Database

**File**: `snip-sync/src/db.rs` (~1000 lines)

SQLite via `sqlx` with in-memory support for tests.

### Tables

| Table | Columns |
|-------|---------|
| `users` | `id`, `api_key_hash`, `api_key_prefix`, `device_id`, `created_at` |
| `libraries` | `id`, `user_id`, `name`, `created_at` |
| `snippets` | `id`, `user_id`, `library_id`, `description`, `command`, `tags`, `created_at`, `updated_at`, `device_id`, `deleted`, `encrypted` |

### API Key Hashing

- **Storage**: Argon2id hash of API key
- **Lookup**: SHA-256 prefix (first 8 chars of base64) for indexed lookup
- **Migration**: `migrate_plaintext_api_keys()` backfills hashes for legacy data

### Key Operations

| Operation | Description |
|-----------|-------------|
| `create_user(api_key)` | Hash key, generate device_id, insert user |
| `get_user_by_api_key(key)` | Verify hash, return user |
| `upsert_snippet(snippet, user_id, library_id)` | Insert or update snippet |
| `get_snippets(user_id, library_id, since, limit, offset)` | Paginated snippet retrieval |
| `get_latest_timestamp(user_id, library_id)` | Latest `updated_at` for sync |
| `create_library(user_id, name)` | Create library for user |
| `list_libraries(user_id, limit, offset)` | Paginated library listing |
| `delete_library(user_id, library_id)` | Soft-delete library |
| `verify_library_ownership(user_id, library_id)` | Authorization check |

## gRPC Service

**File**: `snip-sync/src/main.rs`

Implements `SnippetSync` trait from `snip-proto`:

| RPC | Description |
|-----|-------------|
| `Health` | Returns version + healthy status |
| `Register` | Create user, return API key + device_id |
| `GetSnippets` | Paginated snippet retrieval |
| `PushSnippets` | Bulk insert/update snippets |
| `Sync` | Bidirectional sync (upsert local, return server snippets) |
| `CreateLibrary` | Create library |
| `ListLibraries` | List user's libraries |
| `DeleteLibrary` | Delete library |
| `ListPremadeLibraries` | List available premade libraries |
| `GetPremadeLibrary` | Download premade library content |

### Input Validation

- Max command length: 1024 bytes
- Max description length: 1024 bytes
- Max tags: 50
- Max tag length: 100 bytes
- Request timeout: 30s (configurable)

## HTTP Server

**File**: `snip-sync/src/main.rs`

Axum-based HTTP server with two endpoints:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | JSON health check (no auth) |
| `/metrics` | GET | Prometheus metrics (Basic auth required) |

### CORS

Configurable via `CORS_ALLOWED_ORIGINS` env var or config file. Supports multiple comma-separated origins.

## Rate Limiter

**File**: `snip-sync/src/rate_limiter.rs` (47 lines)

In-memory per-key rate limiting:
- Default: 120 requests/minute per API key
- Sliding window based on `Instant`
- Background cleanup task runs every 60s

## Metrics

**File**: `snip-sync/src/metrics.rs` (67 lines)

Prometheus counters:

| Metric | Description |
|--------|-------------|
| `snip_sync_requests_total` | Total requests |
| `snip_sync_sync_operations_total` | Sync operations |
| `snip_sync_library_operations_total` | Library CRUD operations |
| `snip_sync_rate_limit_hits_total` | Rate limit rejections |
| `snip_sync_auth_failures_total` | Authentication failures |

Protected by HTTP Basic auth (`METRICS_USERNAME`/`METRICS_PASSWORD`).

## Premade Manager

**File**: `snip-sync/src/premade.rs` (214 lines)

Scans a directory for `.toml` snippet library files:
- Reads and parses each file
- Extracts snippet count and description
- Serves content via gRPC
- Path traversal prevention (canonicalize + prefix check)

## Configuration

**File**: `snip-sync/src/main.rs`

Config loaded from `config.toml` (or `CONFIG_PATH` env var):

```toml
[server]
grpc_host = "127.0.0.1"
grpc_port = 50051
http_host = "127.0.0.1"
http_port = 50050

[server.database]
path = "snippets.db"

[server.premade]
directory = "premade-libraries"

[server.limits]
max_command_length = 1024
max_description_length = 1024
max_tags = 50
max_tag_length = 100
request_timeout_secs = 30

[server.rate_limit]
requests_per_minute = 120

[server.metrics]
username = "admin"
password = "secret"

[server.cors]
allowed_origins = "https://example.com"
```

All values can be overridden via environment variables.

## Key Files

- `snip-sync/src/main.rs` — Server entry, gRPC/HTTP setup, config loading
- `snip-sync/src/db.rs` — SQLite database, user/snippet/library operations
- `snip-sync/src/rate_limiter.rs` — Per-key rate limiting
- `snip-sync/src/metrics.rs` — Prometheus counters
- `snip-sync/src/premade.rs` — Premade library file scanning
- `snip-sync/Cargo.toml` — Dependencies
- `snip-sync/config.toml` — Default config template
- `snip-sync/Dockerfile` — Container build
- `snip-sync/docker-compose.yml` — Local development setup
