# Correctness Program Closure Status

Program status: REOPENED
Blocking plan: plans/snip-it-correctness-11-verification-and-crash-closure.md
Baseline: 609ddca5611894684d2ca04a10138ddc606ff301
Final: (Phase 11 pending)

## Program Summary

snip-it (`snp`) is a terminal-first snippet manager for short scripts and commands. The correctness program was initiated to address auto-sync correctness defects and evolved through ten phases to cover architecture, testing, security, and release hardening.

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
| 10 | Final corrective closure (read-only recovery suppression, exact execution outcomes, feature boundary cleanup, self-update hardening, backup/restore hardening, documentation reconciliation) | Complete |

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
- **Total**: 2074 passed, 7 ignored (36+ suites)
- **Unit tests**: snip-it lib (1048), snip-sync (86)
- **Integration tests**: CLI (222), sync (4)
- **Phase 05A suites**: deterministic E2E, failure class contracts, debounce matrix, sync contracts, mutual exclusion, process lifecycle, local contracts, package evidence (174)
- **Phase 07A suites**: persistence unit, identity contract (62)
- **Backup/restore hardening**: backup contracts, restore security, restore transactions (128)
- **CLI contracts**: canary nonexecution, execution outcomes, readonly no recovery, update archive security (60)
- **Auto-sync suites**: closure, concurrency, config, detached worker, lifecycle, mutations, regression, security (193)
- **Other suites**: architecture, output contracts, recovery integration, release4 regression, scale, schema, security, selector integration (167)
- **Workspace tests**: cargo test --workspace --all-features
- **Clippy**: clean (no warnings)
- **Fmt**: clean (no diffs)

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
- Restore path validation rejects entries escaping the config directory (Phase 10)
- Self-update tar extraction rejects absolute paths, parent traversal, symlinks, hard links (Phase 10)

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

## Release Blockers (Phase 10) — Resolved

1. **StartupRecoveryPolicy not wired into dispatch** — FIXED: `classify_command` in `src/main.rs:1191-1243` maps every `Commands` variant exhaustively to a `StartupRecoveryPolicy`
2. **rollback_transaction never called from restore on failure** — FIXED: `restore_cmd.rs:662-670` calls `rollback_transaction` on error
3. **run_edit_output_by_id is dead code** — FIXED: called from `main.rs:915` via `resolve_selector` for exact edit path
4. **No atomic file replacement in restore** — FIXED: `restore_cmd.rs:361-362` uses `atomic_replace` with `Durability::DurableUserData`
5. **No test for exactly one pending generation after content-changing restore** — FIXED: tested in `restore_transactions.rs`
6. **Backup has no consistent snapshot mechanism** — FIXED: `backup_cmd.rs:266-289` takes in-memory snapshot before copying
7. **Lifecycle event assertions silently skipped** — FIXED: `--features test-support` is the only feature flag; CI runs lifecycle tests with test-support instrumentation
8. **HTTP not hard-rejected in self-update** — FIXED: `update.rs:256-259` hard-rejects HTTP URLs
9. **THREAT_MODEL.md claims signed release assets** — FIXED: THREAT_MODEL.md correctly states "not cryptographically signed"

## Release Blockers (Phase 11) — Open

1. **Dry-run recovery classification** — `classify_command` maps `Restore`, `Import`, `Repair` to `Allow` regardless of dry-run flag
2. **Headline E2E permits zero server-side effect** — test accepts count=0 due to keychain exception
3. **Transaction journal not crash-complete** — enriched in-memory journal not durably rewritten before live replacements
4. **No automatic transaction crash recovery** — interrupted journals detectable but not automatically recovered
5. **Transaction lock stale forever** — bare `create_new` file with no PID, nonce, or liveness check
6. **Backup snapshot not serialized with mutations** — sequential reads without coordination
7. **General config entries not restored** — `kind = "config"` entries emitted but restore ignores them
8. **Manifest validation permissive** — free-form kind strings, no schema/kind/duplicate rejection
9. **Execution failure mapping incomplete** — timeout, signal, spawn failures can exit 1 instead of 8
10. **Windows ZIP extraction not prevalidated** — delegates to PowerShell `Expand-Archive`
11. **Package CI uses `unzip` for `.crate` files** — `.crate` files are tar/gzip, not ZIP
12. **Closure evidence stale** — older commit, overstated proofs

## Known Non-Blocking Limitations

1. **CRC32 integrity**: Detects accidental corruption but does not authenticate against a malicious local actor. This is by design — the threat model assumes local-only access.
2. **No mutual TLS**: Authentication is API-key-based via bearer token. No client certificate authentication.
3. **No SBOM generation**: Software bill of materials not yet generated.
4. **No build provenance attestation**: Not yet implemented.
5. **Argon2id at OWASP minimum**: 16 MiB memory cost. Higher security could use 64 MiB+.
6. **No AAD in encryption**: Ciphertext not bound to context. Acceptable for current use.
7. **No ciphertext format versioning**: Format implicitly versioned by fixed salt/nonce sizes.

## Exit Criteria Verification

| Criterion | Status |
|-----------|--------|
| Final threat model reflects shipped architecture | Yes — THREAT_MODEL.md accurately states "not cryptographically signed" |
| Secrets absent from unauthorized surfaces | Yes — audit documented in docs/SECURITY_AUDIT.md, sentinel tests cover backup dirs, log files, doctor/status JSON, pending/lock files |
| Process and timeout boundaries truthful and platform-tested | Yes — worker/executor documented and tested |
| Filesystem/archive/update paths hardened | Yes — restore path validation, self-update tar extraction validation, HTTP hard-rejected |
| Protocol/crypto implementation has explicit limits and evidence | Yes — documented in architecture/encryption.md and architecture/sync.md |
| Non-execution canaries pass | Yes — 16 canary tests covering get, list, status, validate, backup, search, library, restore --dry-run, sync run |
| Supply-chain/advisory/license policies pass | Yes — cargo-deny configured, CI job audits all 3 workspace members, docs/SUPPLY_CHAIN_POLICY.md |
| Fuzz/property smoke and regression corpus pass | Partial — no dedicated fuzz targets exist; critical parsing paths covered by unit/integration tests with edge-case inputs |
| Package/install/upgrade evidence committed | Partial — cargo package --workspace passes; no cross-platform install/upgrade matrix executed |
| Release-mode tests pass | Yes — cargo test --release --workspace added to CI |
| Documentation reconciled | Yes — all docs updated for Phase 10 closure |
| plans/snip-it-correctness-program-closure-status.md records real evidence | Yes — updated with resolved blocker status |
| No daemon, resident service, plugin runtime, workflow engine, remote execution, or second binary introduced | Yes — verified |
