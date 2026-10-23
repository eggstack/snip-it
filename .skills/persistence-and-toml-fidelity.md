# Persistence & TOML Byte-Fidelity Skill

## Purpose
Guide agents through local data persistence: atomic writes and durability
classes (`src/utils/atomic.rs`), library save/load (`src/library/`), and
backup/restore/validate/repair (`src/commands/`).

The critical rule in this file is the one that produces **silent data loss**,
not a crash. Read it before touching the save path.

---

## Rule 1: the save path does NOT post-process TOML

`save_library()` (`src/library/persistence.rs:258`) writes the output of
`toml::to_string_pretty` **verbatim**. There is exactly one
serialization-then-write site, and nothing may run between them.

**Why this matters:** any post-processing pass (re-indenting, key sorting,
trimming, "fixing" quotes) silently corrupts any user file containing tabs,
trailing whitespace, or CRLF. The user gets no error — they get mangled
snippets later.

There is a specific historical trap here. A helper called
`quote_strings_containing_backslashes` once ran after serialization. It
corrupted data because its regex could not distinguish a TOML **triple-quoted
multi-line string** from an ordinary **double-quoted** string, and its
single-quoted output preserved escape sequences like `\t` as a literal
two-character pair. The helper still exists for callers that hand-write TOML —
**it must never be applied to serialized output.**

### Rules

- Never insert a transform between `toml::to_string_pretty` and the write.
- The serializer already chooses correct quoting and escaping for every
  character, including tabs, trailing spaces, and CRLF. Trust it.
- New fields need `#[serde(default)]` so old files still parse.

### The golden command corpus

`golden_corpus()` (`tests/support/mod.rs:157`) holds **24 edge-case commands**:
tabs (internal, in a Makefile recipe, adjacent to quotes, before trailing
whitespace), trailing spaces (single and multiple), CRLF and mixed newlines,
no/one/many trailing newlines, backslashes (Windows paths, tab+backslash),
escaped angle brackets, nested quotes, shell operators and substitution,
unicode, leading spaces, a multiline script, blank lines, and variables.

`tests/local_contracts.rs:217` (`test_golden_corpus_preserves_exact_text`) drives
every case through the real binary and asserts the command survives
create → save → load byte-for-byte. It also asserts the **exact snippet count**
matches the corpus size, so a silently dropped snippet fails the test.

**When you change anything in the save/load path, add a corpus case that would
have caught your bug.** These are the regression cases for a data-loss class
that no other test covers.

```bash
cargo test --test local_contracts test_golden_corpus
```

---

## Rule 2: gate every local mutation

`gate_mutation_on_interrupted_transactions(sync_state_dir, transaction_dir)`
must run before any local mutating operation. It needs **both** directories.

- One interrupted journal → auto-rollback and continue.
- Multiple or incomplete journals → **refuse** and direct the user to `snp repair`.
- It inspects the canonical pending marker in `sync_state_dir` while cleaning up
  artifacts in `transaction_dir`.

`check_interrupted_transactions` is a **scan helper, not the gate**. Calling the
scan and proceeding anyway reproduces the corruption it exists to prevent.

Directory model and the full state machine are in
[`transactions-and-auto-sync.md`](transactions-and-auto-sync.md) — read it
alongside this file, not instead of it.

---

## Rule 3: use durability classes, not raw `fs::write`

`atomic_replace` with `AtomicWriteOptions::for_durability()`
(`src/utils/atomic.rs`) is the only sanctioned write for user data. Match the
class to how much the data matters:

| Class | Use for | File fsync | Parent-dir fsync |
|-------|---------|-------------|-----------------|
| `DurableUserData` | pending markers, auto-sync status, restore installs, transaction rollback, sync recovery markers | yes | **required** — hard error on failure |
| `SensitiveConfig` | credentials, sync settings | yes | **required** — hard error on failure |
| `RecoverableMetadata` | caches, usage counters | no | best-effort — logs a warning and continues |

Notes that the shorthand table tends to get wrong:

- **All four variants apply `mode(0o600)` to the temp file unconditionally**
  (`src/utils/atomic.rs:17-20`); the target inherits it. The variants differ in
  **fsync** behavior, not in permissions. `AtomicWriteOptions::preserve_permissions`
  keeps the target's existing mode instead.
- `SensitiveConfig` additionally **rejects symlinks** by default
  (`src/utils/atomic.rs:62`). That is its only behavioral difference from
  `DurableUserData`.
- `RecoverableMetadata`'s parent-dir sync is **not** a guarantee: it degrades to
  a warning and returns `Ok(Some(false))` (`src/utils/atomic.rs:247-264`).

**Snippets and libraries are not written through `atomic_replace`.** They go
through `write_private_atomic` (`src/utils/atomic.rs:278`), which takes no
`Durability` argument: it always fsyncs the file, and fsyncs the parent
directory under `SensitiveConfig` semantics — but only on Unix
(`src/utils/atomic.rs:326-327` is `#[cfg(unix)]`).

> **Windows hazard.** `atomic_replace` calls `sync_parent_dir` unconditionally
> (`src/utils/atomic.rs:434`), and `parent_dir_sync_durable` returns `Err` on
> non-Unix (`src/utils/atomic.rs:180-186`). So on Windows an `atomic_replace`
> with `DurableUserData`/`SensitiveConfig` **replaces the target and then
> returns an error** — and never returns the `AtomicWriteReport`. Callers that
> treat `Err` as "not written" are wrong on Windows. `write_private_atomic` is
> unaffected.

Picking a weaker class than the data deserves is how "the file was there but
empty after a crash" happens. A stronger class is a performance cost, not a
correctness win — don't over-apply it to caches.

---

## Rule 4: malformed data fails closed

- A **malformed** library file or `libraries.toml` → best-effort backup, then
  **error**. Never synthesize a writable empty library; that turns a recoverable
  parse error into data loss on the next save.
- A **missing or empty** file → defaults. These are different cases and must
  stay different.
- Validation is read-only and runs **before** repair.
  - Safe repairs: rebuild index, fix primary selection, remove orphans, generate
    missing IDs.
  - Refuse: same-ID divergence, partially parseable TOML, corrupt pending
    intent, multiple primary candidates.

`snp repair --apply` and `snp restore --replace` each create a pre-operation
backup automatically. Do not remove that.

---

## Module map

| File | Role |
|------|------|
| `src/library/model.rs` | `Snippet`, `Snippets`, `LibraryConfig`, `LibraryMeta` |
| `src/library/persistence.rs` | `load_library`, `save_library`, `save_library_internal`, ID normalization |
| `src/library/manager.rs` | `libraries.toml` registry, primary selection, server linkage, `resolve_readonly_sources` |
| `src/utils/atomic.rs` | `atomic_replace`, `AtomicWriteOptions`, durability classes |
| `src/utils/toml_helpers.rs` | Escape helpers for hand-written TOML only |
| `src/commands/backup_cmd.rs` / `backup_archive.rs` | Secret-free snapshot + SHA-256 manifest |
| `src/commands/restore_cmd.rs` | Manifest validation, merge/replace/dry-run, pre-restore backup |
| `src/commands/validate_cmd.rs` | Read-only validation, severity + repairability |
| `src/commands/repair_cmd.rs` | Conservative, backed-up, idempotent repair; `exit 10` on unsafe-only |

`save_library_internal()` skips the gate and lock for internal callers that
already hold both — do not call it from a new external path without that
guarantee.

---

## Persisted artifacts

Every path, filename, and durability class is catalogued in
[`../docs/PERSISTENCE_INVENTORY.md`](../docs/PERSISTENCE_INVENTORY.md). Config
lives under `~/.config/snp/` (always — there is no per-platform branch; see the
module doc in `src/utils/config.rs` and the one-way macOS legacy migration).

---

## Tests

| Target | Protects |
|--------|----------|
| `tests/local_contracts.rs` | golden corpus byte fidelity, count preservation |
| `tests/persistence_unit.rs` | persistence primitives |
| `tests/schema.rs` | TOML/JSON schema round-trips |
| `tests/manifest_contracts.rs` | backup manifest + checksums |
| `tests/recovery_integration.rs` | recovery flows |
| `tests/repair_transactions.rs` | repair idempotency (feature-gated, serial) |
| `tests/restore_transactions.rs` | restore transaction correctness |
| `src/library/tests.rs` | in-crate round-trip and normalization unit tests |

```bash
cargo test --test local_contracts
cargo test --workspace --lib
cargo test --features test-support --test repair_transactions -- --test-threads=1
```

`repair_transactions` gates on `test-support` and must run serially. Deep
crash/restore suites (`transaction_crash_recovery`,
`cleanup_crash_failpoints`, `restore_crash_failpoints`) run only under
`bash scripts/release-check.sh verify` with a clean tree — they are not part of
ordinary CI.

---

## Common mistakes

- Adding a "cleanup" or "normalization" pass after `toml::to_string_pretty`.
- Treating a malformed library as an empty one and then saving over it.
- Writing user data with `fs::write` instead of an `atomic_replace` durability class.
- Using `check_interrupted_transactions` as if it were the gate.
- Skipping the pre-restore / pre-repair backup.
- Claiming crash-recovery behavior was verified when only the non-crash targets ran.
