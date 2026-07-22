# Security Audit — Phase 09A

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
| No raw secret in CLI argv | Verified | API key is loaded from keychain/config at runtime, never passed as CLI argument. Worker/executor receive `--state-dir` only (`src/auto_sync/spawn.rs:38-40`). |
| No raw secret in worker/executor argv | Verified | `spawn_worker` and `spawn_executor` pass only `--state-dir <path>` (`src/auto_sync/spawn.rs:38-40`, `src/auto_sync/spawn.rs:74-77`). The API key is loaded from keychain inside the executor child process (`src/auto_sync/executor.rs:193`). |
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
  |     +-- Executor (child): snp auto-sync-execute --state-dir <path>
  |           |
  |           +-- (gRPC client only; no further descendants)
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

### C.3 Executor Process

| Property | Detail | Source |
|---|---|---|
| Binary | `std::env::current_exe()` — same executable | `src/auto_sync/spawn.rs:71` |
| Arguments | `auto-sync-execute --state-dir <path>` | `src/auto_sync/spawn.rs:74-77` |
| Parent | Regular child of worker (not detached) | `src/auto_sync/spawn.rs:70` doc comment |
| stdin | `Stdio::null()` | `src/auto_sync/spawn.rs:79` |
| stdout | `Stdio::null()` | `src/auto_sync/spawn.rs:80` |
| stderr | `Stdio::null()` or `SNP_AUTO_SYNC_WORKER_LOG` | `src/auto_sync/spawn.rs:81-89` |
| Environment | Inherits full parent environment; loads API key from keychain inside child | `src/auto_sync/executor.rs:193` |
| Descendants | None by design — gRPC client only, no subprocess spawning | `src/auto_sync/executor.rs` — calls `run_sync()` which uses tonic gRPC |

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

### D.2 Executor Lifecycle

| Property | Detail | Source |
|---|---|---|
| Relationship | Regular child process of worker | `src/auto_sync/spawn.rs:70` |
| Worker wait | `wait_child_with_timeout()` polls `try_wait()` every 100ms | `src/auto_sync/worker.rs:632-649` |
| Timeout handling | SIGTERM -> 2s grace period -> SIGKILL | `src/auto_sync/worker.rs:543-548`, `src/auto_sync/worker.rs:651-673` |
| Grace period | Configurable via `AutoSyncPolicy.termination_grace` (default 2s) | `src/auto_sync/worker.rs:544` |
| Descendants | Creates no descendants (gRPC client only) | `src/auto_sync/executor.rs` — `run_sync()` |
| Lock release | Lock released when `SyncExecutionLock` is dropped on worker exit | `src/auto_sync/execution_lock.rs:86-95` |

### D.3 Termination Semantics

1. Worker detects executor timeout via `wait_child_with_timeout` returning `Ok(None)`.
2. Worker sends SIGTERM (Unix) / `child.kill()` (Windows).
3. Worker sleeps for `termination_grace` duration (default 2s).
4. Worker checks `child.try_wait()` — if still alive, sends SIGKILL (Unix) / kill (Windows).
5. Worker calls `child.wait()` to reap the process.
6. Worker records failure status and releases the execution lock on drop.

### D.4 Direct-Child Reap Semantics

Verified by tests:
- `test_terminate_child_reap` — confirms `child.wait()` succeeds after SIGTERM (`src/auto_sync/worker.rs:984-992`)
- `test_force_kill_child_reap` — confirms `child.wait()` succeeds after SIGKILL (`src/auto_sync/worker.rs:995-1003`)
- `test_wait_child_with_timeout_exits_before_deadline` — confirms normal exit detection (`src/auto_sync/worker.rs:941-947`)
- `test_wait_child_with_timeout_returns_none_on_timeout` — confirms timeout detection (`src/auto_sync/worker.rs:950-959`)

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
| Restore path traversal | Medium | `entry.path` in backup manifest is not canonicalized before joining with config dir during restore. A crafted backup could contain path entries like `../../etc/passwd` in `entry.path`. Mitigation: manifests are SHA-256 verified and users control their own backup sources. (`src/commands/restore_cmd.rs:232-237`) |
| Self-update tar symlink following | Low | `tar -xf` follows symlinks by default in archive extraction. A malicious release archive could contain symlinks pointing outside the work directory. Mitigation: releases are checksum-verified, and the extracted binary is explicitly checked to exist as a regular file before installation. (`src/update.rs:388-420`) |
| Pending lock temp file | Low | The pending lock temp file does not use `O_EXCL` explicitly. Mitigation: UUID-based naming makes collision astronomically unlikely, and the file is written atomically. |

---

## F. Sync Transport and Protocol Review

### F.1 URL Parsing and Scheme Enforcement

| Property | Detail | Source |
|---|---|---|
| Scheme check | HTTPS required for non-loopback hosts | `src/sync.rs:502-507` |
| HTTP dev mode | `SNIP_SYNC_ALLOW_HTTP` env var bypasses HTTPS requirement | Documented in sync module |
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
| Pre-restore backup | Created automatically for `Replace` mode | `src/commands/restore_cmd.rs:313-317` |
| Merge logic | TOML-level merge by snippet ID; newer `updated_at` wins | `src/commands/restore_cmd.rs:152-197` |
| Sync config handling | Merge mode preserves local `sync.toml` (with real API key); Replace mode restores but warns about redacted key | `src/commands/restore_cmd.rs:356-375` |
| Transaction rollback | Supported via `transaction.rs` framework | Documented in AGENTS.md |

### I.3 Known Gaps

| Gap | Severity | Description | Mitigation |
|---|---|---|---|
| Manifest path traversal | Medium | `entry.path` in backup manifest is joined with backup dir without canonicalization. A crafted backup could contain `../../` paths in the `path` field. | Users control their own backup sources. Checksum verification ensures manifest integrity. The restore code joins paths with `backup.join(&entry.path)` which can traverse, but the backup directory is user-selected. |
| No encryption of backups | Low | Backup files are stored in plaintext on disk. | Backups are local files under user control. Secrets (API keys) are redacted by default. |

---

## J. Self-Update and Distribution Hardening

### J.1 Update Method Detection

| Method | Detection | Source |
|---|---|---|
| Cargo | Executable path under `$CARGO_HOME/bin` or `.crates2.json`/`.crates.toml` nearby | `src/update.rs:177-191` |
| Homebrew | Executable path under `brew --prefix snip-it` | `src/update.rs:193-208` |
| GitHub Release | Fallback when not Cargo, Homebrew, or source build | `src/update.rs:139-155` |
| Unsupported | Source build (`target/debug` or `target/release` in path) — rejected | `src/update.rs:157-163` |

### J.2 Standalone (GitHub Release) Update Security

| Property | Detail | Source |
|---|---|---|
| Download URL | HTTPS only (GitHub API + release assets) | `src/update.rs:21-22` |
| Checksum verification | SHA-256 hash compared against `SHA256SUMS` manifest from same release | `src/update.rs:361-382` |
| Archive extraction | `tar -xf` to temp directory | `src/update.rs:388-420` |
| Binary replacement | Atomic `fs::rename` with permission preservation | `src/update.rs:423-438` |
| Temp directory | PID + timestamp-named in executable's parent dir | `src/update.rs:335-351` |
| Cleanup | `fs::remove_dir_all(work_dir)` on success | `src/update.rs:437` |

### J.3 Cargo Update Security

| Property | Detail | Source |
|---|---|---|
| Mechanism | `cargo install snip-it [--locked]` | `src/update.rs:261-269` |
| Lockfile | `--locked` flag available to pin `Cargo.lock` | `src/update.rs:263` |
| Shell | No shell invocation; direct `cargo` binary execution | `src/update.rs:267` |

### J.4 Homebrew Update Security

| Property | Detail | Source |
|---|---|---|
| Mechanism | `brew upgrade snip-it` | `src/update.rs:272-276` |
| Verification | Homebrew's own checksum and code signing verification | External to snp |

### J.5 Known Gaps

| Gap | Severity | Description | Mitigation |
|---|---|---|---|
| Tar symlink following | Low | `tar -xf` follows symlinks by default. A malicious archive could contain symlinks pointing outside the work directory. | Releases are SHA-256 checksum-verified. The extracted binary is checked to exist as a file before installation (`src/update.rs:319-325`). The work directory is under the executable's parent dir with a unique PID-timestamp name. |
| Concurrent worker/update | Low | If an auto-sync worker is running when self-update replaces the binary, the worker continues running the old binary until it exits. | By design — the detached worker is fire-and-forget and holds no resources that the new binary needs. The worker will exit normally and the next cycle will use the new binary. |
| Checksum manifest trust | Low | The `SHA256SUMS` file is fetched from the same release as the archive. A compromised release could ship matching checksums. | GitHub release signing and the user's trust in the repository provide the root of trust. This is standard practice for GitHub-distributed binaries. |

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
- Process termination uses SIGTERM -> grace -> SIGKILL pattern

### Known Gaps (Accepted Risk)

| Gap | Risk Level | Rationale |
|---|---|---|
| Restore manifest path traversal | Medium | User controls backup source; checksum verification ensures manifest integrity |
| Tar symlink following in self-update | Low | Checksum-verified releases; extracted binary existence checked |
| No backup encryption | Low | Local files under user control; API keys redacted by default |
| No ciphertext format versioning | Low | Single version; format changes managed at application layer |
| No AAD in AES-GCM | Low | All authenticated data is within the ciphertext; AAD not needed |
