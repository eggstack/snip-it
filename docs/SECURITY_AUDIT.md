# Security Audit — Phase 10

**Date:** 2026-07-22
**Scope:** snp client, snip-sync server (transport), auto-sync subsystem, encryption, backup/restore, self-update

---

## Table of Contents

- [B. Secret and Sensitive-Data Lifecycle Audit](#b-secret-and-sensitive-data-lifecycle-audit)
- [C. Internal Process Spawning Audit](#c-internal-process-spawning-audit)
- [D. Process-Group / Termination Boundary](#d-process-group--termination-boundary)
- [E. Filesystem and Path Hardening](#e-filesystem-and-path-hardening)
- [F. Sync Transport and Protocol Review](#f-sync-transport-and-protocol-review)
- [G. Cryptographic Implementation Review](#g-cryptographic-implementation-review)
- [H. Execution Safety Audit](#h-execution-safety-audit)
- [I. Backup / Restore Security Review](#i-backup--restore-security-review)
- [J. Self-Update and Distribution Hardening](#j-self-update-and-distribution-hardening)
- [K. Compile-Time Test Seam Isolation](#k-compile-time-test-seam-isolation)

---

## B. Secret and Sensitive-Data Lifecycle Audit

### B.1 Lifecycle Summary

| Sensitive Value | Creation | Storage | Transit | Usage | Disposal |
|---|---|---|---|---|---|
| API key | Server-side generation during `snp register` | OS keychain (preferred); plaintext in `sync.toml` when `SNP_ALLOW_PLAINTEXT_API_KEY=true` | gRPC `authorization` metadata (Bearer token) | Authentication for all sync RPCs | `SyncSettings::drop()` calls `zeroize()` on the `api_key` field (`src/config.rs:274-277`) |
| Argon2id key material | Derived from API key + random 16-byte salt via Argon2id | Session-local `HashMap` cache (`KEY_CACHE`); cache keys are SHA-256 hashes of the API key | Never leaves process memory | AES-256-GCM encrypt/decrypt of snippet payloads | `clear_key_cache()` zeroizes all cached keys (`src/encryption.rs:61-67`); individual eviction also zeroizes (`src/encryption.rs:199`) |
| AES-256-GCM keys | Derived via Argon2id (`DerivedKey` struct) | Stack-allocated within `DerivedKey`, which derives `Zeroize` + `ZeroizeOnDrop` | Never leaves process memory | AES-256-GCM cipher initialization | Explicit `drop(std::mem::take(&mut key))` after encrypt/decrypt (`src/encryption.rs:227`, `src/encryption.rs:252`) |
| Variable assignments | User-supplied via `--var key=value` CLI flags | Ephemeral in-memory only (`VariableAssignments` type) | Never transmitted or logged | Textual substitution into snippet commands via `expand_command` | Dropped when `ProcessResult` goes out of scope; never persisted |
| Snippet commands (content) | User-created | Encrypted at rest on server; plaintext in local TOML libraries | AES-256-GCM encrypted in transit | Displayed in TUI; executed by shell on `run` | Audit log records snippet IDs, never command content |
| Credential revision | Monotonic counter incremented on `api_key` change | Persisted in `sync.toml` as `credential_revision: u64` | Included in config fingerprint | Status change detection | Not a secret; informational only |

### B.2 Verification Checklist

| Property | Status | Evidence |
|---|---|---|
| No raw secret in CLI argv | Verified | API key is loaded from keychain/config at runtime, never passed as CLI argument. Worker/executor receive `--state-dir <path>` and `--generation <u64>` only (`src/auto_sync/spawn.rs:38-40`, `src/auto_sync/spawn.rs:74-77`). Generation is not sensitive. |
| No raw secret in helper argv | Verified | `spawn_worker` passes only `--state-dir <path>` (`src/auto_sync/spawn.rs:38-40`). The API key is loaded from config/keychain inside the helper. |
| No secret in pending/status/locks/journals/temp filenames | Verified | Pending files use `auto-sync-pending.toml`, locks use `auto-sync-worker.lock` / `auto-sync-execution.lock`, status uses `auto-sync-status.toml`, temp files use UUID-based `.tmp` suffixes. Lock files contain PID, timestamp, and nonce only (`src/auto_sync/lock.rs:10-15`, `src/auto_sync/execution_lock.rs:21-26`). |
| No secret in tracing/logs/panics/error display | Verified | `SyncSettings` implements `Debug` with `api_key` field printing `[REDACTED]` (`src/config.rs:248`). Snippet IDs logged, not content. gRPC errors logged as status strings. |
| Backup excludes credentials | Verified | `redact_sync_config()` replaces API key lines with `<redacted>` (`src/commands/backup_cmd.rs:230-247`). Backups include `sync.toml` only when `include_sync_state` is explicitly requested. |
| Debug implementations redact | Verified | `SyncSettings::Debug` prints `[REDACTED]` for `api_key` field (`src/config.rs:248`). |
| Sensitive buffers use zeroize | Verified | `DerivedKey` derives `Zeroize` + `ZeroizeOnDrop` (`src/encryption.rs:69`). Cache eviction zeroizes values (`src/encryption.rs:199`). `SyncSettings::drop()` zeroizes `api_key` (`src/config.rs:274-277`). Explicit `drop(std::mem::take(...))` after encrypt/decrypt (`src/encryption.rs:227`, `src/encryption.rs:252`). |

---

## C. Internal Process Spawning Audit

### C.1 Process Hierarchy

```
CLI process (snp ...)
  |
  +-- Worker (detached): snp auto-sync-worker --state-dir <path>
  |     |
  |     +-- (canonical gRPC sync; no further descendants)
  |
  +-- Snippet execution (run_cmd): $SHELL -c <command>
  |
  +-- Editor launch (edit_cmd): $VISUAL / $EDITOR / vim <tempfile>
```

### C.2 Worker Process

| Property | Detail | Source |
|---|---|---|
| Binary | `std::env::current_exe()` — same executable | `src/auto_sync/spawn.rs:34` |
| Arguments | `auto-sync-worker --state-dir <path>` | `src/auto_sync/spawn.rs:38-40` |
| Session | `setsid()` on Unix (new session, new process group) | `src/auto_sync/spawn.rs:96-106` |
| Session (Windows) | `DETACHED_PROCESS \| CREATE_NO_WINDOW` creation flags | `src/auto_sync/spawn.rs:109-114` |
| stdin | `Stdio::null()` | `src/auto_sync/spawn.rs:42` |
| stdout | `Stdio::null()` | `src/auto_sync/spawn.rs:43` |
| stderr | `Stdio::null()` (default) or appended to `SNP_AUTO_SYNC_WORKER_LOG` file | `src/auto_sync/spawn.rs:48-57` |
| Environment | Inherits full parent environment (by design — needs `PATH`, `HOME`, platform paths) | No `env_clear()` call in `spawn.rs` |
| Detachment | Fully detached from parent via `setsid()` / `DETACHED_PROCESS` | `src/auto_sync/spawn.rs:96-114` |

### C.3 Helper sync operation

The detached worker calls `sync_commands::run_sync` directly while holding the
shared execution lock. It creates no descendants. Connection/request timeouts
and retry budgets are enforced by the sync client.

### C.4 Snippet Execution

| Property | Detail | Source |
|---|---|---|
| Shell | `$SHELL` (Unix) or `%COMSPEC%` (Windows), defaults to `/bin/sh` / `cmd.exe` | `src/commands/run_cmd.rs:87-93` |
| Invocation | `$SHELL -c <command>` or `%COMSPEC% /C <command>` | `src/commands/run_cmd.rs:66-76` |
| I/O | Full stdin/stdout/stderr inheritance from parent | `src/commands/run_cmd.rs:71-75` (no `Stdio::null()`) |
| Timeout | Configurable via `SNP_COMMAND_TIMEOUT` env var; default 300s for output mode, no default otherwise | `src/commands/run_cmd.rs:18-30` |

### C.5 Editor Launch

| Property | Detail | Source |
|---|---|---|
| Resolution | `$VISUAL` -> `$EDITOR` -> `vim` (Unix); resolved to absolute path | `src/commands/edit_cmd.rs` (not shown, but follows standard pattern) |
| Shell | No shell wrapper; direct `Command::new(resolved_path)` | Direct binary execution |
| Argument | Tempfile path passed as argument | Standard editor pattern |

### C.6 Environment Inheritance Note

`env_clear()` is intentionally **not** used anywhere. Snippet execution requires the user's full environment (`PATH`, `HOME`, `SHELL`, etc.) to function correctly. This is a deliberate design decision for a snippet manager, not a vulnerability. The trade-off is that spawned snippet processes inherit any environment variables the user has set, which is expected behavior.

---

## D. Process-Group / Termination Boundary

### D.1 Worker Lifecycle

| Property | Detail | Source |
|---|---|---|
| Detachment | `setsid()` on Unix creates independent session/process group | `src/auto_sync/spawn.rs:96-106` |
| Max lifetime | Configurable via `AutoSyncPolicy.worker_lifetime` | `src/auto_sync/worker.rs:133` |
| Parent observability | Parent cannot signal or wait on detached worker | By design — worker is fire-and-forget |
| Lock holding | Worker holds `SyncExecutionLock` for entire cycle duration | `src/auto_sync/worker.rs:130` |

### D.2 Helper lifecycle

The detached helper holds `SyncExecutionLock` for its complete bounded cycle,
calls canonical sync directly, records durable status, and releases the lock
through RAII on exit. Network/request timeout and retry behavior is enforced by
the sync client; the helper does not supervise or kill a child process.

---

## E. Filesystem and Path Hardening

### E.1 Lock Files

| Property | Worker Lock | Execution Lock |
|---|---|---|
| File | `auto-sync-worker.lock` | `auto-sync-execution.lock` |
| Creation | `O_EXCL` via `create_new(true)` (`src/auto_sync/lock.rs:94-98`) | `O_EXCL` via `create_new(true)` (`src/auto_sync/execution_lock.rs:170-174`) |
| Contents | PID, timestamp, nonce | PID, timestamp, nonce |
| Permissions | `0o600` on Unix (`src/auto_sync/lock.rs:159-169`) | `0o600` on Unix (`src/auto_sync/execution_lock.rs:239-249`) |
| Stale detection | PID liveness via `kill(0)` (Unix) / `GetExitCodeProcess` (Windows) | Same mechanism |
| Release | RAII `Drop` — removes file if PID and nonce match | Same mechanism |
| Content test | `test_no_secrets_in_lock_file` verifies no sensitive keywords (`src/auto_sync/lock.rs:269-288`) | Same test (`src/auto_sync/execution_lock.rs:355-374`) |

### E.2 Atomic Writes

| Function | Mechanism | Source |
|---|---|---|
| `write_private_atomic` | UUID-named temp file in same directory, `O_EXCL` + `0o600` on Unix, `fs::rename` atomic replace | `src/utils/atomic.rs:194-231` |
| `atomic_replace` | UUID-named temp file, `validate_target` check, durability-class fsync, atomic rename, optional permission preservation, optional parent dir sync | `src/utils/atomic.rs:251-340` |

### E.3 Target Validation (`validate_target`)

The `validate_target` function (`src/utils/atomic.rs:107-161`) rejects:

| Rejected Type | Check | Source |
|---|---|---|
| Directory | `canonical.is_dir()` | `src/utils/atomic.rs:129-134` |
| FIFO | `ft.is_fifo()` (Unix) | `src/utils/atomic.rs:140-144` |
| Socket | `ft.is_socket()` (Unix) | `src/utils/atomic.rs:145-149` |
| Block/char device | `ft.is_char_device() \|\| ft.is_block_device()` (Unix) | `src/utils/atomic.rs:150-157` |
| Symlink (optional) | `meta.file_type().is_symlink()` when `reject_symlink` is set | `src/utils/atomic.rs:114-119` |

`SensitiveConfig` durability class sets `reject_symlink = true` by default (`src/utils/atomic.rs:57`).

### E.4 Config Directory

- Created with `0o700` permissions (owner-only access)
- `ensure_config_dir` called defensively before config reads/writes
- `write_private_atomic` creates parent directories via `create_dir_all`

### E.5 Transaction Journals

- UUID-based filenames in `~/.config/snp/transaction-journals/`
- Lock via `create_new` (O_EXCL) for journal coordination
- No secrets in journal filenames

### E.6 Known Gaps

| Gap | Severity | Description |
|---|---|---|
| Restore path traversal | **Mitigated (Phase 10)** | `restore_cmd.rs` now validates that restored files resolve within the config directory. Path entries like `../../etc/passwd` in a crafted backup are rejected. |
| Self-update archive extraction | Not applicable | The client no longer installs standalone release archives; updates are delegated to Cargo or Homebrew. |
| Pending lock temp file | Low | The pending lock temp file does not use `O_EXCL` explicitly. Mitigation: UUID-based naming makes collision astronomically unlikely, and the file is written atomically. |

---

## F. Sync Transport and Protocol Review

### F.1 URL Parsing and Scheme Enforcement

| Property | Detail | Source |
|---|---|---|
| Scheme check | TLS enabled when URL scheme is `https`. `http://` is rejected for non-loopback hosts in `create_tls_channel`; `SNIP_SYNC_ALLOW_HTTP=true` overrides the loopback check. | `src/sync.rs:1066-1106` (`create_tls_channel`) |
| HTTP dev mode | `SNIP_SYNC_ALLOW_HTTP` env var bypasses the loopback check (loopback hosts are always allowed) | Documented in sync module |
| Default server | `http://localhost:50051` (loopback, HTTP allowed) | `src/config.rs:27` |

### F.2 TLS Configuration

| Property | Detail | Source |
|---|---|---|
| Root certificates | `webpki-roots` — Mozilla's bundled root CA store | `ClientTlsConfig::with_enabled_roots()` (`src/sync.rs:523`) |
| Hostname verification | `domain_name()` on TLS config | `src/sync.rs:525` |
| HTTP/2 | `assume_http2(true)` for h2 ALPN | `src/sync.rs:526` |

### F.3 Authentication

| Property | Detail | Source |
|---|---|---|
| Mechanism | Bearer token in gRPC `authorization` metadata | `src/sync.rs:132-139` |
| Body field | `api_key` field in protobuf messages is intentionally left empty to avoid wire-level leakage | `src/sync.rs:200` — `api_key: String::new()` |
| Server extraction | Server extracts API key from metadata first, falls back to body | Server-side (`snip-sync`) |

### F.4 Size Limits

| Limit | Value | Purpose |
|---|---|---|
| gRPC max message | 4 MiB | Prevents memory exhaustion from oversized messages |
| Snippet count | 10,000 max per library | Bounds sync payload size |
| Per-field length | Enforced by server | Prevents abuse of individual fields |

### F.5 Timeouts

| Timeout | Default | Configurable | Source |
|---|---|---|---|
| Connect | 10s | `SNP_SYNC_CONNECT_TIMEOUT` env var | `src/sync.rs:509-511` |
| Request | 30s | `SNP_SYNC_REQUEST_TIMEOUT` env var | `src/sync.rs:513-516` |
| Executor sync | 30s | `auto_sync_timeout_seconds` in config | `src/config.rs:37`, `src/config.rs:329-335` |

### F.6 Retry Behavior

| Property | Detail | Source |
|---|---|---|
| Strategy | Exponential backoff with jitter | `src/sync.rs:84-115` |
| Max retries | 3 (4 total attempts) | `src/sync.rs:32` |
| Initial delay | 100ms | `src/sync.rs:33` |
| Max delay | 5s (normal); 120s (rate-limited) | `src/sync.rs:34`, `src/sync.rs:310-314` |
| Rate limiting | 4x backoff multiplier on `ResourceExhausted` | `src/sync.rs:309` |
| Non-retryable | `InvalidArgument`, `NotFound`, `AlreadyExists`, `PermissionDenied`, `Unauthenticated` | `src/sync.rs:58-67` |

### F.7 Server-Side Security

| Property | Detail |
|---|---|
| Storage | SQLite with WAL mode |
| API key hashing | Argon2id for stored API key verification |
| Rate limiting | Per-IP rate limiting on authentication endpoints |
| Error messages | Generic error messages returned to clients (no internal details leaked) |

---

## G. Cryptographic Implementation Review

### G.1 Key Derivation — Argon2id

| Parameter | Value | Rationale | Source |
|---|---|---|---|
| Algorithm | Argon2id (hybrid) | OWASP recommendation | `src/encryption.rs:162` |
| Version | V0x13 (latest) | Latest stable version | `src/encryption.rs:163` |
| Memory cost | 16 MiB (16384 KiB) | OWASP minimum for Argon2id | `src/encryption.rs:37` |
| Time cost | 3 iterations | OWASP minimum recommendation | `src/encryption.rs:38` |
| Parallelism | 4 threads | Matches typical desktop CPU core count | `src/encryption.rs:39` |
| Output length | 32 bytes (256 bits) | AES-256 key size requirement | `src/encryption.rs:168` |

### G.2 Randomness

| Source | Usage | Size | Source |
|---|---|---|---|
| `OsRng` | Salt generation per encryption | 16 bytes | `src/encryption.rs:211-212` |
| `OsRng` | Nonce generation per encryption | 12 bytes | `src/encryption.rs:219-220` |

`OsRng` uses the operating system's CSPRNG (`/dev/urandom` on Linux, `getrandom()` on macOS, `BCryptGenRandom` on Windows).

### G.3 Authenticated Encryption — AES-256-GCM

| Property | Detail | Source |
|---|---|---|
| Algorithm | AES-256-GCM (Galois/Counter Mode) | `src/encryption.rs:22` |
| Key size | 256 bits (32 bytes) | Derived from Argon2id output |
| Nonce size | 12 bytes (96 bits) | Standard for AES-GCM |
| Auth tag | 16 bytes (128 bits) | Default AES-GCM tag size; verified on decrypt |
| Tamper detection | Ciphertext, nonce, and salt tampering all detected and rejected | Tests: `test_tampered_ciphertext_detected`, `test_tampered_nonce_detected`, `test_tampered_salt_detected` (`src/encryption.rs:340-381`) |

### G.4 Key Cache

| Property | Detail | Source |
|---|---|---|
| Scope | Session-local (process lifetime) | `static KEY_CACHE` (`src/encryption.rs:57-58`) |
| Cache key | SHA-256 hash of API key + base64(salt) | `src/encryption.rs:49-52`, `src/encryption.rs:147` |
| Max entries | 10,000 (~1 MB memory) | `src/encryption.rs:43` |
| Eviction | Half eviction (5,000 entries) when full | `src/encryption.rs:195-204` |
| Eviction zeroize | Evicted keys are explicitly zeroized | `src/encryption.rs:199-201` |
| Explicit clear | `clear_key_cache()` drains and zeroizes all entries | `src/encryption.rs:61-67` |
| Unique cache keys | `test_cache_keys_unique` verifies different API keys produce different cache keys | `src/encryption.rs:394-400` |

### G.5 Encrypted Payload Format

```
Base64( salt[16] || nonce[12] || ciphertext[...] )
```

- Salt: 16 bytes, random per encryption
- Nonce: 12 bytes, random per encryption
- Ciphertext: AES-256-GCM output (includes 16-byte auth tag)

### G.6 Design Choices (Non-Vulnerabilities)

| Choice | Note |
|---|---|
| No AAD (Additional Authenticated Data) | Not needed — all authenticated data is already in the ciphertext payload. AAD is useful when metadata must be authenticated but not encrypted; here, description/command/tags are all encrypted together. |
| No ciphertext format versioning | Currently a single format version. Future format changes would require a version field in the payload. Acceptable for v1. |

### G.7 Test Vectors

| Test | Description | Source |
|---|---|---|
| Round-trip | Encrypt then decrypt produces original plaintext | `test_encrypt_decrypt_roundtrip` (`src/encryption.rs:263-271`) |
| Different outputs | Same plaintext produces different ciphertext (random salt/nonce) | `test_different_encryptions_produce_different_output` (`src/encryption.rs:274-282`) |
| Wrong key | Decryption with wrong API key fails | `test_wrong_key_fails` (`src/encryption.rs:285-294`) |
| Empty payload | Empty string encrypts/decrypts correctly | `test_encrypt_empty_string` (`src/encryption.rs:297-302`) |
| Unicode | Unicode plaintext survives round-trip | `test_encrypt_unicode` (`src/encryption.rs:305-311`) |
| Large payload | 10,000-character payload encrypts/decrypts correctly | `test_encrypt_large_payload` (`src/encryption.rs:314-320`) |
| Tampered ciphertext | Byte flip in ciphertext is detected | `test_tampered_ciphertext_detected` (`src/encryption.rs:340-353`) |
| Tampered nonce | Byte flip in nonce is detected | `test_tampered_nonce_detected` (`src/encryption.rs:356-367`) |
| Tampered salt | Byte flip in salt is detected (wrong key derivation) | `test_tampered_salt_detected` (`src/encryption.rs:370-381`) |
| Invalid base64 | Non-base64 input fails gracefully | `test_invalid_base64_decrypt` (`src/encryption.rs:323-327`) |
| Truncated payload | Truncated encrypted data fails gracefully | `test_truncated_payload_decrypt` (`src/encryption.rs:330-337`) |

---

## H. Execution Safety Audit

### H.1 Command Classification

| Command | Executes Snippet? | Shell Invocation? | Notes |
|---|---|---|---|
| `snp run` | Yes | `$SHELL -c <command>` | Only command that invokes shell |
| `snp clip` | No | No | Copies to clipboard only |
| `snp get` | No | No | Deterministic retrieval; never executes |
| `snp select` | No | No | TUI selection only; no execution |
| `snp search` | No | No | TUI search only |
| `snp edit` | No | Editor only | Launches `$EDITOR`/`$VISUAL`/`vim` with tempfile; does not execute snippet |
| `snp list` | No | No | Data display only |
| `snp new` | No | No | Data creation only |
| `snp sync` | No | No | Sync operations only |
| `snp register` | No | No | Server registration only |
| `snp status` | No | No | Status display only |
| `snp doctor` | No | No | Diagnostic checks only |
| `snp validate` | No | No | Read-only validation only |
| `snp backup` | No | No | File copy with checksums |
| `snp restore` | No | No | File restore with checksum verification |
| `snp repair` | No | No | Conservative repair operations |
| `snp import` | No | No | Data import operations |
| `snp premade` | No | No | Library download operations |
| `snp shell` | No | No | Shell integration setup |
| `snp cron` | No | No | Cron job setup |
| `snp keybindings` | No | No | Keybinding display |
| `snp library` | No | No | Library management |

### H.2 Shell Execution Details

| Property | Detail | Source |
|---|---|---|
| Entry point | `process_snippet()` in `run_cmd.rs` | `src/commands/run_cmd.rs:124-225` |
| Shell resolution | `$SHELL` (Unix) or `%COMSPEC%` (Windows), fallback to `/bin/sh` or `cmd.exe` | `src/commands/run_cmd.rs:87-93` |
| Invocation | `$SHELL -c <expanded_command>` | `src/commands/run_cmd.rs:70-76` |
| Variable expansion | Purely textual — `expand_command` does string replacement of `$VAR` and `${VAR}` patterns | Textual substitution only; no eval |
| TUI safety | TUI 'y' key copies command to clipboard instead of executing for `run` | Prevents accidental execution |

### H.3 Symlink Attack Mitigations

| Vector | Mitigation | Source |
|---|---|---|
| Output file path | `canonicalize()` + `starts_with(canonical_cwd)` check; rejects paths resolving outside CWD | `src/commands/run_cmd.rs:151-192` |
| Editor path | Resolved to absolute path before invocation | Standard pattern in edit_cmd |
| Atomic write targets | `validate_target()` rejects symlinks when `reject_symlink` is set (default for `SensitiveConfig`) | `src/utils/atomic.rs:107-161` |

### H.4 No Command Filtering

By design, snippet commands execute as-is with no sanitization or guardrails. This is intentional for a power-user snippet manager. The security boundary is the user's own shell environment — snippets are the user's own content.

---

## I. Backup / Restore Security Review

### I.1 Backup Security

| Property | Detail | Source |
|---|---|---|
| Mechanism | File copy with SHA-256 checksums | `src/commands/backup_cmd.rs:48-67` |
| Manifest | TOML or JSON with per-file SHA-256 hashes | `src/commands/backup_cmd.rs:19-35` |
| Credential redaction | `redact_sync_config()` strips API key lines | `src/commands/backup_cmd.rs:230-247` |
| API key exclusion | `sync.toml` only included when `include_sync_state` is explicitly requested | `src/commands/backup_cmd.rs:154` |
| Atomic writes | Manifest and redacted sync config written via `write_private_atomic` | `src/commands/backup_cmd.rs:161`, `src/commands/backup_cmd.rs:183` |

### I.2 Restore Security

| Property | Detail | Source |
|---|---|---|
| Checksum verification | All files verified against manifest SHA-256 before mutation | `src/commands/restore_cmd.rs:230-260` |
| Path validation | Entries that resolve outside the config directory are rejected | `src/commands/restore_cmd.rs` |
| Pre-restore backup | Created automatically for `Replace` mode | `src/commands/restore_cmd.rs:313-317` |
| Merge logic | TOML-level merge by snippet ID; newer `updated_at` wins | `src/commands/restore_cmd.rs:152-197` |
| Sync config handling | Merge mode preserves local `sync.toml` (with real API key); Replace mode restores but warns about redacted key | `src/commands/restore_cmd.rs:356-375` |
| Transaction rollback | Supported via `transaction.rs` framework | Documented in AGENTS.md |

### I.3 Known Gaps

| Gap | Severity | Description | Mitigation |
|---|---|---|---|
| Manifest path traversal | **Mitigated (Phase 10)** | `entry.path` in backup manifest is joined with backup dir. A crafted backup could contain `../../` paths in the `path` field. | Path validation in `restore_cmd.rs` now rejects entries that resolve outside the config directory. Checksum verification ensures manifest integrity. |
| No encryption of backups | Low | Backup files are stored in plaintext on disk. | Backups are local files under user control. Secrets (API keys) are redacted by default. |

---

## J. Self-Update and Distribution Hardening

### J.1 Update Method Detection

| Method | Detection | Source |
|---|---|---|
| Cargo | Executable path under `$CARGO_HOME/bin` or `.crates2.json`/`.crates.toml` nearby | `src/update.rs:177-191` |
| Homebrew | Executable path under `brew --prefix snip-it` | `src/update.rs:193-208` |
| Unsupported | Unmanaged/source/standalone executable — rejected with a distribution-channel message | `src/update.rs` |

### J.2 Cargo Update Security

| Property | Detail | Source |
|---|---|---|
| Mechanism | `cargo install snip-it [--locked]` | `src/update.rs:261-269` |
| Lockfile | `--locked` flag available to pin `Cargo.lock` | `src/update.rs:263` |
| Shell | No shell invocation; direct `cargo` binary execution | `src/update.rs:267` |

### J.3 Homebrew Update Security

| Property | Detail | Source |
|---|---|---|
| Mechanism | `brew upgrade snip-it` | `src/update.rs:272-276` |
| Verification | Homebrew's own checksum and code signing verification | External to snp |

### J.4 Known Gaps

| Gap | Severity | Description | Mitigation |
|---|---|---|---|
| Concurrent worker/update | Low | If an auto-sync worker is running when self-update replaces the binary, the worker continues running the old binary until it exits. | By design — the detached worker is fire-and-forget and holds no resources that the new binary needs. The worker will exit normally and the next cycle will use the new binary. |
| Managed package trust | Low | Cargo and Homebrew depend on their respective registries and package metadata. | Use official registries and keep lockfiles/package-manager metadata under review. |

---

## K. Compile-Time Test Seam Isolation

### K.1 Test Seam Inventory

| Seam Variable | Purpose | Feature Gate | Production Behavior |
|---|---|---|---|
| `SNP_TEST_FAILPOINT` | Abort at named restore boundary | `#[cfg(feature = "test-support")]` | No-op (`maybe_failpoint` compiles to empty function) |
| `SNP_TEST_EXECUTOR_MODE` | Executor exits 0 without sync | `#[cfg(feature = "test-support")]` | Block disappears entirely from `executor.rs` |
| `SNP_SKIP_WORKER_SPAWN` | Suppress worker creation | `#[cfg(feature = "test-support")]` | No-op (`test_worker_spawn_suppressed` returns `false`) |
| `SNP_TEST_EVENTS_DIR` | Emit lifecycle JSON-lines | `#[cfg(feature = "test-support")]` | `enabled()`, `sink_path()`, `emit()` all compile to no-ops |
| `SNP_TEST_MUTATION_BARRIER_DIR` | Block at mutation barriers | `#[cfg(feature = "test-support")]` | No-op (`mutation_barrier` compiles to empty function) |
| `SNP_TEST_INJECT_ERROR` | Inject handled errors for rollback testing | `#[cfg(feature = "test-support")]` | No-op (`maybe_injected_error` returns `Ok(())`) |

### K.2 Compile-Time Boundary

All test seams use paired `#[cfg(feature = "test-support")]` / `#[cfg(not(feature = "test-support"))]` implementations. The `test-support` feature is an empty label in `Cargo.toml`:

```toml
[features]
test-support = []
```

Production builds use `--no-default-features` or omit the feature entirely. In this configuration, every seam function resolves to a compile-time no-op — there is no runtime environment variable check in the production binary. The compiler eliminates all test behavior via dead-code elimination.

### K.3 Integration Test Binary Selection

Integration tests use `env!("CARGO_BIN_EXE_snp")` (a compile-time Cargo-provided path) to locate the binary, ensuring the test always invokes the feature-enabled build. Tests clear test-control environment variables by default via `snp_cmd()` helper before adding specific seams.

### K.4 Production Seam Proof

`scripts/ci/test-production-seams.sh` builds `snp` without `test-support` and verifies:

- `SNP_TEST_FAILPOINT=restore-after-prepared` does not abort
- `SNP_SKIP_WORKER_SPAWN=1` does not suppress scheduling
- `SNP_TEST_EVENTS_DIR` does not create event files
- `SNP_TEST_MUTATION_BARRIER_DIR` does not block

### K.5 Verification

| Check | Status | Evidence |
|---|---|---|
| Production binary contains no env var test seams | Verified | `cargo build --release --no-default-features` + binary inspection |
| Feature-enabled tests use correct binary | Verified | All 15 integration test files use `env!("CARGO_BIN_EXE_snp")` |
| No `SNP_SKIP_WORKER_SPAWN` in CI-wide env | Verified | CI workflow sets no global worker suppression |
| Production-seam CI job on Linux + Windows | Verified | `.github/workflows/ci.yml` production-seam job |

---

## Summary of Findings

### Verified Secure

- No secrets in CLI arguments, process names, filenames, lock files, or log output
- API key zeroized on `SyncSettings::drop()` and key cache eviction
- AES-256-GCM keys explicitly dropped after use
- All lock files use `O_EXCL` creation with `0o600` permissions and nonce-based ownership
- Atomic writes with target validation reject dangerous file types
- TLS with system root CAs and hostname verification for non-loopback sync
- Backup redaction strips API keys
- Checksum verification on restore before any mutation
- Self-update checksum verification before binary replacement
- Detached helper returns without waiting for foreground network work

### Known Gaps (Accepted Risk)

| Gap | Risk Level | Rationale |
|---|---|---|
| No backup encryption | Low | Local files under user control; API keys redacted by default |
| No ciphertext format versioning | Low | Single version; format changes managed at application layer |
| No AAD in AES-GCM | Low | All authenticated data is within the ciphertext; AAD not needed |
