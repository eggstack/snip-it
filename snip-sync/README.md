# snip-sync Server

A gRPC server for syncing snippets between clients.

> **For production, terminate TLS at a reverse proxy** (nginx, Caddy,
> traefik) with a real certificate (Let's Encrypt, etc.). The server
> itself speaks plain gRPC; TLS is delegated to the proxy. See the
> [Production Deployment](#production-deployment) section.

## Quick Start

```bash
# Build the server
cd snip-sync
cargo build --release

# Run with default config (127.0.0.1:50051 for gRPC, 127.0.0.1:50050 for HTTP)
./target/release/snip-sync
```

A complete annotated configuration is at `config.example.toml` — copy
it to `config.toml` and edit as needed:

```bash
cp config.example.toml config.toml
$EDITOR config.toml
```

## Configuration

The server looks for `config.toml` in the current working directory. If not found, a default config is created.

### Config File Location

- Working directory: `./config.toml`
- Or set via: `CONFIG_PATH=/path/to/config.toml`

### Generating a Local TLS Certificate

For local development, generate a self-signed certificate with:

```bash
./scripts/gen-dev-cert.sh ./certs
```

This writes `./certs/cert.pem` and `./certs/key.pem` (mode 600).
**Do not** ship self-signed certs to production; use Let's Encrypt or
your organization's CA instead.

### Configuration Options

```toml
[server]
grpc_host = "127.0.0.1"   # gRPC server host
grpc_port = 50051          # gRPC server port
http_host = "127.0.0.1"   # HTTP server host (for metrics)
http_port = 50050          # HTTP server port

[server.database]
path = "snippets.db"      # SQLite database path

[server.premade]
directory = "premade-libraries"  # Premade libraries directory

[server.limits]
max_command_length = 1024
max_description_length = 1024
max_tags = 50
max_tag_length = 100
request_timeout_secs = 30

[server.rate_limit]
requests_per_minute = 120

[server.metrics]
# username = "admin"      # Uncomment to enable metrics endpoint
# password = "metrics"

[server.cors]
allowed_origins = ""  # Comma-separated, empty = CORS disabled, "*" = allow all
```

### Environment Variables

Environment variables override config file settings:

| Variable | Description |
|----------|-------------|
| `GRPC_HOST` | gRPC server host |
| `GRPC_PORT` | gRPC server port |
| `HTTP_HOST` | HTTP server host |
| `HTTP_PORT` | HTTP server port |
| `DATABASE_URL` | Database path |
| `PREMADE_DIR` | Premade libraries directory |
| `CONFIG_PATH` | Custom config file path |
| `RATE_LIMIT_PER_MINUTE` | Rate limit per API key |
| `METRICS_USERNAME` | Metrics basic auth username |
| `METRICS_PASSWORD` | Metrics basic auth password |
| `CORS_ALLOWED_ORIGINS` | Comma-separated CORS origins |

## Ports

- **gRPC (50051)**: Main sync API
- **HTTP (50050)**: Prometheus metrics endpoint

## Metrics

When enabled (by setting `username` and `password` in the config), metrics are available at the HTTP endpoint with basic authentication:

```bash
curl -u admin:metrics http://127.0.0.1:50050/metrics
```

## Premade Libraries

The server can serve pre-built snippet libraries to clients. Place `.toml` files in the `premade-libraries` directory.

### Format

```toml
Description = "Library description here"

[[Snippets]]
  Description = "Snippet description"
  Tag = ["tag1", "tag2"]
  command = "the command"
```

## Production Deployment

For production, use a reverse proxy with TLS termination (nginx, traefik, etc.):

### nginx example

```nginx
server {
    listen 443 ssl;
    server_name snip.yourdomain.com;
    
    ssl_certificate /etc/letsencrypt/live/snip/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/snip/privkey.pem;
    
    location / {
        grpc_pass 127.0.0.1:50051;
    }
}
```

### systemd service

Create `/etc/systemd/system/snip-sync.service`:

```ini
[Unit]
Description=snip-sync gRPC server
After=network.target

[Service]
Type=simple
User=snip
WorkingDirectory=/opt/snip-sync
ExecStart=/opt/snip-sync/snip-sync
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Then:
```bash
sudo systemctl daemon-reload
sudo systemctl enable snip-sync
sudo systemctl start snip-sync
```
