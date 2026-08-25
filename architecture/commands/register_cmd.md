# register_cmd — Device Registration

## Overview

`register_cmd` registers a device with the snip-sync server. Device registration is required before sync operations can occur.

## Entry Point

```rust
pub fn run(server: String, force: bool, runtime: &tokio::runtime::Runtime) -> SnipResult<()>
```

## Flow

1. **URL Input** — Prompt for server URL (or use saved setting)
2. **Registration Request** — Call `SyncClient::register()` via gRPC
3. **Store API Key** — Save returned API key to `sync.toml` via `save_sync_settings()` (OS keychain used for read-back; plaintext fallback with `SNP_ALLOW_PLAINTEXT_API_KEY=true`)
4. **Update Config** — Save server URL and direction to `sync.toml`

## Registration Request

```protobuf
message RegisterRequest {
    string device_id = 1;    // Device identifier (empty for new registrations)
}
```

## Response

```protobuf
message RegisterResponse {
    bool success = 1;
    string api_key = 2;      // New API key for this device
    string message = 3;
    string device_id = 4;    // Assigned device identifier
}
```

## Keychain Storage

On platforms with a supported keyring, the API key is stored in the system keychain for secure read-back:
- **macOS**: Keychain via `keyring`
- **Linux**: libsecret DBUS
- **Windows**: Credential Manager

When the keychain is unavailable, the API key is stored in plaintext in `sync.toml`. Set `SNP_ALLOW_PLAINTEXT_API_KEY=true` to allow this explicitly.

## Flags

- `--server <url>` — Server URL (defaults to built-in server URL)
- `--force` — Re-register even if already registered

## Error Handling

- `SnipError::RuntimeError` on registration failure or settings save failure

## Related

- [sync_cmd.md](sync_cmd.md) — Sync operations (requires registration)
- [sync.md](../sync.md) — Sync protocol and merge details
- [config.md](../config.md) — Sync settings
