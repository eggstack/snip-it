# Phase 14B — Persistence Fail-Closed Behavior and Stable Snippet Identity

Status: IMPLEMENTED

Parent roadmap: `plans/snip-it-phase-14-correctness-simplification-roadmap.md`

Reviewed code baseline: `f0ebd1a2246976217bf48260c2dbddd31163533d`

Date: 2026-08-08

Execution target: smaller coding models operating sequentially with narrow context.

## 1. Purpose

This phase addresses two local-data correctness issues:

1. malformed persistent TOML is currently backed up and then converted into default/empty in-memory state, which can later be written over the damaged file;
2. missing or duplicate snippet IDs are repaired with fresh random UUIDs every time the file is loaded, so read-only commands can observe different identities across invocations.

The desired contract is simple:

- malformed files fail closed and remain untouched except for a recovery backup;
- legacy valid snippet files remain readable without requiring an immediate write;
- any synthesized legacy IDs are deterministic across repeated loads;
- the next legitimate library mutation persists those IDs naturally through the existing save path.

Do not add a database, schema service, migration daemon, or a new journal mechanism.

## 2. Allowed files

Primary files:

```text
src/library.rs
src/commands/mod.rs
src/error.rs                 # only if a typed parse/corruption error is needed
src/migration.rs             # only if existing migration ownership clearly fits
```

Focused tests should remain colocated in `src/library.rs` when practical.

Potential documentation after behavior is final:

```text
USER_GUIDE.md
architecture/persistence.md
architecture/overview.md
AGENTS.md
```

Do not change sync protocol, protobufs, SQLite schema, transaction semantics, or backup format.

## 3. Workstream A — Make malformed library TOML fail closed

### 3.1 `load_library()`

Current behavior:

```text
parse error
  -> create .corrupt.bak
  -> log error
  -> return Snippets::default()
```

Required behavior:

```text
parse error
  -> best-effort create .corrupt.bak
  -> return the original parse error as SnipError
  -> do not synthesize an empty writable library
```

The backup failure must not hide the original parse error. Log the backup failure and still return the parse failure.

Do not modify the source file while handling the parse error.

### 3.2 `LibraryManager::new()`

Apply the same fail-closed rule to malformed `libraries.toml`.

Current behavior substitutes `LibraryConfig::default()` after backing up the malformed file. Replace that with an error return so commands cannot proceed against an empty synthetic index.

Missing `libraries.toml` remains valid and must still produce a default empty manager. Only an existing malformed file becomes an error.

### 3.3 Keep legacy escape repair behavior

The existing `fix_invalid_toml_escapes()` compatibility pass is separate from malformed-file handling. Retain it unless a focused test proves it causes the parse corruption under review.

Do not remove Pet compatibility in this phase.

## 4. Workstream B — Align legacy single-file loading

`commands::load_snippets()` already returns a parse error after creating a backup. Keep that fail-closed behavior.

After Workstream A, verify that all three persistence entry points have a consistent rule:

```text
missing file       -> empty/default is valid
empty file         -> empty/default is valid
valid legacy TOML  -> parse and normalize
malformed TOML     -> backup + error
```

Do not introduce a separate corruption policy per command.

## 5. Workstream C — Audit ID assumptions before changing normalization

Before implementing deterministic IDs, search for code that assumes `Snippet.id` is syntactically a UUID rather than an opaque stable identifier.

Check at minimum:

```text
src/selector.rs
src/sync.rs
src/sync_commands.rs
src/commands/**
snip-sync/src/**
```

Search for:

```text
Uuid::parse_str
uuid::Uuid::parse_*
fixed UUID length checks
hyphen-position checks
```

If no code requires UUID syntax, use an opaque deterministic legacy ID as described below.

If UUID syntax is required by a real production path, stop before adding a dependency. Prefer deriving a deterministic UUID-shaped value from existing SHA-256 primitives or, only if materially simpler, enabling the smallest `uuid` feature needed for deterministic UUID generation and measuring its cost in Phase 14D.

## 6. Workstream D — Replace random read-time ID repair with deterministic normalization

### 6.1 Required behavior

For each loaded valid library:

- existing unique non-empty IDs remain unchanged;
- missing IDs receive a deterministic provisional ID;
- for duplicate explicit IDs, the first occurrence keeps the original ID and later occurrences receive deterministic replacement IDs;
- repeated loads of identical file content produce identical IDs;
- loading alone does not write the library file;
- the next normal `save_library()` persists the normalized IDs because they are already present in the in-memory `Snippets` object.

### 6.2 Preferred implementation

Use the already-present `sha2` dependency; do not add a hashing crate.

Create one private normalization helper in `src/library.rs`, conceptually:

```rust
fn normalize_snippet_ids(snippets: &mut [Snippet])
```

A suitable opaque ID shape is:

```text
legacy-<full lowercase sha256 hex>
```

For a missing ID, hash a domain-separated canonical representation of the snippet's stable user data. Include enough fields to avoid common accidental collisions, for example:

```text
"snip-it-legacy-id-v1\0"
description
"\0"
command
"\0"
tags in stored order
"\0"
output
```

For multiple snippets with identical user content and no ID, include the occurrence number among identical base fingerprints so each receives a unique deterministic ID.

For a duplicate explicit ID after the first occurrence, include the duplicated ID plus the snippet fingerprint and duplicate occurrence number under a different domain separator such as `snip-it-duplicate-id-v1`.

The exact serialization may differ, but it must be deterministic and covered by tests.

Do not use wall clock, process ID, random UUIDs, filesystem mtime, or device ID as repair inputs.

### 6.3 Why provisional IDs are acceptable

Read-only commands must not rewrite user TOML merely to repair metadata. Deterministic provisional IDs allow:

- stable exact selection;
- stable usage-index keys across repeated reads;
- stable sync identity if a legacy library is read before its next mutation;

while preserving the local-first editable-file model.

Once any legitimate mutation saves the library, the same IDs become durable fields in the TOML.

## 7. Workstream E — Ensure save behavior naturally persists normalization

Do not add a special migration write path if the existing save path already serializes the normalized in-memory IDs.

Required proof:

1. load legacy ID-less library;
2. record generated deterministic IDs;
3. perform or simulate a normal mutation using the ordinary save path;
4. reload;
5. assert the same IDs are now explicitly stored and remain unchanged.

Do not trigger a save solely because a read detected missing IDs.

## 8. Focused tests

Add or update colocated tests for:

### Malformed data

- malformed library returns `Err` and creates `.corrupt.bak`;
- malformed `libraries.toml` causes `LibraryManager::new()` to return `Err` and creates its backup;
- missing and empty files remain valid empty/default state;
- backup-write failure does not transform the original parse failure into success.

### Identity

- one missing ID is identical across two independent loads of the same file;
- two content-distinct missing-ID snippets receive distinct IDs;
- two identical missing-ID snippets receive distinct but repeatable IDs using occurrence ordering;
- first occurrence of a duplicate explicit ID keeps it;
- later duplicate IDs receive stable replacements;
- valid unique IDs remain byte-for-byte unchanged;
- normal save persists provisional IDs;
- reload after persistence does not generate different IDs;
- ID length stays below the server/client maximum.

If exact selection or usage tracking has an existing integration test file, add one regression proving a legacy ID-less snippet retains the same identity across separate command loads. Do not create a large new process harness solely for this.

## 9. Compatibility constraints

Preserve:

- canonical Pet `[[snippets]]` parsing;
- older `[[Snippets]]` aliases;
- older capitalized field aliases;
- editable TOML format;
- current sort/save behavior unless a test proves ID persistence is affected;
- user-provided IDs.

Do not require users to run `snp repair` just to read a valid legacy ID-less Pet file.

Malformed files are different: those must now stop normal operation until repaired.

## 10. Routine verification

After focused tests:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p snip-it library
bash scripts/check.sh
```

Do not run crash-recovery release suites solely because library parsing changed, unless the implementation touches transaction code contrary to this plan.

## 11. Stop conditions

Stop and amend the plan if:

- a production path truly requires UUID syntax and a deterministic solution cannot be implemented cleanly with existing dependencies;
- returning an error from malformed `libraries.toml` breaks a documented recovery path that intentionally depends on synthesized defaults;
- deterministic identity requires changing the sync protocol or server schema;
- implementing the change would require automatic write-on-read migration.

## 12. Final acceptance criteria

- [ ] Existing malformed library TOML cannot be treated as an empty writable library.
- [ ] Existing malformed `libraries.toml` cannot be treated as a default writable index.
- [ ] Corrupt files are backed up best-effort before the parse error is returned.
- [ ] Missing and empty files retain existing valid behavior.
- [ ] Missing IDs are deterministic across repeated read-only loads.
- [ ] Duplicate-ID repair is deterministic and preserves the first explicit ID.
- [ ] Unique existing IDs are unchanged.
- [ ] Normal save persists repaired IDs without a dedicated write-on-read migration.
- [ ] No new dependency is added unless the UUID-assumption audit proves it necessary and the plan is amended.
- [ ] Pet-compatible valid files remain readable.
- [ ] Focused tests pass.
- [ ] `bash scripts/check.sh` passes.

## 13. Suggested implementation commit

```text
phase-14b: fail closed on corrupt TOML and stabilize legacy IDs
```
