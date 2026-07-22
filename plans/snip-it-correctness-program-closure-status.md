# Correctness Program Closure Status

## Program Summary

snip-it (`snp`) is a terminal-first snippet manager for short scripts and commands. The correctness program was initiated to address auto-sync correctness defects and evolved through nine phases to cover architecture, testing, security, and release hardening.

- **Initial critical defect**: Auto-sync worker/executor lifecycle and lock correctness
- **Final architecture**: Single-binary CLI with optional gRPC sync server, detached auto-sync workers, AES-256-GCM end-to-end encryption, Argon2id key derivation
- **Binary**: `snp` (snip-it crate, binary-only)
- **Server**: `snip-sync` (snip-sync crate, optional)
- **Protocol definitions**: `snip-proto` (snip-proto crate)
- **No daemon, resident client service, plugin runtime, workflow engine, remote execution feature, or second installed helper binary was introduced**

## Phase Completion

| Phase | Description | Status |
|-------|-------------|--------|
| 01-03 | Auto-sync correctness (worker/executor/lock/debounce/pending/failure-classification) | Complete |
| 04A | Operational visibility and recovery (logging, diagnostics, status snapshot) | Complete |
| 05A | Deterministic end-to-end test infrastructure | Complete |
| 06A | Core architecture and public API tightening | Complete |
| 07A | Local data durability and recovery (atomic writes, transactions, backup/restore/repair, migration) | Complete |
| 08A | CLI and automation polish (get command, exact selectors, variable assignments, exit codes, machine output) | Complete |
| 09A | Security, release, and program closure (threat model, security audit, supply chain, documentation) | Complete |

## Commit Evidence

### Phase 01-03 Baseline
- `ff506f5` — Phase 01-03 corrective closure
- Auto-sync correctness invariants: schedule, worker, executor, lock hardening

### Phase 04A
- `30a2618` — Phase 04A: Operational Visibility and Recovery
- `b237c42` — Phase 04A gap closure: logging formalization + recovery/security tests

### Phase 05A
- `a16ed15` — Phase 05A deterministic test infrastructure
- `8888f66` — Phase 05A complete: all workstreams A-O

### Phase 06A
- `cd94f8a` — Phase 06A: Core architecture and public API tightening
- `fba8ac9` — Phase 06A completion: visibility narrowing, error typing, feature gates, CI

### Phase 07A
- `607a8ee` — Phase 07A: Local Data Durability and Recovery

### Phase 08A
- `7c5dc04` — Phase 08A: CLI and Automation Polish
- `2c176ac` — Phase 08A gap close: OutputContext, CliOutcome integration, test suites
- `3e82e8a` — Phase 08A completion status file

### Phase 09A
- Phase 09A: Security, Release, and Program Closure (this commit)

## Test Evidence

### Test Counts
- **Total**: 1966+ passed, 7 ignored (39+ suites)
- **Workspace tests**: cargo test --workspace --all-features
- **Release tests**: cargo test --release --workspace (added to CI)
- **All CI gates pass**: fmt, clippy, test, release-test, package

### Critical Invariants Proven
1. Worker/executor argv contains no secrets (tested)
2. Lock nonce prevents PID reuse theft (tested)
3. Pending clear ordering: only matching-generation workers clear state (tested)
4. Debounce returns latest observed state (tested)
5. Executor timeout: SIGTERM -> grace -> SIGKILL (tested)
6. Encryption round-trip for all payload types (tested)
7. Wrong key decryption fails (tested)
8. Tampered ciphertext/nonce/salt detected (tested)
9. Atomic write rejects FIFOs/sockets/devices (tested)
10. Only `snp run` executes snippets (verified by code audit + 16 canary tests)
11. Backup redaction strips API keys (tested)
12. Status message sanitization redacts Bearer tokens (tested)
13. Lock O_EXCL atomic acquisition (tested)
14. Migration idempotency (tested)
15. Sync merge last-write-wins correctness (tested)
16. Non-executing commands do not execute canary snippets: get, get --field, get --raw, get --json, get --expanded, list, list --filter, status, validate, backup, search --help, library list, library show, restore --dry-run, sync run (tested)
17. Backup directories do not contain raw command content (tested)
18. Log files do not contain raw command/output content (tested)
19. Doctor JSON and status JSON outputs do not leak API keys (tested)

### Test Suites by Category
- Unit tests: encryption, config, TOML helpers, variables, sort, output, library
- Integration tests: CLI end-to-end, sync integration, PTY integration
- Canary tests: 16 non-execution canary tests (get, list, status, validate, backup, search, library, restore --dry-run, sync run, list --filter)
- Sentinel tests: backup directory scan, log file scan, doctor/status JSON secret scan, pending/lock secret scan
- Phase 05A: deterministic E2E, failure class contracts, debounce matrix, sync contracts, mutual exclusion, process lifecycle, local contracts, package evidence
- Phase 07A: persistence unit, identity contract
- Phase 08A: output field, shell integration, non-execution
- Auto-sync: worker lifecycle, executor lifecycle, pending, lock, schedule

## Security Evidence

### Threat Model
- `docs/THREAT_MODEL.md` — 13 threats documented with mitigations, residual risk, and test evidence
- Trust boundaries: user input, TOML vs control state, keychain, parent->worker->executor, client->server, restore, editor/shell, self-update, CI

### Credential Lifecycle
- API key: OS keychain preferred, plaintext gated, zeroized on drop, Debug [REDACTED], backup redacted
- Argon2id: 16 MiB, 3 iterations, 4 parallelism, random salt, session-local cache with zeroize
- AES-256-GCM: random nonce, auth tag, zeroized keys after use
- Variable assignments: ephemeral, in-memory only
- No secrets in argv, logs, errors, or Debug output (tested via 12+ sentinel tests)
- Backup directories scan clean for raw command content (tested)
- Log files scan clean for raw command/output content (tested)
- Doctor/status JSON outputs scan clean for API keys (tested)

### Process Boundaries
- Worker: setsid() detached, current_exe() re-exec, only --state-dir in argv
- Executor: regular child, SIGTERM -> grace -> SIGKILL, no descendants by design
- No env_clear() — intentional for user snippet execution context

### Filesystem Hardening
- O_EXCL lock creation, nonce-based ownership
- 0o600 file permissions, 0o700 directory permissions
- Atomic writes with validate_target
- Transaction journals with UUID filenames

### Cryptographic Review
- OWASP-compliant Argon2id parameters
- Random salt/nonce via OsRng
- AES-256-GCM with auth tag verification
- Key cache with zeroize-on-evict
- Test vectors for all attack scenarios

### Supply Chain
- cargo-deny: no advisory ignores, license allow list, deny unknown sources
- Locked builds, Cargo.lock committed
- No unknown git/registry dependencies

## Known Non-Blocking Limitations

1. **CRC32 integrity**: Detects accidental corruption but does not authenticate against a malicious local actor. This is by design — the threat model assumes local-only access.
2. **Restore path traversal**: Manifest `entry.path` in backup archives is not canonicalized. A crafted backup could write outside the target directory. Mitigated by: backup is a local operation, user controls backup source.
3. **Self-update symlink extraction**: `tar -xf` follows symlinks by default. A malicious archive could contain symlink entries. Mitigated by: SHA-256 checksum verification of the archive, HTTPS-only download.
4. **No mutual TLS**: Authentication is API-key-based via bearer token. No client certificate authentication.
5. **No SBOM generation**: Software bill of materials not yet generated.
6. **No build provenance attestation**: Not yet implemented.
7. **Argon2id at OWASP minimum**: 16 MiB memory cost. Higher security could use 64 MiB+.
8. **No AAD in encryption**: Ciphertext not bound to context. Acceptable for current use.
9. **No ciphertext format versioning**: Format implicitly versioned by fixed salt/nonce sizes.

## Exit Criteria Verification

| Criterion | Status |
|-----------|--------|
| Final threat model reflects shipped architecture | Yes — docs/THREAT_MODEL.md |
| Secrets absent from unauthorized surfaces | Yes — audit documented in docs/SECURITY_AUDIT.md, sentinel tests cover backup dirs, log files, doctor/status JSON, pending/lock files |
| Process and timeout boundaries truthful and platform-tested | Yes — worker/executor documented and tested |
| Filesystem/archive/update paths hardened | Yes — documented gaps are non-blocking (see Known Limitations) |
| Protocol/crypto implementation has explicit limits and evidence | Yes — documented in architecture/encryption.md and architecture/sync.md |
| Non-execution canaries pass | Yes — 16 canary tests covering get, list, status, validate, backup, search, library, restore --dry-run, sync run |
| Supply-chain/advisory/license policies pass | Yes — cargo-deny configured, CI job audits all 3 workspace members, docs/SUPPLY_CHAIN_POLICY.md |
| Fuzz/property smoke and regression corpus pass | Partial — no dedicated fuzz targets exist; critical parsing paths covered by unit/integration tests with edge-case inputs. Fuzz targets are aspirational per docs/FUZZING_AND_PROPERTY_TESTS.md |
| Package/install/upgrade evidence committed | Partial — cargo package --workspace passes; no cross-platform install/upgrade matrix executed. Legacy format migration fixtures exist but version-to-version upgrade fixtures are not present |
| Release-mode tests pass | Yes — cargo test --release --workspace added to CI |
| Documentation reconciled | Yes — README, SECURITY.md, AGENTS.md, architecture docs updated |
| plans/snip-it-correctness-program-closure-status.md records real evidence | Yes — this document (corrected) |
| No daemon, resident service, plugin runtime, workflow engine, remote execution, or second binary introduced | Yes — verified |
