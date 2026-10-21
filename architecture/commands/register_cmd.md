# register_cmd — Device Registration with the Sync Server

[← Back to Overview](../overview.md)

## Overview

`src/commands/register_cmd.rs` (79 lines) registers this device with a
snip-sync server over gRPC, persists the resulting `SyncSettings`
(enabled, server URL, device ID, API key, `credential_revision = 1`), and
prints a masked key.

It does **not** create snippets, libraries, or sync traffic. It does not
call `notify_mutation` — registration creates no snippet content, so there
is no sync intent to schedule.

Related: [sync_cmd.md](sync_cmd.md) (the sync side of the credential),
[premade_cmd.md](premade_cmd.md) (the other consumer of the API key),
[../encryption.md](../encryption.md), [../config.md](../config.md).

## CLI surface

```rust
// src/commands/register_cmd.rs:5-11
#[derive(Debug, Clone, clap::Args)]
pub struct RegisterArgs {
    #[arg(long, default_value = crate::config::DEFAULT_SERVER_URL)]
    pub server: String,
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub force: bool,
}
```

`snp register` (alias `reg`) — `src/main.rs:143-145`, dispatched at
`src/main.rs:763-765`.

| Flag | Type | Default | Behavior |
|------|------|---------|----------|
| `--server <URL>` | `String` | `DEFAULT_SERVER_URL` (`http://localhost:50051`, `src/config/sync_settings.rs:24`) | Server to register against. An explicit non-default value overrides the saved URL |
| `--force` | bool | off | Re-register even when a device ID is already present |

`RegisterArgs` is the canonical args struct for this command (it is
declared in the command module, not inline in `main.rs`, unlike the `Sync`
group).

## Flow / steps

`run(server, force, runtime)` (`src/commands/register_cmd.rs:14`):

1. **Already registered?** Unless `--force`, `load_sync_settings()` is
   consulted; if it succeeds and `settings.device_id` is non-empty, print
   `Already registered! Device ID: <id>`, the config file path
   (`config::get_sync_config_path()`), and `Use --force to re-register.`,
   then return `Ok(())` — exit 0 (`:15-26`). A settings-load failure does
   **not** short-circuit: registration proceeds.
2. **Resolve the server URL** (`:28-38`):
   - an explicit `--server` that differs from `DEFAULT_SERVER_URL` wins;
   - otherwise the saved `settings.server_url` is used when it is non-empty;
   - otherwise the flag/default is used.

   A saved custom URL therefore survives a bare `snp register --force`.
3. **Register** — `runtime.block_on(SyncClient::register(server_url))`
   (`:40`). The client opens a TLS channel (a transport failure becomes
   `sync_failure(ConnectFailed)`) and issues the `Register` RPC through
   `retry_grpc_unified!` with `None::<SyncRunLimits>` (unbounded) and
   `RetryBackoff::Standard` (`src/sync.rs:1047-1076`). A response with
   `success == false` becomes `sync_failure(RegistrationFailed)` carrying
   the server message.
4. **Persist.** On success the handler builds a **fresh** `SyncSettings`
   from `Default::default()` plus `enabled = true`, `server_url`,
   `api_key`, `device_id`, and `credential_revision = 1`
   (`:42-47`), then `save_sync_settings` (`:49-52`). A save failure is
   printed to stderr and propagated. Because the struct is rebuilt from
   `default()`, **no prior field survives** — a re-registration resets
   `sync_direction`, `auto_sync`, the debounce/max-delay/timeout values,
   and the clipboard auto-clear setting. Re-apply them with
   `snp sync config` afterwards.
5. **Print.** `Registration successful!`, a masked key, the keychain note,
   the device ID, and `Saved to: <path>` (`:54-69`). The mask keeps the
   first 4 and last 4 characters and renders `****` for keys of 8
   characters or fewer (`:55-62`).

## The typed missing-API-key error

`SyncClient` has a private guard that every authenticated RPC calls before
touching the wire:

```rust
// src/sync.rs:900-908
fn require_api_key(&self) -> SnipResult<()> {
    if self.settings.api_key.is_empty() {
        return Err(SnipError::runtime_error(
            "Sync API key missing",
            Some("No API key is configured. Run 'snp register --force' to provision one."),
        ));
    }
    Ok(())
}
```

`sync.toml` deserializes `api_key` with `#[serde(default)]`, so a
headerless legacy config parses cleanly with an empty key. Checking only
`enabled` let requests go out unauthenticated and burn the whole
retry/backoff ladder on requests that can only ever come back
`Unauthenticated`.

`require_api_key` is called by: `list_libraries` (`src/sync.rs:1080`),
`create_library` (`:1016`), `list_premade_libraries` (`:1051`),
`get_premade_library` (`:1083`), and `search_premade_libraries`
(`:1127`).

The underlying `add_api_key_metadata` (`src/sync.rs:306-314`) no longer
asserts — it logs `refusing to attach an empty API key to a sync request`
and returns without attaching a header:

```rust
pub(crate) fn add_api_key_metadata<T>(request: &mut tonic::Request<T>, api_key: &str) {
    if api_key.is_empty() {
        tracing::error!("refusing to attach an empty API key to a sync request");
        return;
    }
    if let Ok(val) = format!("Bearer {api_key}").parse() {
        request.metadata_mut().insert("authorization", val);
    }
}
```

## Mutation vs read-only

Mutating (configuration only). It writes `sync.toml` and, when the OS
keychain is available, the API key entry. It does **not** touch snippet
libraries, so there is no transaction gate
(`gate_mutation_on_interrupted_transactions`) and no local-data lock:
sync-settings persistence has its own CRC32-guarded atomic path.

The auto-sync *worker* is not started by registration; `auto_sync` stays
off until `snp sync config --auto-sync on` is run (and sync itself must
be enabled).

## Tokio runtime

`run` takes `&tokio::runtime::Runtime` and uses `runtime.block_on(...)`
for the `Register` RPC. `RUNTIME` is the global `LazyLock` runtime in
`src/main.rs:29-35`, passed in at `src/main.rs:764`.

Startup policy: `Commands::Register(_)` uses
`StartupRecoveryPolicy::SuppressExplicitSync` and
`StartupServices::Logging` (`src/main.rs:1007-1009`) — a registration does
not reschedule pending auto-sync work.

## Auto-sync trigger

None. `register` never calls `notify_mutation`. Snippets created *after*
registration carry the new `device_id`; snippets created before it sync
with their stored IDs.

## Error / exit mapping

`SnipResult<()>` — every `Err` becomes `error: <message>` on stderr and
exit **1** (`src/main.rs:1063-1069`).

| Situation | Result |
|-----------|--------|
| Already registered without `--force` | exit 0, guidance printed |
| TLS/channel failure | client raises `sync_failure(ConnectFailed)`, re-wrapped as `runtime_error("Registration failed")` → exit 1 |
| Server returns `success = false` | client raises `sync_failure(RegistrationFailed)`, re-wrapped as `runtime_error("Registration failed")` → exit 1 |
| `save_sync_settings` fails | printed + propagated → exit 1 |

> The typed sync-failure kind does **not** survive the command boundary:
> `register_cmd.rs:64-69` converts every client error into
> `SnipError::runtime_error("Registration failed", Some(&e.to_string()))`,
> so `SyncFailureKind::ConnectFailed` / `RegistrationFailed` never reach the
> CLI outcome mapping. The exit code is still 1 and the original message is
> preserved verbatim in the detail field.

There is no `CliOutcome` on this path and no exit-2 (usage) case: clap
rejects unknown flags before the handler runs.

## Keychain and settings persistence

`save_sync_settings` (`src/config/sync_settings.rs:464`) writes
`sync.toml` with a CRC32 integrity header and `0600` permissions, and
upserts the API key into the OS keychain (`keyring`). In a production build
the keychain is not optional: when it is unavailable the save is **refused**
rather than falling back to plaintext. The `SNP_ALLOW_PLAINTEXT_API_KEY` seam
that permits plaintext is compiled in only under `test-support`. `migrate_plaintext_api_key`
(`src/config/sync_settings.rs:353-391`) migrates a plaintext key to the
keychain on load and replaces it with the `@keychain` marker.

Keep `keyring = "4"` with its default features — keyring 4's `default` is `v1`,
which hard-enables the three platform stores, and `keyring::Entry` is gated on
`v1`, so `default-features = false` is a compile error rather than a silent
downgrade. There is no mock store.

## Testing seams

Tests never touch the real keychain or network: `RecordingServer` stands
in for the server, `TempDir` + `XDG_CONFIG_HOME` isolate `sync.toml`, and
`SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on all test commands. `set_var`
in tests requires `unsafe` (edition 2024).

## Key Files

| Path | Role |
|------|------|
| `src/commands/register_cmd.rs` | `RegisterArgs` + the `run` handler |
| `src/main.rs:143-145`, `:726-728` | Clap wiring and dispatch |
| `src/sync.rs:1047-1076` | `SyncClient::register` — TLS channel + `Register` RPC |
| `src/sync.rs:900-908` | `require_api_key` — typed missing-key error |
| `src/sync.rs:306-314` | `add_api_key_metadata` — refuses an empty key |
| `src/config/sync_settings.rs` | `SyncSettings`, `save`/`load`, keychain migration, `DEFAULT_SERVER_URL` |
| `src/utils/config.rs:220` | `get_sync_config_path` (re-exported as `config::get_sync_config_path`) |
| `src/error.rs` | `SnipError` — carries no credentials |

## Invariants

- **Never print or log the full API key.** The masked form is the only
  user-visible rendering; `SnipError` carries no credentials.
- **`credential_revision = 1`** on a fresh registration (rotation bumps
  it).
- **Re-registration resets unrelated settings** — the struct is rebuilt
  from `default()`, so customizations must be re-applied.
- **A saved custom server URL survives a bare `snp register --force`.**
- **The keychain is preferred over plaintext** in `sync.toml`.

## Gotchas

- **`--force` is required to re-register**, and it also discards the
  previous device's configuration.
- **`snp register` never fails with "not enabled"** — a successful
  registration unconditionally sets `enabled = true`.
- **The already-registered check only looks at `device_id`.** A config
  with a device ID but no API key still short-circuits without `--force`.
- **`register` has no `--json` mode**; output is human text only.
