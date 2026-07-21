# Phase 09A: Security, Release, and Program Closure

## Authority and baseline

This plan supersedes:

```text
plans/snip-it-correctness-09-security-release-hardening.md
```

Begin only after Phases 04A–08A are functionally complete and their status files identify no behavior-critical deferrals. Baseline implementation commit for the program is `ff506f5934957c4fd989224a6f0e0cf10f907567`; the final closure status must record all subsequent commit ranges.

## Purpose

Close the correctness program through a final threat-model review, secret and process audit, filesystem and protocol hardening, supply-chain and package verification, fuzz/property coverage, cross-platform release gates, upgrade evidence, and a committed closure record.

This phase validates the completed product. It must not be used to hide unfinished semantics from earlier phases or launch a broad redesign.

## Required outcomes

1. The final architecture has an explicit threat model and trust-boundary map.
2. Credentials, encryption material, variable assignments, and snippet payloads are absent from inappropriate process, file, log, backup, diagnostic, and CI surfaces.
3. Internal subprocesses use justified argv, environment, handle, and path behavior.
4. Timeout termination covers the real process boundary before lock release.
5. Filesystem operations reject unsafe path types and traversal.
6. Sync transport and cryptographic framing have explicit limits and vectors.
7. Non-executing commands are proven not to execute snippets.
8. Self-update/release assets are verified and rollback-safe.
9. Dependency, license, advisory, CI-permission, and provenance policies are enforced.
10. Required parsers/state machines have bounded fuzz/property coverage.
11. Linux, macOS, and Windows release candidates pass package/install/upgrade tests.
12. A final status document proves every phase complete and lists only non-blocking limitations.

## Non-goals

Do not:

- redesign the sync protocol without a separate compatibility plan;
- add a sandbox and claim arbitrary shell commands are safe;
- add mandatory signing infrastructure that blocks reasonable releases without an operational plan;
- introduce a daemon/service/plugin system;
- replace checksums with unverifiable trust claims;
- expand into remote execution or hosted service management.

---

## Security posture

`snip-it` deliberately stores and can execute user-authored shell commands. It is not a sandbox. The product security goals are:

- execute only after explicit user action;
- preserve exact user intent;
- protect local snippet data and credentials from accidental disclosure/corruption;
- prevent malformed imported/restored/remote state from causing unintended execution or silent loss;
- make detached synchronization bounded and truthful;
- make release/update artifacts verifiable;
- fail conservatively at trust boundaries.

Document same-user attacker limitations: CRC32 and file permissions detect corruption and reduce accidental exposure but do not authenticate state against a fully compromised user account.

---

## Workstream A — Final threat model

Create or update:

```text
SECURITY.md
docs/THREAT_MODEL.md
```

### Assets

- snippet commands/descriptions/tags/output/folders/favorites;
- stable IDs and timestamps;
- library/index configuration;
- usage metadata;
- sync credentials and credential revisions;
- encryption keys/derived material/nonces;
- encrypted remote payloads and metadata;
- pending intent and durable attempt status;
- locks, transaction journals, backups, logs;
- installed binary/update channel/release assets;
- shell/editor/clipboard boundaries.

### Trust boundaries

- user input and imported files;
- editable local TOML versus private control state;
- OS keychain/plaintext fallback;
- parent CLI to detached worker;
- worker to executor;
- client to sync server;
- local archive restore;
- editor/shell invocation;
- self-update/package manager;
- CI/release publishing.

### Threat actors and failures

- malicious Pet/TOML/import file;
- malicious backup archive;
- compromised/malformed sync server;
- network attacker under invalid TLS policy;
- same-account local process;
- malicious editor or shell config;
- symlink/FIFO/device substitution;
- PID reuse/stale locks;
- interrupted writes/process crashes;
- oversized/decompression input;
- compromised release asset/checksum metadata;
- accidental execution of an unsafe snippet;
- dependency/supply-chain compromise.

For each threat record:

- mitigation;
- residual risk;
- user responsibility;
- tests/evidence;
- owner/module.

---

## Workstream B — Secret and sensitive-data lifecycle audit

Trace values from creation/load through use/drop:

```text
API key
credential revision token
sync encryption password/key material
Argon2 output
AES key and nonce
variable assignments
snippet command/description
remote authorization metadata
```

Required assertions:

- no raw secret in CLI argv;
- no raw secret in worker/executor argv or process title;
- internal state-dir argv contains no sensitive payload;
- no secret in pending/status/locks/journals/temp filenames;
- no secret in tracing/logs/panics/error display;
- no secret in JSON diagnostics unless the command explicitly requests snippet content;
- ordinary backup excludes credentials/key material;
- CI artifacts contain no sentinel secret;
- help/examples do not encourage secrets in shell history;
- plaintext credential fallback is explicit, restrictive, and prominently documented;
- `Debug` implementations redact secret wrappers;
- sensitive buffers use `zeroize` where practical and meaningful;
- derived keys live only as long as needed;
- sanitized persisted errors remain bounded.

### Sentinel harness

Use unique test sentinels for:

- API key;
- encryption password;
- variable assignment;
- snippet command/description;
- URL credential.

Scan:

- stdout/stderr;
- logs;
- pending/status/locks/journals;
- backup/archive;
- doctor/status/validate JSON;
- test event files;
- CI artifacts;
- package contents where relevant.

A sentinel match in an unauthorized surface is release-blocking.

---

## Workstream C — Internal process spawning and environment hardening

Audit:

- worker re-exec;
- executor re-exec;
- shell execution;
- editor invocation;
- update/package-manager commands;
- clipboard helper paths if any.

### Worker/executor requirements

- direct `current_exe` invocation, no shell;
- arguments as OS strings;
- hidden subcommands;
- state path validation;
- stdin/stdout/stderr inheritance exactly documented;
- no unnecessary terminal handles;
- no raw credentials/variables in argv;
- inherited environment minimized through a reviewed denylist or allowlist strategy;
- preserve required HOME/XDG/platform paths, TLS roots, and test controls only when appropriate;
- production cannot enable dangerous test failpoints;
- spawn failure preserves pending;
- executable replacement/update race has defined behavior;
- working directory does not change correctness.

Do not blindly `env_clear()` if it breaks keychain/TLS/platform behavior. Record every inherited sensitive variable class and test the chosen policy.

### Shell/editor requirements

- shell is used only for explicit snippet execution;
- editor invoked directly with safely split configured command/args;
- no snippet execution through editor parsing;
- exact current-directory behavior documented;
- command/variable values not logged;
- terminal restored after failure/signals.

---

## Workstream D — Process-group/job termination boundary

Determine whether executor sync can create descendants now or after supported TLS/keychain/platform integrations.

### If descendants are impossible by design

- document the constraint;
- test executor child tree remains single-process during controlled sync;
- preserve direct-child termination/reap semantics.

### If descendants are possible

Unix:

- executor in a dedicated process group/session appropriate to supervision;
- timeout sends termination to group;
- grace period;
- force kill group;
- direct child reaped;
- no known descendant remains;
- lock released last.

Windows:

- place executor in a Job Object with kill-on-close or explicit termination;
- close handles safely;
- verify all assigned processes exit;
- lock released after confirmation.

Required tests:

- child ignores graceful termination;
- descendant/child remains active until group/job kill;
- pending preserved;
- lock lifetime covers all work;
- later retry succeeds;
- no zombie/orphan/console leak.

Documentation must describe the actual boundary, not a stronger cancellation claim.

---

## Workstream E — Filesystem and path hardening

Apply one policy across:

- libraries/config/usage;
- pending/status/locks/journals;
- logs;
- backup/restore;
- import/export;
- temp files;
- update downloads/extraction.

Requirements:

- symlink-aware metadata;
- reject directories/FIFOs/sockets/devices where regular files required;
- path traversal and absolute path rejection for untrusted archives;
- create sensitive files with restrictive permissions from first open;
- unique create-new temp files in same directory for atomic replace;
- size/count limits before allocation/extraction;
- safe Unicode/non-UTF-8 handling according to format;
- no check-then-open race where an open-handle/flag solution is practical;
- ownership-checked lock removal;
- live lock never stolen by age alone;
- PID reuse mitigated by nonce/start identity where feasible;
- Windows reparse-point and sharing behavior tested;
- quarantine paths cannot escape intended root;
- backup/restore extraction never follows archive links.

Add race-oriented tests for target replacement between validation and open where deterministic platform primitives permit.

---

## Workstream F — Sync transport and protocol review

Review and document:

- server URL parsing/normalization;
- TLS requirement;
- loopback HTTP development exception;
- hostname/SNI/certificate validation;
- custom CA behavior if supported;
- API-key metadata transport;
- registration and credential replacement;
- request/response size limits;
- gRPC deadlines and executor timeout interaction;
- decompression/allocation bounds;
- protocol version handling;
- replay/stale revision behavior;
- malformed/unknown fields;
- remote error-body sanitization;
- server metadata leakage.

Required tests:

- reject non-loopback plaintext remote URL;
- accept documented loopback development mode;
- wrong hostname/certificate failure;
- authentication failure redaction;
- oversized/truncated/malformed response;
- version mismatch;
- deadline/timeout classification;
- server restart/persistence;
- compromised server cannot obtain plaintext snippet data under stated encryption model.

Do not casually make incompatible protocol changes. Clear bugs may be fixed with versioning/migration evidence; broader changes require a separate plan.

---

## Workstream G — Cryptographic implementation review

Audit:

- Argon2id parameters, salts, and versioning;
- AES-256-GCM key use;
- nonce generation/uniqueness;
- associated data;
- ciphertext framing/version;
- authentication tag failure;
- key/password handling and zeroization;
- random source failures;
- encrypted payload size limits;
- metadata left visible to server;
- error classification/redaction.

Required evidence:

- known deterministic test vectors where appropriate;
- round-trip across supported versions;
- wrong key failure;
- modified tag/ciphertext failure;
- truncated/oversized frame rejection;
- nonce uniqueness stress/property test;
- empty/large/multiline/Unicode payloads;
- no plaintext sentinel in server database or request capture;
- migration/version compatibility.

If review identifies a cryptographic design limitation requiring protocol change, document it as release-blocking or a separate versioned follow-up; do not silently defer a critical flaw.

---

## Workstream H — Explicit execution safety audit

Audit all commands/actions against the Phase 08A contract.

Non-executing surfaces must include:

```text
list
get
status
doctor
validate
backup
restore --dry-run
repair --dry-run
import preview/dry-run
export
search preview
select print/insert
sync
```

Requirements:

- only `run` or a clearly labeled TUI execute action invokes the shell;
- insertion into a shell prompt does not execute;
- parsing/highlighting/variable analysis never executes;
- import/restore validation never executes;
- sync treats commands as encrypted/text data;
- variable expansion is textual only;
- no implicit command substitution evaluation;
- TUI execute key distinct from copy/print;
- confirmation where execution target is ambiguous;
- terminal restoration on panic/error/signal.

Use canary snippets that would create a file/network request if executed. Assert no canary effect for every non-executing path.

---

## Workstream I — Backup/restore security review

Validate Phase 07A implementation:

- manifest/checksum authenticity scope documented;
- SHA-256 verification before mutation;
- path normalization/traversal rejection;
- symlink/device/FIFO/socket rejection;
- compressed-size and expanded-size limits;
- file-count/depth limits;
- no credentials included by default;
- explicit include-sync-state does not include raw credentials;
- pre-restore backup;
- transaction rollback;
- quarantine paths safe;
- conflict report cannot inject terminal control sequences in machine mode;
- archive names handled across platforms.

Fuzz archive manifest/path handling and test common zip/tar traversal patterns according to adopted format.

---

## Workstream J — Self-update and distribution hardening

Audit every `snp update` path:

```text
Cargo-installed
Homebrew-installed
standalone release archive
```

Standalone requirements:

- HTTPS only;
- exact platform/architecture asset selection;
- download to private temp path;
- verify SHA-256 from expected checksum metadata;
- checksum parser selects exact filename and rejects ambiguity;
- archive traversal/symlink entries rejected;
- extracted binary is a regular file;
- optional signature/provenance verification if operationally supported;
- preserve old binary until new asset verifies;
- atomic replacement where permissions allow;
- rollback/clear failure behavior;
- no automatic `sudo`;
- refuse replacement of an unexpected executable path;
- hidden worker/executor current-exe behavior remains correct after update;
- concurrent running worker/update behavior is documented and tested.

Cargo/Homebrew paths:

- direct argument invocation, no unsafe shell construction;
- clear detection and opt-in behavior;
- package-manager errors mapped cleanly;
- no credential/environment leakage.

Required tests:

- checksum mismatch;
- missing/ambiguous checksum;
- wrong asset;
- archive traversal;
- interrupted download;
- failed replacement rollback;
- installed-path smoke test;
- update while worker exists according to policy.

---

## Workstream K — Dependency, license, and supply-chain policy

Add/verify:

- committed lockfile and locked release builds;
- `cargo deny` advisories/licenses/bans/sources;
- documented advisory exception format with owner/rationale/review date;
- duplicate dependency review, especially crypto/TLS/windows stacks;
- no unexpected git/path dependencies in published artifacts;
- license compatibility for themes/assets;
- minimal GitHub Actions permissions;
- pinned action versions or commit SHAs according to project policy;
- protected release environment;
- scoped crates.io/GHCR tokens;
- dependency update automation;
- SBOM generation;
- build provenance/attestation where supported;
- reproducible build notes or comparison where practical.

Do not automatically fail on every advisory without an exception mechanism, but do not silently ignore advisories.

---

## Workstream L — Fuzzing and property tests

Add focused targets for untrusted parsing/state transitions:

```text
snippet TOML parser
legacy/Pet migration
variable/default/choice parser
selector/query normalization
pending/status/lock/journal parser
backup manifest/path validation
encryption frame parser
sync merge input
server URL/config parser
```

Required properties:

- no panic;
- bounded allocation/time for configured limits;
- valid round-trip stability;
- migration idempotency;
- invalid input never executes commands;
- corruption never maps to success/current;
- conditional clear never removes newer generation;
- normalized archive paths remain under root;
- same input produces deterministic diagnostics/order;
- encryption tampering fails closed.

CI:

- bounded fuzz/property smoke job;
- longer local/nightly commands documented;
- retain minimized regression corpus;
- no required release invariant depends solely on a long-running external fuzz service.

---

## Workstream M — CI and release gates

Required commands:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --release --workspace
cargo package --workspace
cargo deny check
```

Also require:

- supported minimal/default/all feature builds;
- docs/doctests;
- Phase 05A real-server/process/crash suites;
- Phase 07A backup/restore/repair/migration suites;
- Phase 08A output/shell/non-execution suites;
- Linux/macOS/Windows success;
- package/install smoke tests;
- secret-sentinel scan;
- fuzz/property smoke;
- release checksum/update verification;
- upgrade from previous stable release fixtures;
- no behavior-critical ignored tests;
- no placeholder success/TODO/unimplemented path in critical modules;
- no test-support controls in package/release binaries.

Use an automated critical-source placeholder scan as a supplement, not the sole review.

### CI artifact policy

- upload only on failure where useful;
- sanitize and sentinel-scan before upload;
- bounded retention;
- no raw credential store/database containing plaintext secrets;
- document artifact contents.

---

## Workstream N — Package, install, and upgrade matrix

Verify:

```text
cargo package/install
Homebrew package/formula path
standalone Linux archive
standalone macOS archive
standalone Windows archive
```

For each:

- install fresh;
- help/version/completions;
- local create/list/get/run dry-safe smoke;
- hidden worker/executor re-exec;
- auto-sync against ephemeral server;
- backup/validate/status;
- uninstall/upgrade behavior;
- path with spaces/Unicode where applicable.

Upgrade fixtures from prior stable releases must cover:

- legacy single-file layout;
- library layout;
- sync config with missing new fields;
- pending/status schema migration;
- existing keychain/plaintext fallback policy;
- user-edited TOML with legacy fields;
- rollback or source preservation on failed migration.

No release while a supported platform package path lacks smoke evidence.

---

## Workstream O — Documentation reconciliation

Review all user and architecture documents for final behavior:

- README;
- USER_GUIDE;
- command help/completions;
- architecture overview/deep dives;
- PUBLIC_API/semver policy;
- persistence/identity/migration docs;
- status/recovery/doctor docs;
- JSON/exit-code contracts;
- shell integration;
- security/threat model;
- sync privacy/TLS/credential policy;
- backup/restore secret policy;
- update verification;
- release process;
- supported versions/vulnerability reporting;
- contributor checklist.

No documentation may claim:

- remote failure changes a completed mutation exit retroactively;
- timeout cancels in-process unkillable work;
- missing/corrupt state means current;
- CRC32 authenticates against a malicious local actor;
- non-executing commands execute;
- a daemon/service exists;
- unsupported CLI flags exist.

---

## Workstream P — Final closure status

Create:

```text
plans/snip-it-correctness-program-closure-status.md
```

Required contents:

### Program summary

- initial critical defect and final architecture;
- Phase 01–03 corrective baseline;
- Phase 04A–09A completion table;
- explicit no-daemon/single-binary statement.

### Commit evidence

- commit ranges per phase;
- migrations/config/API/CLI changes;
- release version/tag candidate.

### Test evidence

- test counts by category;
- exact critical invariants proven;
- Linux/macOS/Windows CI links/status;
- package/install/upgrade evidence;
- fuzz/property targets and corpus;
- secret-sentinel results;
- cargo deny/dependency/license results;
- performance/startup comparison.

### Security evidence

- threat-model completion;
- credential/process/filesystem/protocol/crypto/update audit outcomes;
- known residual risks;
- advisory exceptions with dates.

### Limitations

- documented non-blocking limitations;
- separately filed future enhancements;
- no unresolved behavior-critical TODO.

The closure document must not claim evidence that was not actually obtained.

---

## Recommended implementation sequence

1. Finalize threat model and sentinel harness.
2. Audit/redact secret lifecycle.
3. Harden internal process environment/handles/path validation.
4. Confirm or implement process-group/Job Object supervision.
5. Complete filesystem/path/archive hardening.
6. Review transport and cryptographic implementation with tests/vectors.
7. Complete non-execution canary audit.
8. Harden self-update/distribution.
9. Add supply-chain, fuzz, and release policies.
10. Run package/install/upgrade matrix.
11. Reconcile documentation.
12. Run full release gates and write closure status.

## Release-blocking conditions

Do not close/release while any is true:

- a successful sync path can avoid server effect;
- pending clears on non-success;
- a timeout releases lock while executor work remains;
- corrupt state appears current;
- a required platform test is skipped without equivalent evidence;
- secret sentinel appears in unauthorized output/artifact;
- restore/update archive traversal is possible;
- checksum/update verification is incomplete;
- non-executing command can execute a snippet;
- package/upgrade path is untested on a supported platform;
- required migration can destroy/overwrite source on failure;
- documentation contradicts implementation;
- critical TODO/unimplemented/placeholder remains;
- a prior phase status identifies an unresolved blocking item.

## Exit criteria

Phase 09A and the complete correctness program are closed only when:

- final threat model reflects the shipped architecture;
- secrets and sensitive payloads are absent from unauthorized surfaces;
- process and timeout boundaries are truthful and platform-tested;
- filesystem/archive/update paths are hardened;
- protocol/crypto implementation has explicit limits and evidence;
- non-execution canaries pass;
- supply-chain/advisory/license policies pass;
- fuzz/property smoke and regression corpus pass;
- all release gates pass on Linux, macOS, and Windows;
- package/install/upgrade evidence is committed;
- documentation is reconciled;
- `plans/snip-it-correctness-program-closure-status.md` records real evidence and only non-blocking limitations;
- no daemon, resident client service, plugin runtime, workflow engine, remote execution feature, or second installed helper binary was introduced.