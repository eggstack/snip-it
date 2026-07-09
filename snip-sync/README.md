# snip-sync Server

A gRPC server for syncing snippets between clients.

> **For production, terminate TLS at a reverse proxy** (nginx, Caddy,
> traefik) with a real certificate (Let's Encrypt, etc.). The server
> itself speaks plain gRPC; TLS is delegated to the proxy.

## Quick Install

```bash
cargo install snip-sync
snip-sync init
snip-sync edit
SNIP_SYNC_ALLOW_HTTP=true snip-sync serve
```

## First-Run Setup

`snip-sync init` creates:
- Config file at `~/.config/snip-sync/config.toml`
- Dev certificates at `~/.config/snip-sync/certs/`
- Required directories (data, state, premade)

```bash
snip-sync init              # create config + certs
snip-sync init --skip-cert  # skip cert generation
snip-sync init --force-cert # regenerate certs
```

## Configuration

Default config path: `~/.config/snip-sync/config.toml`
Override: `CONFIG_PATH=/path/to/config.toml`

Edit with: `snip-sync edit`

### Config File

```toml
[server]
grpc_host = "127.0.0.1"
grpc_port = 50051
http_host = "127.0.0.1"
http_port = 50050

[server.database]
# path = "snippets.db"  # defaults to ~/.config/snip-sync/snippets.db

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
# username = "admin"
# password = "metrics"

[server.cors]
allowed_origins = ""
```

### Environment Variables

| Variable | Description |
|----------|-------------|
| `GRPC_HOST` / `GRPC_PORT` | gRPC server bind |
| `HTTP_HOST` / `HTTP_PORT` | HTTP server bind |
| `DATABASE_URL` | Database path |
| `PREMADE_DIR` | Premade libraries dir |
| `CONFIG_PATH` | Config file path |
| `TLS_ENABLED` | Set `true` when TLS is handled by reverse proxy |
| `SNIP_SYNC_ALLOW_HTTP` | Set `true` for local plaintext dev |
| `METRICS_USERNAME` / `METRICS_PASSWORD` | Metrics auth |

## Dev Certificates

```bash
snip-sync cert              # generate in ~/.config/snip-sync/certs/
snip-sync cert --out-dir ./certs  # custom location
snip-sync cert --force      # overwrite existing
```

Generated certs are self-signed (CN=localhost, SANs=localhost+127.0.0.1).
Use them with a reverse proxy, NOT directly with snip-sync.

## Service Management

### systemd (recommended)

```ini
# /etc/systemd/system/snip-sync.service
[Unit]
Description=snip-sync gRPC server
After=network.target

[Service]
Type=simple
ExecStart=/home/user/.cargo/bin/snip-sync serve
Environment=TLS_ENABLED=true
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable snip-sync
sudo systemctl start snip-sync
```

### cron (lightweight fallback)

```
@reboot /home/user/.cargo/bin/snip-sync croncheck
*/5 * * * * /home/user/.cargo/bin/snip-sync croncheck
```

`croncheck` health-checks the server; if down, spawns a detached `snip-sync serve`.
Use systemd for production — croncheck is a lightweight fallback.

## Paths

```bash
snip-sync paths           # human-readable
snip-sync paths --json    # machine-readable
```

## Updating

```bash
snip-sync update              # cargo install snip-sync
snip-sync update --dry-run    # preview
snip-sync update --locked     # with lockfile
```

## Source Build (contributors)

```bash
git clone https://github.com/eggstack/snip-it.git
cd snip-sync
cargo build --release
./target/release/snip-sync --help
```

## Docker

```bash
docker pull ghcr.io/eggstack/snip-it/snip-sync:latest
```

## Troubleshooting

- **Config not found**: Run `snip-sync init` first
- **TLS required**: Set `SNIP_SYNC_ALLOW_HTTP=true` for local dev
- **Port in use**: Check `snip-sync paths` for configured ports
- **Server won't stop**: Use `snip-sync stop --force` or `kill -9 <pid>`
