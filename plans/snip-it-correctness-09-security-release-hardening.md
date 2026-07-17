# Phase 09: Security and Release Hardening

## Purpose

Close the correctness program with an updated threat model, secret-handling audit, subprocess and filesystem hardening, supply-chain controls, package verification, and release-blocking evidence.

This phase is not a substitute for the earlier correctness work. It verifies that the completed architecture remains safe under hostile local inputs, interrupted process lifecycles, compromised or malformed remote responses, and release-distribution risks.

## Preconditions

All earlier phases should be functionally complete:

- truthful canonical synchronization;
- generation-safe pending/debounce behavior;
- typed failure/backoff/status;
- operator recovery surfaces;
- real process/server integration tests;
- intentional architecture/public API boundaries;
- local backup/validation/repair;
- deterministic CLI/output contracts.

Do not use this phase to defer behavior-critical TODOs from earlier work.

## Security posture

`snip-it` executes user-selected commands through the user’s shell. It is not a sandbox and cannot make untrusted commands safe. The security objective is therefore to:

- preserve user intent;
- avoid unintended execution;
- protect synchronization credentials and encryption material;
- prevent control-file manipulation from causing silent state loss;
- constrain detached process behavior;
- reject malformed or unsafe imported/restored data;
- ensure update and release artifacts are verifiable;
- make failures explicit without leaking sensitive content.

## Workstream A: Update the threat model

Document assets:

- snippet commands and descriptions;
- tags/output metadata;
- local library integrity;
- snippet IDs and synchronization timestamps;
- API keys and keyring references;
- encryption keys/derived material;
- sync server account/library metadata;
- pending and status intent;
- update channel and installed binary;
- shell/editor/clipboard boundaries.

Document adversaries and failure sources:

- malicious imported Pet/TOML file;
- malicious backup archive;
- compromised sync server;
- network attacker when TLS policy is violated;
- local unprivileged process under the same account;
- malicious or replaced editor executable;
- malicious shell configuration;
- symlink/FIFO/device substitution in config/state paths;
- process ID reuse and stale lock artifacts;
- malformed/corrupt local files;
- interrupted writes and process crashes;
- compromised release asset or update metadata;
- accidental user invocation of dangerous snippet content.

For each threat, state:

- in-scope mitigation;
- residual risk;
- user responsibility;
- test evidence.

Clarify that CRC32/integrity fields detect accidental corruption and some malformed state but are not cryptographic authenticity against a same-user attacker.

## Workstream B: Audit secret lifecycle

Trace API keys and encryption material from creation/loading through use and drop.

Required checks:

- no secret in CLI argv;
- no secret in detached worker/executor argv;
- no secret in process titles;
- no secret in pending/status/lock/temp files;
- no secret in logs/tracing fields;
- no secret in panic messages;
- no secret in JSON diagnostics;
- no secret in backup by default;
- no secret in CI artifacts;
- no secret in shell history examples;
- keyring retrieval errors do not print values;
- plaintext credential fallback requires explicit opt-in and restrictive permissions;
- sensitive buffers use `zeroize` where feasible and meaningful;
- derived keys do not live longer than required;
- clone/debug implementations do not duplicate or format secrets;
- error source chains are sanitized before persistence/presentation.

Add sentinel-secret tests across:

- stdout;
- stderr;
- worker/executor logs;
- status/pending/locks;
- crash artifacts collected by tests;
- backup archives;
- doctor/status JSON.

## Workstream C: Harden process spawning

Review every external process invocation:

- detached worker re-exec;
- supervised executor re-exec;
- user shell execution;
- editor invocation;
- self-update package manager commands;
- clipboard helper if any platform uses one;
- certificate/server helper scripts if invoked.

Requirements:

- use direct executable invocation, not a shell, except deliberate snippet execution;
- pass arguments as separate OS strings;
- document current working directory behavior;
- minimize inherited environment for internal subprocesses while preserving required config path variables;
- close or null inherited stdin/stdout/stderr as intended;
- avoid leaking terminal handles;
- use `current_exe` safely and handle binary replacement/update races;
- verify re-exec target is the intended executable where practical;
- internal subcommands remain hidden;
- worker/executor reject unsafe or unexpected state paths;
- process creation errors preserve pending state;
- no detached child inherits sensitive variable values unnecessarily.

### Environment allowlist

Consider constructing an allowlisted environment for worker/executor containing only:

- HOME/XDG/platform config variables required to locate state;
- logging controls explicitly supported;
- test controls only in test builds;
- TLS/system variables required by the networking stack where unavoidable.

A full `env_clear()` can break certificate discovery or platform services, so use measured allowlisting/denylisting with tests.

## Workstream D: Process-group termination and timeout truthfulness

Determine whether the executor or sync path can create descendants. If yes, killing only the direct child may leave work running after the lock is released.

Unix preferred design:

- place executor in its own process group/session appropriate to supervision;
- terminate the group on timeout;
- wait for direct child;
- verify no known descendants remain;
- release lock only afterward.

Windows preferred design:

- use a Job Object if practical to bind executor descendants;
- terminate the job on timeout;
- close handles correctly;
- verify process exit;
- preserve pending.

If descendants cannot be spawned by design, document and test that constraint. Timeout documentation must match the actual termination boundary.

## Workstream E: Filesystem and local-state hardening

Apply consistent defenses to:

- snippet libraries;
- configuration;
- sync settings;
- pending/status/lock artifacts;
- logs;
- backup/restore paths;
- temporary files;
- update downloads.

Required checks:

- reject or safely handle symlink targets according to documented policy;
- reject FIFOs, devices, sockets, and directories where regular files are required;
- create sensitive files with restrictive permissions from first creation;
- avoid predictable temp names;
- keep temp files in same directory for atomic replacement;
- prevent path traversal in restore/import/archive extraction;
- bound file sizes before allocation;
- validate UTF-8 only where the format requires it;
- preserve exact command bytes under documented UTF-8 contract;
- use ownership-checked lock cleanup;
- treat malformed locks conservatively;
- avoid age-only live-lock theft;
- handle PID reuse with nonce/identity evidence;
- define same-user attacker limitations explicitly.

Add local race tests for path replacement between validation and open. Use open-handle-based validation or platform flags where practical rather than check-then-open only.

## Workstream F: Sync protocol and cryptographic review

Review:

- TLS requirement and loopback HTTP exception;
- server URL normalization;
- hostname/SNI validation;
- certificate trust configuration;
- API-key transport metadata;
- registration behavior;
- Argon2id parameters and salt handling;
- AES-256-GCM nonce generation and uniqueness;
- associated data use;
- ciphertext/version framing;
- replay/version semantics;
- error handling for authentication-tag failure;
- merge behavior with tampered or stale payloads;
- server-side metadata leakage;
- maximum message sizes and decompression/allocation limits.

Do not redesign the protocol casually during final closure. File separate design work for incompatible cryptographic/protocol changes. Fix clear implementation errors and add vectors/tests now.

Required test categories:

- known round-trip vectors;
- nonce uniqueness under stress;
- wrong key/tag failure;
- truncated/oversized payload;
- version mismatch;
- malformed protobuf/gRPC response;
- plaintext remote URL rejection;
- loopback development exception;
- credential redaction;
- server compromise cannot decrypt ciphertext under stated model.

## Workstream G: Command execution safety audit

The tool intentionally executes snippets. Ensure it does so only through explicit commands/actions.

Audit:

- `run` and TUI keybindings;
- search/select behavior;
- shell integration insertion versus execution;
- imported snippet preview;
- variable expansion;
- editor workflow;
- clipboard behavior;
- exact lookup commands.

Requirements:

- `list`, `get`, `select`, import dry-run, validate, doctor, status, backup, and export never execute snippets;
- `run` is explicit;
- TUI execution key is documented and distinct from copy/print;
- no command executes during parsing, highlighting, preview, variable detection, or sync;
- variable expansion remains textual and does not evaluate substitutions itself;
- imported commands are never evaluated during validation;
- shell integration does not use `eval` unless execution is explicitly requested and documented;
- multiline commands preserve intent;
- terminal restoration occurs on errors/signals.

Add canary tests using commands that would create files if accidentally executed; assert files remain absent for every non-executing path.

## Workstream H: Self-update and distribution hardening

Audit `snp update` paths:

- Cargo-installed update;
- Homebrew update;
- standalone GitHub release archive;
- checksum verification;
- architecture/platform selection;
- temporary download location;
- archive extraction;
- binary replacement;
- rollback/failure behavior.

Requirements for standalone update:

- HTTPS only;
- verify expected SHA-256 from release checksum file;
- parse checksum file safely and select exact asset name;
- reject path traversal/symlink archive entries;
- verify extracted binary is a regular file;
- replace atomically where permissions allow;
- preserve old binary until new asset verifies;
- clear error/rollback behavior;
- no `sudo` invocation without explicit user control;
- update cannot replace a different executable unexpectedly;
- hidden worker re-exec behavior remains valid after update.

Consider signed provenance/attestations as an enhancement, not a replacement for checksums.

## Workstream I: Supply-chain and dependency policy

Add or verify:

- `cargo deny` advisories, bans, licenses, and sources;
- dependency lockfile committed and used in release CI;
- minimal GitHub Actions permissions;
- pinned action versions or commit SHAs according to project policy;
- release environment protections;
- crates.io token scoped and protected;
- GHCR permissions scoped for `snip-sync` image;
- SBOM generation where practical;
- release provenance/attestation;
- dependency update automation with CI;
- review of duplicate crypto/TLS stacks;
- no unexpected git dependencies;
- license compatibility for bundled themes/assets.

Do not automatically deny all advisories without a documented exception process. Exceptions require owner, rationale, expiration/review date, and compensating controls.

## Workstream J: Fuzzing and property tests

Add focused fuzz/property targets for untrusted parsers and state transitions:

- snippet TOML parser;
- legacy/Pet migration;
- variable and choice parser;
- import/restore archive manifest/path validation;
- pending/status/lock parsers;
- encryption frame parser;
- sync merge inputs;
- shell completion/config parsing where useful.

Properties:

- no panic;
- bounded allocation;
- round-trip stability for valid data;
- migration idempotency;
- invalid input never executes commands;
- corruption never maps to synchronized/current;
- conditional clear never removes newer generation;
- normalized paths remain under intended root.

Run a bounded fuzz smoke job in CI and maintain longer local/nightly commands separately.

## Workstream K: Release gates

Required release-candidate commands:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --release --workspace
cargo package --workspace
cargo deny check
```

Also require:

- minimal/default/all feature builds where supported;
- docs.rs/documentation build;
- real-server sync suite;
- detached timeout/process suite;
- backup/restore/repair suite;
- shell integration suite;
- package/install smoke tests;
- Linux/macOS/Windows success;
- secret-sentinel scan;
- dependency/advisory report;
- release asset checksum verification test;
- upgrade from previous stable release using representative fixtures;
- no behavior-critical `TODO`, `unimplemented!`, placeholder success, or ignored required test.

Add an automated source scan for dangerous placeholders in critical modules, but review results manually to avoid brittle keyword-only gating.

## Workstream L: Release evidence and closure document

Create a final status file in `plans/` containing:

- roadmap phase completion table;
- final architecture summary;
- commit ranges;
- public behavior changes;
- migrations/config additions;
- test counts by category;
- platform CI links/status;
- security checks and fuzz targets;
- package/install evidence;
- performance/startup comparison;
- known limitations;
- deferred non-blocking work;
- explicit statement that no daemon, resident client service, plugin runtime, workflow engine, or hosted service was introduced.

Do not call the program complete while:

- any platform-sensitive required test is skipped;
- any sync success path can bypass real synchronization;
- pending can clear on non-success;
- timeout can release lock while descendants continue;
- secret sentinel appears in artifacts;
- restore/path traversal tests fail;
- package/update verification is incomplete;
- documentation contradicts implementation.

## Required tests

### Secret tests

- sentinel key/command in all process and file surfaces;
- panic/error/log redaction;
- backup exclusion;
- CI artifact scan;
- plaintext opt-in permissions.

### Process tests

- argv/env inspection;
- terminal/descriptor inheritance;
- current executable replacement scenarios;
- process-group/job termination;
- child/descendant death before unlock;
- spawn failure preservation;
- Windows and Unix native behavior.

### Filesystem tests

- symlink/FIFO/device/directory substitution;
- path traversal;
- race replacement where feasible;
- permission creation and repair;
- oversized input;
- archive extraction safety;
- lock nonce/PID reuse simulation.

### Protocol/crypto tests

- TLS policy;
- authentication failure;
- malformed/oversized frames;
- nonce uniqueness;
- wrong-key/tag failure;
- version mismatch;
- replay/stale behavior under documented model;
- no plaintext payload server-side.

### Execution tests

- every non-run command canary does not execute;
- shell insertion does not execute;
- preview/highlight/import/validate/sync do not execute;
- explicit run behavior and exit mapping;
- terminal restoration.

### Release tests

- package contents;
- Cargo/Homebrew/standalone detection;
- checksum mismatch rejection;
- archive traversal rejection;
- atomic update/rollback;
- previous-version upgrade fixtures;
- all release gates.

## Documentation

Update:

- `SECURITY.md`;
- threat model;
- sync privacy model;
- command execution warning;
- credential storage policy;
- plaintext HTTP restrictions;
- backup secret policy;
- update verification;
- vulnerability reporting;
- supported versions;
- release process;
- contributor security checklist;
- architecture process and trust boundaries.

## Recommended commit sequence

1. Update threat model and add sentinel-secret harness.
2. Audit/redact secret lifecycle and error/log surfaces.
3. Harden internal process environments and state-path validation.
4. Implement process-group/Job Object supervision if required.
5. Complete filesystem race/path-type/archive hardening.
6. Review protocol/crypto limits and add vectors/fuzz targets.
7. Audit all non-executing command paths with canary tests.
8. Harden self-update and release asset verification.
9. Add supply-chain policy and release gates.
10. Run full platform matrix and write closure evidence.

## Exit criteria

Phase 09 and the roadmap are complete only when:

- the threat model reflects the final architecture;
- secrets are absent from argv, state, logs, backups, diagnostics, and CI artifacts;
- internal subprocesses inherit only justified environment/handles;
- timeout supervision covers the actual process tree and releases lock only after termination;
- filesystem path-type, traversal, permission, and race defenses are tested;
- sync cryptographic/protocol behavior has bounded malformed-input tests;
- non-executing commands are proven not to execute canary snippets;
- standalone updates verify checksums and extract safely;
- dependency and workflow permissions are reviewed;
- fuzz/property smoke targets run;
- all release gates and supported platform jobs pass;
- no behavior-critical placeholder/TODO or ignored required test remains;
- final closure evidence is committed;
- project scope remains a lightweight terminal snippet manager with optional self-hosted encrypted sync.
