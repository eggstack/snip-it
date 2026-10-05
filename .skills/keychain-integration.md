# Keychain Integration Pattern

## Overview
API keys should be stored in the OS keychain rather than plaintext config files.
This skill covers the pattern used in snp for keychain integration.

## Implementation Pattern

### Dependencies
```toml
keyring = "4"
```

Keyring 4's default `v1` feature set provides the platform stores (Apple Keychain,
Windows Credential Manager, zbus Secret Service on Linux desktop). Do **not**
build with `default-features = false` without re-enabling a store — credential
persistence silently degrades to keyring's mock store.

### Storage (on save)
```rust
const KEYCHAIN_SERVICE: &str = "snp-sync";
const KEYCHAIN_DEFAULT_USER: &str = "api-key";
const KEYCHAIN_MARKER: &str = "@keychain";

fn keychain_store(api_key: &str, user: &str) -> SnipResult<()> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, user)
        .map_err(|e| SnipError::runtime_error("keychain entry", Some(&e.to_string())))?;
    entry.set_password(api_key)
        .map_err(|e| SnipError::runtime_error("keychain store", Some(&e.to_string())))?;
    Ok(())
}
```

### Retrieval (on load)
```rust
fn keychain_retrieve(user: &str) -> SnipResult<String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, user)
        .map_err(|e| SnipError::runtime_error("keychain entry", Some(&e.to_string())))?;
    entry.get_password()
        .map_err(|e| SnipError::runtime_error("keychain retrieve", Some(&e.to_string())))
}
```

### Serde Integration
Use custom serialize/deserialize with `#[serde(serialize_with, deserialize_with)]`:
- **Serialize (production):** store in the keychain and write the `@keychain`
  marker. If the keychain is unavailable, the write is **refused** —
  `"keychain unavailable: {e}; refusing plaintext API-key storage"`
  (`src/config/sync_settings.rs:256-268`). There is **no** plaintext downgrade on
  save. Do not "restore" a fallback here; that would silently write a credential
  to disk.
- **Deserialize:** if the value is `@keychain`, retrieve it from the keychain
  (a retrieval failure is a hard error, never an empty credential). Any other
  value is returned as-is, which is how pre-existing plaintext configs keep
  working (`src/config/sync_settings.rs:331-333`).
- **Fail fast (test builds only):** under `test-support` with the plaintext seam
  set, a stored `@keychain` marker makes deserialization **refuse to load** rather
  than return the literal marker (`src/config/sync_settings.rs:302-317`).
  Returning the marker would authenticate every subsequent sync with a bogus
  credential.

### Migration
On load, if the API key is not empty and not `@keychain`:
1. Store it in the keychain
2. Re-save the config with the `@keychain` marker
3. If the keychain is unavailable, the re-save fails loudly — the legacy
   plaintext value stays on disk and the error names the cause. It is **not**
   downgraded to a warning.

### Keychain unavailable (CI, containers, headless)
In a production build there is no automatic plaintext fallback: saving sync
settings fails with an explicit error. Tests do not hit this because they build
with `--features test-support` and set `SNP_ALLOW_PLAINTEXT_API_KEY=true`, which
takes the plaintext branch before `keychain_store` is ever called. If you add a
real fallback, it is a security change: it must be opt-in, warned, and
documented in `SECURITY.md` — do not add one incidentally.

## Platform Notes
- macOS: Uses Keychain Services (apple-native-keyring-store)
- Linux: Uses Secret Service (dbus/zbus) or Linux Keyutils
- Windows: Uses Windows Credential Store
- All handled transparently by the `keyring` crate

## Test Seams (never remove)

There are **two** test-only credential seams. Both are inert in production builds
because every read site is wrapped in `#[cfg(feature = "test-support")]` — that
cfg gate is the actual protection, not a runtime check:

| Seam | Effect | Gates |
|------|--------|-------|
| `SNP_ALLOW_PLAINTEXT_API_KEY=true` (exact match) | Bypasses the OS keychain **and forbids keychain access**; deserialization fails fast on a `@keychain` marker | `src/config/sync_settings.rs:257,303,373` |
| `SNP_TEST_CREDENTIAL_FILE=<path>` | Reads the API key from a file, for tests that must supply a credential without a keychain | `src/config/sync_settings.rs:251,281,367` |

- Both are read at serialize, deserialize, and sync-setup sites, so a new read
  path must repeat the `#[cfg(feature = "test-support")]` gate.
- `scripts/ci/test-production-seams.sh` builds without `test-support` and
  binary-scans for `SNP_ALLOW_DIR_FSYNC_FAILURE` only (not every test-only var);
  the plaintext-key and credential-file seams rely on the cfg gate above.
- `set_var` in tests needs `unsafe` (edition 2024).
- Linux CI needs `libdbus-1-dev` + `pkg-config` for the Secret Service store.
