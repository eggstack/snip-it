# Phase 07A: Local Data Durability and Recovery

## Authority and baseline

This plan supersedes:

```text
plans/snip-it-correctness-07-local-data-durability-recovery.md
```

Begin after Phase 06A establishes clear ownership for core models and persistence primitives. Phase 05A must protect current storage, migration, and process behavior. Baseline implementation commit: `ff506f5934957c4fd989224a6f0e0cf10f907567`.

## Purpose

Strengthen `snip-it` as a local-first snippet manager by standardizing durable writes, defining stable identity and migration rules, and providing backup, restore, validation, and conservative repair workflows that do not depend on a sync server.

Synchronization is optional convenience and recovery transport. The local editable TOML data remains the primary user asset and must survive failed writes, malformed configuration, interrupted multi-file operations, disabled synchronization, and remote outages.

## Required outcomes

1. Every durable user-data write uses a reviewed atomic persistence path.
2. Failed writes preserve the last valid state.
3. Backup excludes secrets by default and is independently verifiable.
4. Restore supports dry-run, conflict reporting, rollback, and path safety.
5. Validation is comprehensive and strictly read-only.
6. Repair is conservative, backed up, idempotent, and refuses ambiguity.
7. Snippet/library identity behavior is explicit across edit, move, import, export, restore, and sync.
8. Historical migrations are idempotent and fixture-tested.
9. Multi-file operations record pending intent once after full local commit.
10. Unix, macOS, and Windows filesystem semantics are covered.

## Non-goals

Do not:

- replace editable TOML with SQLite or another opaque database;
- create a background backup service;
- include keychain secrets in ordinary backups;
- silently repair ambiguous data;
- make restore contact the server;
- use synchronization status as proof of local durability;
- add cloud backup.

---

## Workstream A — Persistence inventory and ownership

Create a committed inventory of every persisted artifact:

```text
snippet libraries
legacy single-file layout
library index and primary selection
general configuration
sync configuration
usage metadata
theme selection
pending/status/locks
logs
migration metadata
backup manifests/archives
restore journals
import/export outputs
```

For each artifact record:

- canonical path derivation;
- owner module/layer;
- schema/version;
- user-editable versus private;
- secret classification;
- durability class;
- maximum supported size;
- atomicity method;
- permissions/ACL expectation;
- symlink/non-regular-file policy;
- corruption handling;
- unknown-field policy;
- backup inclusion default;
- migration owner;
- synchronization relevance.

Create:

```text
docs/PERSISTENCE_INVENTORY.md
```

Use the inventory to find and remove bespoke `fs::write`/truncate-in-place behavior for durable user assets.

---

## Workstream B — Shared atomic persistence primitive

Implement one shared primitive for durable file replacement, owned by the core/persistence layer.

Recommended API:

```rust
pub struct AtomicWriteOptions {
    pub sensitivity: Sensitivity,
    pub durability: Durability,
    pub preserve_permissions: bool,
    pub max_bytes: u64,
    pub reject_symlink: bool,
}

pub fn atomic_replace(
    target: &Path,
    bytes: &[u8],
    options: AtomicWriteOptions,
) -> Result<AtomicWriteReport, PersistenceError>;
```

Required algorithm:

1. Resolve and validate the parent directory without accepting path traversal from untrusted relative paths.
2. Inspect the target with symlink-aware metadata.
3. Reject directories, FIFOs, sockets, devices, and unsafe symlink targets according to policy.
4. Create a uniquely named temporary file in the same directory with create-new semantics.
5. Apply restrictive permissions at creation time for sensitive content.
6. Write complete bytes.
7. Flush userspace buffers.
8. `sync_all()` when required by durability class.
9. Atomically replace the target using platform-correct behavior.
10. Sync parent directory where supported and justified.
11. Preserve or apply intended permissions.
12. Clean owned temporary files on recoverable failure.

### Durability classes

Define at least:

```text
DurableUserData
SensitiveConfig
RecoverableMetadata
EphemeralCoordination
```

Do not force expensive fsync semantics onto ephemeral locks if not required. Do not weaken snippet/library writes because status files are recoverable.

### Replacement report

Return a typed report containing whether the target existed, bytes written, permissions applied, parent sync support, and transient retry count. Do not expose secrets or file content.

---

## Workstream C — Windows/macOS/Unix replacement semantics

Add platform-specific behavior and tests for:

### Unix/macOS

- rename over existing regular file;
- directory fsync support and failure handling;
- mode preservation;
- symlink rejection;
- case-sensitive/case-insensitive filesystem differences where relevant.

### Windows

- replacement when target is briefly held by antivirus/indexer;
- exact transient sharing-violation filtering;
- bounded retry with deterministic maximum;
- target-open failure preserving original;
- path length/Unicode;
- case-insensitive collision;
- ACL/permission limitations documented;
- temporary file cleanup after failed replace.

Never retry all I/O errors indiscriminately. A permission denial, disk-full error, or invalid target type must fail promptly and preserve the original.

---

## Workstream D — Local mutation transaction boundary

Introduce a lightweight local mutation transaction for operations affecting multiple files:

```text
library create/delete/rename
primary-library changes
bulk import
restore
layout migration
repair
```

Preferred model:

1. Acquire a short local mutation transaction lock.
2. Load and validate all affected state.
3. Produce staged complete replacements.
4. Write a small private transaction journal/manifest only when multiple-file crash recovery requires it.
5. Commit files in a documented recoverable order.
6. Update indexes/config last according to rollback design.
7. Remove/complete journal.
8. Record exactly one pending generation if synchronized content changed.
9. Release transaction lock.
10. Request automatic scheduling once.

Do not introduce a database solely for transaction coordination.

### Transaction invariants

- no network operation inside the local transaction;
- no detached worker before commit;
- no pending generation per individual file;
- failed transaction leaves prior valid state or a journal sufficient for deterministic recovery;
- recovery never increments generation merely by inspecting/completing the transaction;
- sync-origin changes remain suppressed according to existing origin rules.

---

## Workstream E — Stable snippet and library identity contract

Document and enforce identity rules.

Recommended snippet rules:

- edit description/command/tags/output/favorite/folders retains ID;
- usage changes retain ID;
- moving between libraries retains a globally unique ID unless existing protocol semantics require library-scoped IDs;
- native export includes ID;
- Pet-compatible export may omit ID when format has no field;
- native reimport preserves ID when safe;
- Pet/import sources without IDs receive new IDs;
- same ID and identical content deduplicates;
- same ID and different content is an explicit conflict;
- exact duplicate content with different IDs follows a documented duplicate policy;
- restore retains IDs subject to collision rules;
- synchronization uses the same ID across devices;
- deletion semantics/tombstones are tied to stable identity.

Recommended library rules:

- library rename retains library identity if protocol supports it;
- display name and storage filename are distinct where required;
- restore collision behavior is explicit;
- primary selection references stable identity or a validated canonical name.

Use `SnippetId`/`LibraryId` newtypes if Phase 06A establishes them.

### Identity matrix tests

Commit a table covering:

```text
edit
move
copy
duplicate import
native export/import
Pet export/import
restore merge
restore replace
sync push/pull/merge
delete/recreate
library rename
```

---

## Workstream F — Validation command and diagnostic model

Add:

```bash
snp validate
snp validate --library <name>
snp validate --strict
snp validate --json
```

Validation is strictly read-only and local.

Detect:

- malformed TOML;
- unsupported/future schema;
- legacy fields requiring migration;
- missing/invalid/duplicate IDs;
- same-ID divergent content;
- exact duplicate entries;
- invalid timestamps/order;
- malformed variable/default/choice syntax;
- unknown fields and preservation policy;
- library index references to missing files;
- orphaned library files;
- invalid primary library;
- orphaned usage entries;
- non-regular/symlink paths;
- insecure permissions on sensitive files;
- size-limit violations;
- interrupted transaction journal;
- pending/status/lock problems through shared Phase 04A diagnostics.

Recommended diagnostic type:

```rust
pub struct ValidationDiagnostic {
    pub code: String,
    pub severity: Severity,
    pub path: Option<PathBuf>,
    pub library: Option<LibraryId>,
    pub snippet: Option<SnippetId>,
    pub message: String,
    pub repairability: Repairability,
}
```

Requirements:

- stable codes;
- deterministic ordering;
- no command execution;
- no network/keychain access unless explicitly required to validate sync config metadata without prompting;
- valid JSON on machine output;
- no mutation, migration, or normalization while validating.

---

## Workstream G — Backup format and snapshot semantics

Add:

```bash
snp backup
snp backup --output <path>
snp backup --include-usage
snp backup --include-config
snp backup --include-sync-state
snp backup --format directory|archive
snp backup --json
```

### Default contents

Include:

- all snippet libraries;
- library index/primary selection required for restore;
- schema metadata;
- manifest;
- per-file SHA-256;
- size and kind;
- tool version;
- creation timestamp;
- platform-independent relative paths.

Exclude by default:

- API keys/keychain contents;
- encryption keys/derived material;
- plaintext credential fallback;
- locks;
- logs;
- caches;
- temp files;
- pending/status unless explicitly requested;
- editor/shell environment.

### Format

Prefer a simple directory snapshot or standard compressed archive with an explicit manifest. Do not invent an opaque binary format.

Manifest example:

```toml
schema = 1
created_at_unix_ms = 0
snip_it_version = "1.x"
layout = "libraries"

[[files]]
path = "libraries/work.toml"
kind = "snippet_library"
size = 1234
sha256 = "..."
```

### Consistent snapshot

Use the local transaction lock or validated in-memory snapshots so all manifest entries correspond to one logical state. Backup remains read-only and must not record pending or schedule sync.

### Security

- archive paths normalized;
- no absolute paths;
- no symlink/device entries;
- output written atomically;
- destination overwrite requires explicit policy;
- manifest and output errors contain no secrets.

---

## Workstream H — Restore preview, merge, replace, and rollback

Add:

```bash
snp restore <backup> --dry-run
snp restore <backup> --merge
snp restore <backup> --replace
snp restore <backup> --json
```

Required preflight:

- validate archive/directory type;
- enforce total/file count and size limits;
- reject path traversal, absolute paths, symlinks, devices, FIFOs, sockets;
- validate manifest schema;
- verify every checksum before mutation;
- parse all included TOML;
- run identity/collision analysis;
- display planned additions, updates, removals, conflicts, and migrations.

### Merge semantics

Define exact handling for:

- same ID/same content;
- same ID/different content;
- different ID/same content;
- library collision;
- primary library;
- local-only metadata;
- usage metadata;
- unknown future fields.

Do not silently choose a conflict winner.

### Replace semantics

- requires explicit mode;
- automatic pre-restore backup;
- full validation before transaction;
- rollback if any required write fails;
- preserves excluded credentials/config unless explicit supported flags state otherwise;
- no server contact.

### Post-commit synchronization

If synchronized content changes and sync is configured:

- record one pending generation after full restore transaction;
- schedule once according to current auto-sync policy;
- no per-file pending increments;
- dry-run records nothing;
- failed restore records no new generation.

---

## Workstream I — Conservative repair

Add:

```bash
snp repair --dry-run
snp repair --apply
snp repair --library <name>
snp repair --json
```

Safe repair candidates:

- rebuild index from unambiguous valid libraries;
- repair invalid primary selection when exactly one clear replacement exists;
- quarantine owned orphan temp files;
- complete/rollback a known transaction journal;
- normalize supported legacy field names through migration;
- generate missing IDs only when identity is absent and no external identity conflict exists;
- remove orphaned usage entries;
- repair permissions;
- quarantine corrupt status through Phase 04A controls.

Ambiguous cases to refuse:

- same ID/different content;
- unsupported future schema;
- partially parseable corrupt TOML;
- corrupt pending intent;
- sync conflict requiring policy choice;
- credential/keychain state;
- multiple possible primary libraries;
- uncertain library identity.

Every applied repair:

- creates a pre-repair backup;
- emits a structured plan/report;
- uses the local transaction boundary;
- is idempotent;
- preserves unknown fields according to policy;
- records one pending generation only if synchronized content changed;
- does not automatically contact the server.

---

## Workstream J — Migration framework

Centralize migrations by explicit source and target schema/layout.

Recommended interface:

```rust
pub trait Migration {
    fn source(&self) -> SchemaVersion;
    fn target(&self) -> SchemaVersion;
    fn analyze(&self, input: &MigrationInput) -> Result<MigrationPlan, MigrationError>;
    fn apply(&self, plan: &MigrationPlan) -> Result<MigrationOutput, MigrationError>;
}
```

Required properties:

- load old format into typed representation;
- validate before write;
- report lossy transformations;
- backup original;
- atomic commit;
- idempotent second run;
- no rewrite when already canonical;
- exact command preservation under UTF-8 contract;
- supported unknown-field preservation;
- one pending generation after full migration;
- separate control-file schema migrations from user-library migrations.

Required fixtures:

- legacy `[[Snippets]]` spelling;
- capitalized/historical fields;
- single-file to library layout;
- missing IDs;
- historical timestamp/metadata forms;
- current canonical no-op;
- future unsupported schema refusal;
- malformed source preservation.

---

## Workstream K — Crash recovery and transaction journals

For any journaled operation, define startup behavior:

```text
journal absent -> normal
journal prepared, no replacements -> safe rollback/remove staged files
journal partially committed -> deterministic complete or rollback
journal committed, cleanup incomplete -> verify and clean
journal corrupt -> report and refuse automatic mutation
```

Rules:

- journal contains no snippet content unless unavoidable; prefer paths/checksums/state transitions;
- restrictive permissions;
- atomic journal updates;
- bounded age/size does not justify deleting a live/ambiguous journal;
- status/doctor exposes interrupted transaction;
- repair dry-run shows exact action;
- recovery never schedules sync before local transaction is consistent.

---

## Test plan

### Persistence tests

- successful atomic replace;
- write/sync/rename failure preserves original;
- unique temp paths under concurrency;
- target symlink/non-regular rejection;
- permission creation/preservation;
- Windows sharing retries only for expected errors;
- crash cleanup and journal recovery;
- parent directory sync behavior.

### Identity tests

- complete lifecycle matrix;
- collision combinations;
- edit/move retain identity;
- import/restore policies;
- sync consistency;
- deletion semantics.

### Validation/repair tests

- each diagnostic code;
- deterministic JSON;
- validation byte-for-byte read-only;
- repair dry-run byte-for-byte read-only;
- safe repairs idempotent;
- ambiguous repairs refused;
- automatic backup;
- one pending generation after successful repair.

### Backup/restore tests

- default contents and exclusions;
- sentinel secrets absent;
- checksum mismatch;
- path traversal/absolute/symlink/device rejection;
- total/file size limits;
- merge collision matrix;
- replace rollback at each failpoint;
- automatic pre-restore backup;
- cross-platform paths/Unicode;
- one pending generation and one scheduling request after commit.

### Migration tests

- every supported fixture;
- idempotent second run;
- canonical no-op;
- exact command preservation;
- failed migration preserves source;
- future schema refusal;
- transaction crash recovery.

---

## Recommended implementation sequence

1. Commit persistence inventory and shared atomic primitive.
2. Migrate existing durable user-data writers.
3. Add local mutation transaction lock/journal where needed.
4. Define identity contract and fixture matrix.
5. Add validation and stable diagnostics.
6. Add backup manifest/snapshot.
7. Add restore dry-run and conflict engine.
8. Add merge/replace transaction and rollback.
9. Add conservative repair.
10. Centralize migrations and crash recovery.
11. Complete platform tests/docs and write `plans/snip-it-correctness-07a-status.md`.

## Required verification

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --release --workspace
```

Add dedicated durability, backup/restore, validation/repair, and migration targets on Linux, macOS, and Windows.

## Exit criteria

Phase 07A is complete only when:

- durable user-data writes share one reviewed atomic path;
- failure preserves the previous valid state;
- multi-file operations have deterministic transaction/crash recovery;
- backup is consistent, checksummed, simple, and secret-free by default;
- restore is fully prevalidated, dry-run capable, conflict-aware, and rollback-safe;
- validation is comprehensive and read-only;
- repair is backed up, conservative, idempotent, and refuses ambiguity;
- identity behavior is documented and tested across the full lifecycle;
- migrations are explicit, idempotent, and fixture-covered;
- synchronized multi-file changes record exactly one pending generation after commit;
- Unix, macOS, and Windows filesystem tests pass;
- editable TOML remains canonical;
- no database, backup daemon, or cloud dependency was introduced.