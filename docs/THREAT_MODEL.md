# Threat Model for snip-it

**Version:** 1.0
**Date:** 2026-07-22
**Status:** Phase 09A — Workstream A
**Last reviewed:** 2026-07-22

---

## Table of Contents

1. [Purpose and Scope](#1-purpose-and-scope)
2. [Assets](#2-assets)
3. [Trust Boundaries](#3-trust-boundaries)
4. [Threat Actors](#4-threat-actors)
5. [Threat Catalogue](#5-threat-catalogue)
6. [Same-User Attacker Limitations](#6-same-user-attacker-limitations)
7. [Threat-to-Module Mapping](#7-threat-to-module-mapping)
8. [Cryptographic Inventory](#8-cryptographic-inventory)
9. [Open Questions and Future Work](#9-open-questions-and-future-work)

---

## 1. Purpose and Scope

This document defines the security threat model for snip-it, a local-first snippet manager with optional end-to-end encrypted synchronization. It identifies assets, trust boundaries, threat actors, concrete threats, mitigations, and residual risks. The scope covers the full lifecycle of snippet data: creation, local storage, synchronization, backup, restore, and self-update.

### Design Principles

- **Local-first:** All data lives on disk; sync is optional and end-to-end encrypted.
- **No runtime secrets in memory longer than necessary:** API keys are zeroized on drop.
- **Least privilege:** Workers and executors receive only the minimum state they need.
- **Defense in depth:** Atomic writes, file permissions, CRC32 integrity, and TLS all layer together.
- **Transparency:** This document exists so that users and contributors can reason about what snip-it does and does not protect against.

---

## 2. Assets

| Asset | Classification | Storage Location | Synced | Notes |
|-------|---------------|-----------------|--------|-------|
| Snippet commands | Confidential | `snippets.toml`, per-library TOML | Yes (encrypted) | Arbitrary shell content; highest sensitivity |
| Snippet descriptions | Confidential | Same as commands | Yes (encrypted) | Free-text metadata |
| Snippet tags | Internal | Same as commands | Yes (encrypted) | Classification metadata |
| Snippet output | Confidential | `usage.toml` (local) | No | Local-only presentation field |
| Snippet folders | Internal | Same as commands | Yes (encrypted) | Organizational structure |
| Snippet favorites | Internal | Same as commands | Yes (encrypted) | User preference |
| Stable IDs | Internal | Same as commands | Yes (encrypted) | Content-addressed identifiers |
| Timestamps | Internal | Same as commands | Yes (encrypted) | Created/modified metadata |
| Library/index configuration | Internal | `libraries.toml`, `libraries/*.toml` | Yes (encrypted) | Library registry and per-library structure |
| Usage metadata | Internal | `usage.toml` | No | Use count, last-used (never synced) |
| Sync credentials | Secret | OS keychain or `sync.toml` | No | Bearer token for gRPC auth |
| Credential revisions | Sensitive | `sync.toml` | No | Tracks key rotation state |
| Encryption keys | Secret | OS keychain (preferred) | No | Derived via Argon2id |
| Derived material | Secret | In-memory | No | Argon2id output, session-scoped |
| Nonces | Secret | Embedded in ciphertext | Yes | 12-byte AES-GCM random nonces |
| Encrypted remote payloads | Confidential | Sync server | Yes | Ciphertext blobs; server cannot decrypt |
| Encrypted remote metadata | Confidential | Sync server | Yes | Encrypted field-level metadata |
| Pending intent | Sensitive | `pending-sync` files | No | Coordinates detached worker cycles |
| Durable attempt status | Internal | `auto-sync-status.toml` | No | CRC32-integrity-protected, redacted |
| Lock files | Internal | `lock` files in state dir | No | O_EXCL creation, nonce-based ownership |
| Transaction journals | Sensitive | `transaction-journals/` | No | Crash-recovery state |
| Backups | Confidential | `backups/` | No | SHA-256 checksummed snapshots |
| Logs | Internal | `~/.config/snp/logs/` | No | Structured file-rotated logs |
| Installed binary | N/A | System or user-local path | No | Integrity depends on install method |
| Update channel | N/A | crates.io / GitHub Releases | No | SHA-256 verified downloads |
| Release assets | N/A | GitHub Releases, crates.io | No | Signed and checksummed |

---

## 3. Trust Boundaries

Each boundary below represents a transition where data crosses from one trust domain to another. Security properties must be re-evaluated at every boundary.

### Boundary 1: User Input and Imported Files

**Description:** Data entering snip-it from the user or from imported files (e.g., `snp import`).

**Properties:** Arbitrary content from untrusted sources. May contain shell metacharacters, oversized payloads, invalid TOML, path traversal attempts, or symlink/FIFO substitutions.

**Mitigations:** Input validation, TOML parse validation, `sanitize_library_name`, path traversal rejection, size limits.

### Boundary 2: Editable Local TOML vs. Private Control State

**Description:** User-editable snippet TOML files versus internal control files (lock files, pending intents, status, transaction journals).

**Properties:** Users can hand-edit snippet TOML but should not manually edit control state. Control files use CRC32 integrity (not cryptographic authentication).

**Mitigations:** CRC32 integrity checks, opaque filenames for control state, documentation of editable vs. internal files.

### Boundary 3: OS Keychain vs. Plaintext Fallback

**Description:** API keys stored in the OS keychain versus plaintext fallback in `sync.toml`.

**Properties:** Keychain provides OS-level access control. Plaintext fallback is readable by any process running as the same user.

**Mitigations:** Keychain-first strategy, `SNP_ALLOW_PLAINTEXT_API_KEY` environment gate for fallback, backup redaction strips API keys, zeroize on drop.

### Boundary 4: Parent CLI to Detached Worker

**Description:** The `snp` parent process spawns a detached worker (`snp auto-sync-worker`).

**Properties:** The worker runs in a separate process group (`setsid()`). Only `--state-dir` is passed in argv. No secrets appear in process titles or command-line arguments.

**Mitigations:** Minimal argv, `setsid()` for process isolation, no secrets in environment or args, nonce-based lock ownership.

### Boundary 5: Worker to Executor

**Description:** The detached worker spawns a child executor (`snp auto-sync-execute`).

**Properties:** The executor runs within the worker's process group. The worker holds the `SyncExecutionLock` for the entire cycle. The executor never acquires the execution lock itself.

**Mitigations:** Structural invariant: executor source never references `execution_lock`. Lock held by worker for full detached cycle. Executor timeout (30s default) prevents runaway.

### Boundary 6: Client to Sync Server

**Description:** The local gRPC client communicates with the snip-sync server.

**Properties:** All data in transit is end-to-end encrypted. The server never sees plaintext snippets. Bearer token in gRPC metadata. TLS required for non-loopback connections.

**Mitigations:** AES-256-GCM encryption with random nonces, TLS enforcement, `SNIP_SYNC_ALLOW_HTTP` gate for loopback-only exceptions, server stores Argon2id hashes of API keys (not plaintext).

### Boundary 7: Local Archive Restore

**Description:** Restoring from a local backup archive.

**Properties:** Archives may be tampered, corrupted, or contain path traversal payloads. Restore can overwrite current state.

**Mitigations:** SHA-256 checksum verification, path validation in `restore_cmd.rs`, dry-run mode, transaction journals for rollback, backup redaction.

### Boundary 8: Editor/Shell Invocation

**Description:** snip-it invokes the user's configured editor for snippet editing.

**Properties:** The editor path is resolved to an absolute path. No shell execution occurs in the editor invocation path. The editor process inherits only the file path being edited.

**Mitigations:** Absolute path resolution for editor, no shell interpretation of editor config, editor receives only the file to edit.

### Boundary 9: Self-Update / Package Manager

**Description:** The binary updates itself or is updated via a package manager.

**Properties:** Downloaded binaries may be tampered. Update channels may be compromised.

**Mitigations:** SHA-256 verification of downloaded binaries, checksum file validation, package manager signature verification (Homebrew, cargo).

### Boundary 10: CI / Release Publishing

**Description:** Build and release pipeline that produces signed artifacts.

**Properties:** Compromised CI could inject malicious code. Release signing keys could be stolen.

**Mitigations:** GitHub Actions with pinned runners, release signing, checksum publication, code review requirements.

---

## 4. Threat Actors

| Actor | Capability | Motivation | Scope |
|-------|-----------|------------|-------|
| **Malicious file author** | Craft TOML, backup archives, or import payloads | Code execution, data exfiltration, denial of service | Cross-user via shared snippets/libraries |
| **Network attacker (MitM)** | Intercept/modify network traffic | Credential theft, data tampering | Sync connections |
| **Compromised sync server** | Full server-side access | Data access, tampering, denial of service | All synced data |
| **Local co-tenant** | Same-user process execution | Data access, credential theft, lock tampering | Local filesystem |
| **Malicious editor/shell** | Config-file-level control | Code execution at snippet-edit time | Editor invocation |
| **Compromised dependency** | Supply chain control | Arbitrary code execution | Build and runtime |
| **Compromised release infrastructure** | CI/CD pipeline access | Binary replacement, backdoor injection | Self-update channel |
| **Accidental self** | Normal user behavior | Unintended snippet execution | All commands |

---

## 5. Threat Catalogue

### T1: Malicious Pet / TOML / Import File

| Field | Detail |
|-------|--------|
| **Description** | A crafted `.toml` snippet file or imported library contains oversized fields, deeply nested structures, invalid TOML, path traversal sequences, or shell metacharacters in commands. |
| **Attack vector** | User imports a shared library or receives a snippet file from an untrusted source. |
| **Mitigations** | TOML parse validation with strict deserialization; `sanitize_library_name` rejects path separators and control characters; command size cap (16 MiB); import rejects oversized payloads; content is not executed on import. |
| **Residual risk** | Low. A valid TOML file with malicious shell content in a `command` field is by design executable when the user runs it. This is the intended use case. |
| **User responsibility** | Review imported snippets before executing them. Treat snippet commands as you would any shell script from an untrusted source. |
| **Tests / evidence** | `tests/deterministic_e2e.rs`, `tests/local_contracts.rs`, `tests/package_evidence.rs`. Golden command corpus verifies TOML round-trip fidelity. |
| **Owner / module** | `src/commands/import_cmd.rs`, `src/library.rs`, `src/utils/toml_helpers.rs` |

### T2: Malicious Backup Archive

| Field | Detail |
|-------|--------|
| **Description** | A backup archive is crafted with path traversal entries (e.g., `../../etc/passwd`), checksum mismatches, or embedded malicious data. |
| **Attack vector** | User restores from an untrusted or corrupted backup file. |
| **Mitigations** | SHA-256 checksum verification against manifest; path validation in `restore_cmd.rs` rejects entries escaping the restore target; dry-run mode available; transaction journals enable rollback; `redact_sync_config` strips API keys from backup content. |
| **Residual risk** | Low. Checksum and path validation prevent most attacks. A backup from a fully compromised source could still contain valid but malicious snippet content (same as T1). |
| **User responsibility** | Only restore backups you created or trust. Use dry-run mode first. |
| **Tests / evidence** | `tests/deterministic_e2e.rs` (restore path). `src/commands/backup_cmd.rs`, `src/commands/restore_cmd.rs` unit tests. |
| **Owner / module** | `src/commands/backup_cmd.rs`, `src/commands/restore_cmd.rs` |

### T3: Compromised / Malformed Sync Server

| Field | Detail |
|-------|--------|
| **Description** | The sync server is compromised, returns malformed responses, or is impersonated. It could attempt to serve crafted encrypted payloads or harvest credentials. |
| **Attack vector** | Server-side compromise, DNS hijacking (with invalid TLS policy), or supply-chain attack on server deployment. |
| **Mitigations** | End-to-end encryption: server never sees plaintext snippets, commands, descriptions, or tags. AES-256-GCM with random 12-byte nonces; authentication tag verification on decryption rejects tampered ciphertext. API key is Argon2id-hashed server-side (server never stores plaintext). TLS required for non-loopback connections. |
| **Residual risk** | Low for confidentiality and integrity of snippet data. Metadata (timestamps, sizes) may be visible to the server in encrypted envelope headers. Denial of service is possible. |
| **User responsibility** | Verify your sync server URL is correct. Use a trusted server deployment. |
| **Tests / evidence** | `tests/sync_integration.rs`, `tests/sync_contracts.rs`. Encryption unit tests in `src/encryption.rs`. |
| **Owner / module** | `src/sync.rs`, `src/sync_commands.rs`, `src/encryption.rs`, `src/config.rs` |

### T4: Network Attacker Under Invalid TLS Policy

| Field | Detail |
|-------|--------|
| **Description** | An attacker on the network intercepts sync traffic when TLS is disabled or downgraded. |
| **Attack vector** | `SNIP_SYNC_ALLOW_HTTP=true` or loopback exception used in a non-loopback context; network co-located attacker. |
| **Mitigations** | TLS is required for all non-loopback connections. `SNIP_SYNC_ALLOW_HTTP` is gated and only effective for loopback addresses. Sync client rejects plaintext connections outside loopback. |
| **Residual risk** | Very low. The environment variable gate makes accidental plaintext use unlikely. A user who deliberately disables TLS for a non-loopback server accepts the risk. |
| **User responsibility** | Do not set `SNIP_SYNC_ALLOW_HTTP=true` for non-loopback servers. Use TLS in all production and staging environments. |
| **Tests / evidence** | `src/sync.rs` (TLS enforcement), `tests/sync_integration.rs` (loopback-only HTTP tests). |
| **Owner / module** | `src/sync.rs`, `src/config.rs` |

### T5: Same-Account Local Process

| Field | Detail |
|-------|--------|
| **Description** | A process running under the same user account reads snippet files, lock files, status, credentials, or other local state. |
| **Attack vector** | Malicious script, browser extension, or co-installed application running as the same user. |
| **Mitigations** | File permissions: `0o600` for sensitive files, `0o700` for directories. Lock files use O_EXCL creation with nonce-based ownership to detect tampering. CRC32 integrity on `auto-sync-status.toml` detects corruption (not cryptographic authentication). API keys stored in OS keychain where available; plaintext fallback is explicitly gated. |
| **Residual risk** | **Medium.** A same-user process can read all local files regardless of `0o600` permissions. CRC32 and file permissions detect corruption and reduce accidental exposure but do not authenticate state against a fully compromised user account. See [Section 6: Same-User Attacker Limitations](#6-same-user-attacker-limitations). |
| **User responsibility** | Treat your user account as a security boundary. Be cautious about installing untrusted software that runs under your account. |
| **Tests / evidence** | Lock nonce tests in `src/auto_sync/lock.rs`, status integrity tests in `src/auto_sync/status.rs`. |
| **Owner / module** | `src/utils/atomic.rs`, `src/auto_sync/lock.rs`, `src/auto_sync/status.rs`, `src/config.rs` |

### T6: Malicious Editor or Shell Config

| Field | Detail |
|-------|--------|
| **Description** | The user's configured editor path is manipulated to execute arbitrary code when snip-it opens a snippet for editing. |
| **Attack vector** | `EDITOR` or `VISUAL` environment variable set to a malicious command; shell rc file manipulation; editor binary replacement. |
| **Mitigations** | Editor path is resolved to an absolute path before invocation. No shell execution occurs in the editor path resolution. snip-it does not invoke the editor through a shell. The editor receives only the file path as an argument. |
| **Residual risk** | Low. If the editor binary itself is compromised, it can act arbitrarily when invoked by any program. This is outside snip-it's control. |
| **User responsibility** | Use a trusted editor. Verify that `EDITOR` / `VISUAL` point to legitimate binaries. |
| **Tests / evidence** | Editor path resolution logic in `src/commands/edit_cmd.rs`, `src/commands/new_cmd.rs`. |
| **Owner / module** | `src/commands/edit_cmd.rs`, `src/commands/new_cmd.rs`, `src/ui/mod.rs` |

### T7: Symlink / FIFO / Device Substitution

| Field | Detail |
|-------|--------|
| **Description** | A file system entry that appears normal is actually a symlink to a sensitive path, a FIFO/pipe, or a device file, used to redirect writes or reads to unintended locations. |
| **Attack vector** | Crafted snippet file or backup entry replaces a config file with a symlink to `/etc/passwd` or similar. |
| **Mitigations** | `validate_target()` in `src/utils/atomic.rs` rejects FIFOs, sockets, and device files. Atomic write path canonicalizes the target before writing. Temp-file-then-rename pattern prevents partial writes to unintended targets. |
| **Residual risk** | Low. Canonical path checks and target validation prevent most substitution attacks. Race conditions (TOCTOU) between validation and write are mitigated by the atomic rename pattern. |
| **User responsibility** | Ensure your config directory does not contain unexpected symlinks or special files. |
| **Tests / evidence** | `validate_target()` tests in `src/utils/atomic.rs`. |
| **Owner / module** | `src/utils/atomic.rs` |

### T8: PID Reuse / Stale Locks

| Field | Detail |
|-------|--------|
| **Description** | A lock file from a crashed process is left behind. A new process happens to reuse the same PID, appearing to own the stale lock. |
| **Attack vector** | Process crash during sync cycle; PID recycling by the OS. |
| **Mitigations** | Lock files contain a nonce (random value) created at lock acquisition time. Ownership is verified by nonce match, not PID alone. Lock staleness is handled by timeout logic. `STALE_LOCK_THRESHOLD_SECS` was removed (dead code); timeout-based expiry handles stale locks. |
| **Residual risk** | Very low. Nonce-based ownership prevents PID-reuse false positives. Timeout ensures eventual lock release after crashes. |
| **User responsibility** | None. This is handled transparently. |
| **Tests / evidence** | Lock nonce tests in `src/auto_sync/lock.rs`, `src/auto_sync/execution_lock.rs`. |
| **Owner / module** | `src/auto_sync/lock.rs`, `src/auto_sync/execution_lock.rs` |

### T9: Interrupted Writes / Process Crashes

| Field | Detail |
|-------|--------|
| **Description** | A write to a snippet file, config file, or status file is interrupted mid-write, leaving a corrupted partial file. |
| **Attack vector** | Power loss, OOM kill, `SIGKILL`, or panic during file I/O. |
| **Mitigations** | Atomic write pattern: write to a temporary file in the same directory, then `rename()` to the target path. `rename()` is atomic on POSIX for same-filesystem moves. Durability classes control `fsync` behavior per use case. Transaction journals provide crash recovery for multi-file operations. |
| **Residual risk** | Low. The temp-file-then-rename pattern is the standard approach for crash-safe file writes. The main residual risk is data loss for the in-flight operation, not corruption of existing data. |
| **User responsibility** | None. This is handled transparently. |
| **Tests / evidence** | Atomic write tests in `src/utils/atomic.rs`. Transaction journal tests in `src/transaction.rs`. |
| **Owner / module** | `src/utils/atomic.rs`, `src/transaction.rs` |

### T10: Oversized / Decompression Input

| Field | Detail |
|-------|--------|
| **Description** | An oversized payload is sent to the CLI, sync client, or server, causing excessive memory allocation, disk usage, or denial of service. |
| **Attack vector** | Crafted import file, oversized snippet, malformed gRPC message, or decompression bomb. |
| **Mitigations** | 16 MiB command size cap. gRPC message size limits on both client and server. Import command rejects oversized payloads. Argon2id parameters are bounded (16 MiB memory, 3 iterations, 4 parallelism). |
| **Residual risk** | Low. Size limits prevent most memory exhaustion attacks. Decompression bombs in gRPC payloads are bounded by message size limits. |
| **User responsibility** | Do not manually craft oversized TOML files or send malformed gRPC messages to your own server. |
| **Tests / evidence** | Size limit tests in `src/commands/import_cmd.rs`, gRPC config in `src/sync.rs`. |
| **Owner / module** | `src/commands/import_cmd.rs`, `src/sync.rs`, `src/encryption.rs` |

### T11: Compromised Release Asset / Checksum

| Field | Detail |
|-------|--------|
| **Description** | A release binary is replaced with a malicious version, or its checksum is tampered. |
| **Attack vector** | Compromised GitHub account, CI pipeline, or CDN; MITM on download. |
| **Mitigations** | SHA-256 checksum verification of downloaded binaries during self-update. Checksum file validation. Package managers (Homebrew, cargo) perform their own signature verification. Tar extraction rejects absolute paths, parent-directory traversal, symlinks, and hard links. HTTPS-only downloads. UUID-based temp directories prevent collision. |
| **Residual risk** | Low. SHA-256 provides strong integrity verification. If both the binary and checksum are compromised in the same release, detection requires manual review of release artifacts. |
| **User responsibility** | Verify release signatures where available. Use official installation channels. |
| **Tests / evidence** | Self-update verification logic in `src/update.rs`. |
| **Owner / module** | `src/update.rs` |

### T12: Accidental Execution of Unsafe Snippet

| Field | Detail |
|-------|--------|
| **Description** | A user accidentally executes a snippet with destructive shell content (e.g., `rm -rf /`). |
| **Attack vector** | Selecting the wrong snippet in the TUI, or scripting `snp run` without review. |
| **Mitigations** | Only `snp run` executes snippets. `snp get` retrieves without execution. `snp clip` copies to clipboard without execution. TUI displays command before execution. `--copy` flag available for non-execution output. Exact selectors (`--id`, `--command-exact`) reduce selection errors. |
| **Residual risk** | Medium. The user explicitly chooses to execute. snip-it intentionally does not sandbox or restrict snippet execution (by design for power users). |
| **User responsibility** | Review snippets before executing them. Use `snp get --field command` to inspect before running. |
| **Tests / evidence** | CLI integration tests in `tests/integration.rs`. |
| **Owner / module** | `src/commands/run_cmd.rs`, `src/commands/get_cmd.rs`, `src/commands/clip_cmd.rs` |

### T13: Dependency / Supply-Chain Compromise

| Field | Detail |
|-------|--------|
| **Description** | A crate dependency is compromised, introducing malicious code into the build or runtime. |
| **Attack vector** | Malicious crate publication, typosquatting, maintainer account compromise. |
| **Mitigations** | `cargo-deny` enages license and advisory checks. Locked builds via `Cargo.lock`. Workspace uses known, audited dependencies. Unknown registries are denied. |
| **Residual risk** | Low. `cargo-deny` and lockfiles reduce but do not eliminate supply-chain risk. A compromised transitive dependency that passes `cargo-deny` checks could still introduce vulnerabilities. |
| **User responsibility** | Review dependency changes when updating `Cargo.lock`. Monitor security advisories. |
| **Tests / evidence** | CI pipeline runs `cargo-deny`. |
| **Owner / module** | `Cargo.toml`, `Cargo.lock`, CI configuration. |

---

## 6. Same-User Attacker Limitations

A process running under the same user account as snip-it has effectively unrestricted access to the user's file system. snip-it's local protections (file permissions, CRC32, lock nonces) are **not** cryptographic authentication mechanisms and do not provide security against a fully compromised user account.

### What Local Protections Do

| Mechanism | Protects Against | Does NOT Protect Against |
|-----------|-----------------|--------------------------|
| `0o600` file permissions | Accidental exposure to other users on a multi-user system; casual file browsing | Any process running as the same user |
| `0o700` directory permissions | Same as above for directories | Same as above |
| CRC32 integrity (`auto-sync-status.toml`) | Accidental corruption, bit rot, partial writes | Intentional modification by a same-user attacker (CRC32 is not a cryptographic MAC) |
| Lock file nonces | Stale lock detection, PID-reuse false positives | A malicious process that reads the nonce from the lock file and writes a matching nonce |
| O_EXCL lock creation | Concurrent creation races | A process that deletes the lock file and recreates it |

### Design Rationale

snip-it is a local-first tool designed for single-user desktop environments. The primary threat model assumes the user's account is trusted. Local protections are defense-in-depth against accidental corruption and software bugs, not against a hostile local actor. Adding full state authentication (e.g., HMAC-based lock files) would increase complexity without meaningful security gain for the target deployment scenario.

### Recommendation

Users who require protection against local attackers should use full-disk encryption (FileVault, LUKS) and restrict physical access to their machine.

---

## 7. Threat-to-Module Mapping

| Threat | Primary Module(s) | Test Coverage | Residual Risk |
|--------|-------------------|---------------|---------------|
| T1: Malicious Pet/TOML/Import | `import_cmd.rs`, `library.rs`, `toml_helpers.rs` | `deterministic_e2e`, `local_contracts`, `package_evidence` | Low |
| T2: Malicious Backup Archive | `backup_cmd.rs`, `restore_cmd.rs` | `deterministic_e2e`, unit tests | Low |
| T3: Compromised Sync Server | `sync.rs`, `sync_commands.rs`, `encryption.rs` | `sync_integration`, `sync_contracts` | Low |
| T4: Network Attacker (TLS) | `sync.rs`, `config.rs` | `sync_integration` | Very Low |
| T5: Same-Account Local Process | `atomic.rs`, `lock.rs`, `status.rs`, `config.rs` | Lock/status unit tests | **Medium** |
| T6: Malicious Editor/Shell Config | `edit_cmd.rs`, `new_cmd.rs`, `ui/mod.rs` | Editor resolution tests | Low |
| T7: Symlink/FIFO/Device Substitution | `atomic.rs` | `validate_target()` tests | Low |
| T8: PID Reuse / Stale Locks | `lock.rs`, `execution_lock.rs` | Lock nonce tests | Very Low |
| T9: Interrupted Writes / Crashes | `atomic.rs`, `transaction.rs` | Atomic write tests, transaction tests | Low |
| T10: Oversized/Decompression Input | `import_cmd.rs`, `sync.rs`, `encryption.rs` | Size limit tests | Low |
| T11: Compromised Release Asset | `update.rs` | Self-update verification tests | Low |
| T12: Accidental Unsafe Execution | `run_cmd.rs`, `get_cmd.rs`, `clip_cmd.rs` | CLI integration tests | **Medium** |
| T13: Dependency/Supply-Chain | `Cargo.toml`, CI config | `cargo-deny` | Low |

---

## 8. Cryptographic Inventory

| Primitive | Parameters | Usage | Location |
|-----------|-----------|-------|----------|
| **Argon2id** | 16 MiB memory, 3 iterations, 4 parallelism, random salt per encryption | Key derivation from passphrase to AES-256 key | `src/encryption.rs` |
| **AES-256-GCM** | Random 12-byte nonce, 16-byte auth tag | Symmetric encryption of snippet data and metadata | `src/encryption.rs` |
| **SHA-256** | Standard | Binary checksum verification (self-update), backup manifest checksums | `src/update.rs`, `src/commands/backup_cmd.rs` |
| **CRC32** | Standard | Integrity check on `auto-sync-status.toml` (not cryptographic) | `src/auto_sync/status.rs` |
| **OS Keychain** | Platform-native | API key and encryption key storage | `src/config.rs`, `src/encryption.rs` |

### Cryptographic Properties

- **Confidentiality:** AES-256-GCM provides confidentiality for all synced snippet data. The sync server never has access to plaintext.
- **Integrity:** AES-GCM auth tags detect ciphertext tampering. CRC32 detects accidental corruption (not intentional modification).
- **Nonce security:** Each encryption operation generates a cryptographically random 12-byte nonce. Nonce reuse is prevented by random generation.
- **Key derivation:** Argon2id with memory-hard parameters resists GPU/ASIC attacks on passphrase guessing.
- **No authentication of local state:** File permissions and CRC32 do not provide cryptographic authentication. A same-user attacker can modify local state without detection beyond CRC32 integrity (which is trivially recomputed).

---

## 9. Open Questions and Future Work

| Item | Status | Notes |
|------|--------|-------|
| Full-disk encryption recommendation | Documented | See Section 6 |
| HMAC-based lock file authentication | Deferred | Not justified for single-user desktop threat model |
| Server-side access logging | Not in scope | Server-side concern, not snip-it client |
| Encrypted local at-rest for snippet files | Deferred | Full-disk encryption is the recommended approach; per-file encryption adds complexity with limited benefit when the user account is trusted |
| Supply-chain hardening (e.g., `cargo-vet`) | Potential future work | `cargo-deny` is the current mitigation |

---

*This document is a living artifact. Update it when new threats are identified, mitigations change, or the scope of snip-it evolves.*
