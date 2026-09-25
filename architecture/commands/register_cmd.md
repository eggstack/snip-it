# register_cmd — Device Registration with Sync Server

[← Back to Overview](../overview.md)

## Overview

`src/commands/register_cmd.rs` (79 lines) registers this device with a
snip-sync server over gRPC, persists `SyncSettings` (enabled, URL,
device ID, key, `credential_revision=1`), and prints a masked key.
Idempotent without `--force`.

## CLI surface

`RegisterArgs` (`register_cmd.rs:6`), `snp register` (alias `reg`):
`--server URL` (default `DEFAULT_SERVER_URL`), `--force` (re-register
even when a device ID exists). Requires the global Tokio `RUNTIME`
(`main.rs:726-728`).

## Flow / steps

`run(server, force, runtime)` (`register_cmd.rs:14`):

1. Unless `force`, `load_sync_settings` with a non-empty `device_id`
   short-circuits: print device ID + config path + `--force` hint,
   return `Ok`.
2. Resolve URL: explicit non-default flag wins; else the saved
   `server_url` if non-empty; else the flag/default. (A saved custom
   URL survives a bare `snp register --force`.)
3. `runtime.block_on(SyncClient::register(url))` → `(api_key,
   device_id)`; failure → `runtime_error("Registration failed")`.
4. Build `SyncSettings{enabled:true, server_url, api_key, device_id,
   credential_revision:1}` and `save_sync_settings` (keychain when
   available; failure → stderr + propagate).
5. Print success, key masked as `prefix...suffix` (first/last 4 chars;
   `****` for short keys), keychain note, device ID, and config path.

## Mutation vs read-only

Mutating (config): writes `sync.toml` (+ keychain). No library writes,
so no transaction gate or local-data lock — sync-settings persistence
has its own CRC32-guarded atomic path (`config/sync_settings`).

## Auto-sync trigger

None. No `notify_mutation` — registration creates no snippet content.
`new_cmd` stamps the freshly registered `device_id` onto snippets
created afterwards; previously created snippets sync their stored IDs.

## Error / exit mapping

`SnipResult<()>`: registered-without-`--force`, disabled hints, and
save failures surface as described; gRPC failure maps to
`"Registration failed"` with the transport detail (exit 1; sync
failures downstream are exit 7). No `CliOutcome` — success is `Ok`.

## Key invariants

- Never log the full API key: masked display, `SnipError` carries no
  credentials, keychain preferred over `sync.toml` plaintext.
- `credential_revision=1` on fresh registration (rotation bumps it).
- Keep `keyring = "4"` default features or persistence silently falls
  back to the mock store (AGENTS.md).
- `SNP_ALLOW_PLAINTEXT_API_KEY=true` test seam must never be removed;
  tests never touch the real keychain.

## Keychain and settings persistence

`save_sync_settings` writes `sync.toml` (CRC32 integrity header,
`0600` via `SensitiveConfig` durability) and upserts the API key into
the OS keychain when available (`keyring`); on headless/CI stores the
key falls back per the `SNP_ALLOW_PLAINTEXT_API_KEY` seam. The printed
config path (`get_sync_config_path`) tells the operator exactly which
file was written. Re-registration preserves no prior fields — the
settings struct is rebuilt from `default()` plus the three fresh
values, so stale debounce/timeout customizations reset (re-apply via
`snp sync config` afterwards).

## Testing seams

Tests never touch the real keychain or network: `RecordingServer`
stands in for `SyncClient::register`, `TempDir + XDG_CONFIG_HOME`
isolates `sync.toml`, and `SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on
all test commands. `set_var` in tests needs `unsafe` (edition 2024).

## File / line references

- `RegisterArgs`: `src/commands/register_cmd.rs:6`; `run`: `:14`
- Guard: `:15-26`; URL precedence: `:28-38`; save: `:42-52`
- Client: `src/sync.rs` (`SyncClient::register`); settings:
  `src/config/sync_settings.rs`; dispatch: `src/main.rs:726-728`
