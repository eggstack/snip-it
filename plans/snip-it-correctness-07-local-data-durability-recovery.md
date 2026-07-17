# Phase 07: Local Data Durability and Recovery

## Purpose

Strengthen the local-first guarantee by standardizing persistence, adding backup and validation workflows, defining stable identity behavior, and making migrations and repairs conservative and reversible.

Synchronization correctness does not replace local durability. A snippet manager must remain trustworthy when the server is unavailable, synchronization is disabled, configuration is malformed, or a process is interrupted during a write.

## Preconditions

This phase should begin after the core architecture boundaries from Phase 06 are stable enough that persistence primitives have a clear owner.

The Phase 05 harness must protect current user-visible storage and migration behavior.

## Durability principles

1. A successful local mutation survives process exit and ordinary system interruption.
2. Failed writes preserve the last valid file.
3. Backup and validation do not require a sync server.
4. Repair never invents synchronized state or silently discards ambiguous data.
5. User-editable TOML remains canonical and understandable.
6. Migration is idempotent and preserves all supported metadata.
7. Snippet identity has documented behavior across edit, move, import, export, restore, and sync.
8. Sensitive sync credentials are excluded from ordinary backups by default.

## Workstream A: Inventory all persistence paths

Identify every write path for:

- legacy single-file snippets;
- named library TOML files;
- library index/primary-library configuration;
- general settings;
- sync settings;
- usage metadata;
- theme selection;
- pending/status/lock artifacts;
- import/export destinations;
- backups;
- migration markers or schema metadata.

For each path, document:

- owner module;
- schema/version;
- atomicity mechanism;
- permissions;
- durability requirement;
- corruption behavior;
- symlink behavior;
- whether unknown fields are preserved;
- whether it contains secrets;
- backup inclusion policy.

Use the inventory to eliminate one-off write implementations.

## Workstream B: Standardize atomic file replacement

Create a shared, well-tested atomic persistence primitive for durable user data.

Recommended sequence:

1. Validate target parent directory.
2. Refuse unsafe non-regular target conditions according to policy.
3. Create a uniquely named temporary file in the same directory.
4. Apply intended restrictive permissions before writing sensitive content.
5. Write complete bytes.
6. Flush userspace buffers.
7. Call `sync_all()` for files that require durability.
8. Atomically rename/replace the target using platform-correct behavior.
9. Sync parent directory where supported and justified.
10. Verify or preserve intended permissions.
11. Clean temporary artifacts conservatively.

The API should allow policy choices:

```rust
pub struct AtomicWriteOptions {
    pub sensitivity: Sensitivity,
    pub sync_file: bool,
    pub sync_parent: bool,
    pub preserve_permissions: bool,
    pub max_bytes: u64,
}
```

Do not use a universal expensive durability mode for ephemeral status/lock files without analysis. Distinguish:

- durable user data;
- sensitive configuration;
- recoverable metadata;
- ephemeral coordination artifacts.

## Workstream C: Cross-platform replacement semantics

Test and document differences:

- Unix rename over existing file;
- Windows replacement behavior when target is open;
- antivirus/indexer contention;
- filesystem permissions and ACL limitations;
- directory synchronization support;
- path length and Unicode;
- case-insensitive collisions;
- temporary-file cleanup after failed replace.

Use bounded retries only for known transient Windows sharing violations, with exact error filtering and no silent indefinite loop.

## Workstream D: Add backup commands

Recommended surface:

```bash
snp backup
snp backup --output <path>
snp backup --include-usage
snp backup --include-sync-state
snp backup --include-config
snp backup --json
```

Default backup should include:

- all snippet libraries;
- library configuration required to restore layout;
- a manifest;
- schema versions;
- checksums;
- tool version and creation timestamp.

Default backup should exclude:

- API keys;
- encryption keys;
- keyring contents;
- transient locks;
- logs;
- pending/status state unless explicitly requested;
- caches;
- temporary files.

Choose a simple format, such as a directory or compressed archive with an explicit manifest. Avoid a custom opaque binary container.

Manifest example:

```toml
schema = 1
created_at = "..."
snip_it_version = "..."
config_layout = "libraries"

[[files]]
path = "libraries/work.toml"
sha256 = "..."
size = 1234
kind = "snippet_library"
```

Backup must use a consistent snapshot strategy. If multiple files can change concurrently, either acquire a short local data lock or copy through validated in-memory snapshots so the manifest and files correspond.

## Workstream E: Add restore with preview and rollback

Recommended surface:

```bash
snp restore <backup>
snp restore <backup> --dry-run
snp restore <backup> --merge
snp restore <backup> --replace
```

Required behavior:

- validate manifest and checksums before changes;
- reject path traversal and absolute paths;
- reject symlink/device/FIFO archive entries;
- validate schemas and size limits;
- show planned additions/replacements/conflicts;
- create an automatic pre-restore backup;
- use atomic writes;
- preserve original data if any required step fails;
- never import credentials implicitly;
- define merge versus replace semantics precisely;
- update pending sync intent if restored local content differs from current synchronized state;
- avoid triggering a detached worker until restore transaction commits.

Restore should be explicit about whether snippet IDs from the backup are retained and how collisions are handled.

## Workstream F: Add validation command

Recommended surface:

```bash
snp validate
snp validate --library work
snp validate --json
snp validate --strict
```

Validation should detect:

- malformed TOML;
- unsupported schema versions;
- legacy aliases requiring migration;
- missing or duplicate IDs;
- invalid UUIDs;
- duplicate exact entries;
- invalid timestamps;
- malformed variables/choice syntax;
- unknown metadata and whether it is preserved;
- library index entries with missing files;
- orphaned library files;
- invalid primary library;
- orphaned usage metadata;
- unsafe permissions on sensitive files;
- symlink/non-regular-file substitution;
- size-limit violations;
- pending/status corruption via integration with doctor.

Validation is read-only. JSON diagnostics should share stable diagnostic codes with doctor where possible.

## Workstream G: Add conservative repair

Recommended surface:

```bash
snp repair --dry-run
snp repair --apply
snp repair --library work
```

Safe repair candidates:

- rebuild library index from unambiguous valid files;
- remove references to nonexistent libraries after confirmation;
- quarantine malformed temporary files;
- regenerate missing IDs where no external identity exists, with explicit report;
- normalize supported legacy field spelling while preserving values;
- repair permissions;
- remove orphaned usage entries;
- preserve unknown fields according to parser capability.

Ambiguous cases must not be auto-repaired:

- conflicting duplicate IDs with different content;
- unsupported future schema;
- partially valid corrupt TOML where data interpretation is uncertain;
- sync conflicts;
- credential state;
- pending intent with integrity failure.

Every applied repair must:

- create a backup;
- emit a structured report;
- be idempotent;
- use atomic writes;
- avoid automatic sync until full repair transaction commits;
- mark changed local libraries pending afterward if sync is configured.

## Workstream H: Define stable snippet identity

Write and enforce a lifecycle contract for snippet IDs.

Recommended rules:

- editing description/command/tags/output retains ID;
- changing local-only metadata retains ID;
- moving between libraries either retains ID globally or uses a documented library-scoped identity model;
- export includes ID only in snip-it-native formats, not necessarily Pet-compatible output;
- reimport of a native export preserves IDs when safe;
- Pet import assigns new IDs because source has none;
- exact duplicate merge does not create an additional ID;
- collision with same ID/same content deduplicates;
- collision with same ID/different content is an explicit conflict;
- restore retains IDs subject to collision policy;
- sync uses IDs consistently across devices;
- deletion tombstones or delete semantics have a stable identity relationship.

Add a `SnippetId` newtype if Phase 06 architecture makes it appropriate.

## Workstream I: Migration framework

Centralize migrations by schema/layout version rather than scattered load-time rewrites.

Required migration properties:

- read old format into typed representation;
- validate before mutation;
- write through atomic persistence;
- retain backup of original;
- record source and target schema;
- idempotent second run;
- no unnecessary rewrite when already canonical;
- preserve commands exactly where promised;
- preserve unknown fields when compatibility policy requires it;
- report lossy transformations before applying;
- avoid triggering one worker per migrated file; schedule once after transaction.

Cover:

- legacy `[[Snippets]]` spelling;
- capitalized fields;
- legacy single-file to libraries layout;
- historical snip-it metadata versions;
- pending/status schema migrations separately.

## Workstream J: Local transaction boundaries

Multi-file operations such as restore, layout migration, bulk import, or library rename need an explicit transaction strategy.

A lightweight approach:

1. Acquire a short local mutation transaction lock.
2. Load and validate all affected files.
3. Write staged replacement files.
4. Commit files in an order with recoverable journal/manifest.
5. Update index/primary config last or according to rollback design.
6. Record pending generation once after successful transaction.
7. Release lock.
8. Schedule auto-sync once.

Avoid introducing a database solely for local transactions. A small private journal may be justified for crash recovery of multi-file operations.

## Required tests

### Atomic persistence

- successful replacement;
- write failure preserves old file;
- rename failure preserves old file;
- unique temp files under concurrency;
- permissions;
- symlink/non-regular target handling;
- Windows transient sharing behavior;
- parent sync behavior where supported;
- cleanup after crash simulation.

### Backup/restore

- complete default backup;
- secrets excluded;
- manifest/checksum validation;
- path traversal rejection;
- dry-run no mutation;
- merge/replace semantics;
- automatic pre-restore backup;
- rollback on mid-restore failure;
- ID collision cases;
- restored differences create one pending generation;
- cross-platform archive paths.

### Validation/repair

- each diagnostic class;
- stable JSON codes;
- read-only validation;
- safe repair idempotency;
- ambiguous cases refused;
- backup before repair;
- permission repair;
- orphan handling;
- no secret output.

### Identity/migration

- edit/move/import/export/restore/sync ID behavior;
- collision matrix;
- every supported historical fixture migrates;
- second migration makes no changes;
- exact command bytes preserved;
- unknown metadata policy enforced;
- failed migration leaves source intact.

## Documentation

Add or update:

- local durability model;
- backup and restore guide;
- validation/repair guide;
- snippet identity contract;
- migration and compatibility policy;
- backup secret-exclusion policy;
- crash-recovery behavior;
- platform-specific filesystem caveats;
- JSON diagnostic codes.

## Recommended commit sequence

1. Inventory writes and add shared atomic persistence primitive.
2. Migrate existing user-data writes to the primitive.
3. Define identity contract and add collision/migration fixtures.
4. Add validation command and stable diagnostics.
5. Add backup manifest and snapshot implementation.
6. Add restore dry-run, merge/replace, rollback.
7. Add conservative repair with automatic backup.
8. Add multi-file transaction/journal support where required.
9. Complete cross-platform fault tests.
10. Reconcile documentation and closure evidence.

## Exit criteria

Phase 07 is complete only when:

- all durable user-data writes use a reviewed atomic persistence path;
- failed writes preserve the last valid state;
- backup excludes secrets by default and verifies checksums;
- restore supports dry-run, rollback, and collision handling;
- validation is comprehensive and read-only;
- repair is conservative, backed up, and idempotent;
- snippet identity is documented and tested across all lifecycle operations;
- migrations are idempotent and preserve supported data;
- multi-file operations record one pending generation after commit;
- Unix, macOS, and Windows filesystem tests pass;
- user documentation provides a complete recovery workflow.
