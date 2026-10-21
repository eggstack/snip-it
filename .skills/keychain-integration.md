# Keychain Integration Pattern

## Overview
API keys should be stored in the OS keychain rather than plaintext config files.
This skill covers the pattern used in snp for keychain integration.

Operator-facing contract: [`docs/THREAT_MODEL.md`](../docs/THREAT_MODEL.md) and
[`SECURITY.md`](../SECURITY.md). Deep dive:
[`architecture/config.md`](../architecture/config.md#keychain-via-keyring-v4--srcconfigsync_settingsrs236-391).

## Implementation Pattern

### Dependencies
```toml
keyring = "4"
```

Keyring 4's default `v1` feature set provides the platform stores (Apple Keychain,
Windows Credential Manager, zbus Secret Service on Linux desktop). Do **not**
build with `default-features = false`: in keyring 4 `v1` is the feature that
exposes `keyring::Entry` at all (the crate gates its whole `v1` module on it),
so dropping it is a **compile error**, not a silent downgrade — and `v1`
hard-enables exactly those three stores, so there is no "store-less"
configuration to accidentally ship.

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
  working (`src/config/sync_settings.rs:276-333`).
- **Fail fast (test builds only):** under `test-support` with the plaintext seam
  set, a stored `@keychain` marker makes deserialization **refuse to load** rather
  than return the literal marker (`src/config/sync_settings.rs:302-317`).
  Returning the marker would authenticate every subsequent sync with a bogus
  credential.

### Migration
On load, if the API key is not empty and not `@keychain`
(`migrate_plaintext_api_key`, `src/config/sync_settings.rs:353-391`):
1. Store it in the keychain
2. Re-save the config with the `@keychain` marker
3. If the keychain is unavailable, the **store** step fails, the migration
   aborts before the re-save, and the failure is logged at error level with the
   cause named (`src/config/sync_settings.rs:377-384`). The legacy plaintext
   value stays on disk and the load still succeeds with that plaintext key in
   memory. A failed marker re-save is logged the same way
   (`src/config/sync_settings.rs:386-390`).
4. Under `test-support` the migration is skipped entirely when either test seam
   is set — the keychain is not the authoritative source there
   (`src/config/sync_settings.rs:366-375`).

### Keychain unavailable (CI, containers, headless)
In a production build there is no automatic plaintext fallback: saving sync
settings fails with an explicit error. Tests do not hit this because they build
with `--features test-support` and set `SNP_ALLOW_PLAINTEXT_API_KEY=true`, which
takes the plaintext branch before `keychain_store` is ever called. If you add a
real fallback, it is a security change: it must be opt-in, warned, and
documented in `SECURITY.md` — do not add one incidentally.

## Platform Notes
- macOS: Uses Keychain Services (apple-native-keyring-store)
- Linux: Uses Secret Service over the pure-Rust `zbus` client
  (zbus-secret-service-keyring-store). Linux Keyutils is a separate store that
  is **not** part of keyring 4's default `v1` set.
- Windows: Uses Windows Credential Store
- All handled transparently by the `keyring` crate

## Test Seams (never remove)

There are **two** test-only credential seams. Both are inert in production builds
because every read site is wrapped in `#[cfg(feature = "test-support")]` — that
cfg gate is the actual protection, not a runtime check:

| Seam | Effect | Gates |
|------|--------|-------|
| `SNP_ALLOW_PLAINTEXT_API_KEY=true` (exact match) | Bypasses the OS keychain **and forbids keychain access**; deserialization fails fast on a `@keychain` marker | `src/config/sync_settings.rs:257,303,373` |
| `SNP_TEST_CREDENTIAL_FILE=<path>` | Reads the API key from a file, for tests that must supply a credential without a keychain; save writes the plaintext value straight through and the migration is skipped | `src/config/sync_settings.rs:251,281,367` |

- Both are read at serialize, deserialize, and migration sites, so a new read
  path must repeat the `#[cfg(feature = "test-support")]` gate.
- `scripts/ci/test-production-seams.sh` builds without `test-support` and
  binary-scans for `SNP_ALLOW_DIR_FSYNC_FAILURE` only (not every test-only var);
  the plaintext-key and credential-file seams rely on the cfg gate above.
- `set_var` in tests needs `unsafe` (edition 2024).
- Linux CI installs `libdbus-1-dev` + `pkg-config`
  (`.github/workflows/ci.yml`); the default keyring 4 Linux store is the
  pure-Rust `zbus` client, so no C D-Bus library is actually linked.
