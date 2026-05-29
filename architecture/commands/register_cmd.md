# register_cmd — Device Registration

## Overview

`register_cmd` registers a device with the snip-sync server. Device registration is required before sync operations can occur.

## Entry Point

```rust
pub fn run(matches: &ArgMatches) -> SnipResult<()>
```

## Flow

1. **URL Input** — Prompt for server URL (or use saved setting)
2. **Credentials** — Prompt for username/password or API key
3. **Registration Request** — Call `SyncClient::register_device()` via gRPC
4. **Store API Key** — Save returned API key to system keychain via `keyring`
5. **Update Config** — Save server URL and direction to `sync.toml`

## Registration Request

```protobuf
message RegisterRequest {
    string server_id = 1;    // Server identifier
    string device_name = 2;  // Hostname/user@host
    string api_key = 3;      // Pre-shared key or password
}
```

## Response

```protobuf
message RegisterResponse {
    bool success = 1;
    string message = 2;
    string api_key = 3;      // New API key for this device
}
```

## Keychain Storage

The returned API key is stored in the system keychain:
- **macOS**: Keychain via `keyring`
- **Linux**: libsecret DBUS
- **Windows**: Credential Manager

This avoids storing plaintext API keys in config files.

## Flags

- `--server <url>` — Server URL (non-interactive)
- `--name <device>` — Device name override
- `--api-key <key>` — Use existing API key

## Error Handling

- `SnipError::Sync` on registration failure
- `SnipError::Keychain` on keyring access failure
- `SnipError::InvalidCredentials` on auth failure

## Related

- [sync_cmd.md](sync_cmd.md) — Sync operations (requires registration)
- [sync.md](../sync.md) — Sync protocol and merge details
- [config.md](../config.md) — Sync settings
