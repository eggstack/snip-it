# Keychain Integration Pattern

## Overview
API keys should be stored in the OS keychain rather than plaintext config files.
This skill covers the pattern used in snp for keychain integration.

## Implementation Pattern

### Dependencies
```toml
keyring = "3"
```

### Storage (on save)
```rust
const KEYCHAIN_SERVICE: &str = "snp-sync";
const KEYCHAIN_USER: &str = "api-key";
const KEYCHAIN_MARKER: &str = "@keychain";

fn keychain_store(api_key: &str) -> SnipResult<()> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER)
        .map_err(|e| SnipError::runtime_error("keychain entry", Some(&e.to_string())))?;
    entry.set_password(api_key)
        .map_err(|e| SnipError::runtime_error("keychain store", Some(&e.to_string())))?;
    Ok(())
}
```

### Retrieval (on load)
```rust
fn keychain_retrieve() -> SnipResult<String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER)
        .map_err(|e| SnipError::runtime_error("keychain entry", Some(&e.to_string())))?;
    entry.get_password()
        .map_err(|e| SnipError::runtime_error("keychain retrieve", Some(&e.to_string())))
}
```

### Serde Integration
Use custom serialize/deserialize with `#[serde(serialize_with, deserialize_with)]`:
- **Serialize:** Try keychain first, write `@keychain` marker if successful, else write plaintext with warning
- **Deserialize:** If value is `@keychain`, fetch from keychain; else return as-is (legacy plaintext)

### Migration
On load, if API key is not empty and not `@keychain`:
1. Store in keychain
2. Re-save config with `@keychain` marker
3. Log warning if keychain unavailable

### Fallback
If keychain is unavailable (CI, containers, headless):
- Keep plaintext storage
- Log warning about insecure storage
- Application continues to work

## Platform Notes
- macOS: Uses Keychain Services (apple-native-keyring-store)
- Linux: Uses Secret Service (dbus/zbus) or Linux Keyutils
- Windows: Uses Windows Credential Store
- All handled transparently by the `keyring` crate
