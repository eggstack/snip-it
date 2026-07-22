# Fuzzing and Property Tests — Workstream L

**Scope:** Inventory of existing property-based and fuzz-style tests, identification of gaps, and verification of key invariants.

---

## Table of Contents

- [Current Fuzz/Property Test Inventory](#current-fuzzproperty-test-inventory)
- [Required Fuzz Targets](#required-fuzz-targets)
- [Properties Verified](#properties-verified)
- [CI Integration](#ci-integration)
- [Regression Corpus](#regression-corpus)
- [Known Gaps](#known-gaps)

---

## Current Fuzz/Property Test Inventory

The project does not use a dedicated fuzzing framework (e.g., `cargo-fuzz` or `proptest`). Instead, property-like invariants are verified through integration tests and unit tests that exercise parsing, serialization, encryption, and concurrency under varied inputs. The following test suites serve as property tests.

### Round-Trip Tests

Tests that verify `serialize -> deserialize` or `encrypt -> decrypt` produce identity.

- **Encrypt/decrypt round-trip** (`src/encryption.rs`): Verifies `decrypt(key, encrypt(key, plaintext)) == plaintext` for normal strings, empty strings, and Unicode payloads. Also verifies wrong-key decryption fails and that duplicate encryptions produce different ciphertext (nonce uniqueness).
- **Snippet encrypt/decrypt round-trip** (`src/sync.rs`): Verifies `decrypt_snippet` inverts `encrypt_snippet` for snippet commands, including special characters and non-encrypted passthrough.
- **TOML serialize/deserialize round-trip** (`src/library.rs`): Verifies `save_library -> load_library` preserves snippet fields exactly. Tests cover tabs, trailing whitespace, CRLF line endings, and backslash-containing commands. The golden command corpus (24 edge cases) exercises exact-text preservation across all acquisition sources.
- **FailureClass code-string round-trip** (`src/auto_sync/executor.rs`): Verifies `FailureClass -> ExecutorExitCode -> FailureClass` round-trip is lossless for all 11 variants, and that `FailureClass -> code_string -> FailureClass` is also lossless.

### Migration Idempotency Tests

- **`test_fixture_migration_idempotency`** (`src/migration.rs`): Runs the same migration twice on a fixture file and verifies the result is identical. Proves migrations are idempotent.
- **`test_run_migrations_noop_when_current`** (`src/migration.rs`): Verifies that running migrations on a schema at the current version produces no changes.
- **`test_legacy_v1_migration`** (`src/auto_sync/pending.rs`): Verifies legacy pending file format is correctly migrated to the current schema.
- **`test_sync_repair_idempotent`** (`tests/recovery_integration.rs`): Verifies the repair command is idempotent when run multiple times.

### Lock Contention Tests

- **`test_sequential_lock_acquisition`** (`tests/mutual_exclusion.rs`): Proves two sequential lock acquisitions succeed when the first is released.
- **`test_concurrent_lock_acquisition_blocked`** (`tests/mutual_exclusion.rs`): Proves a second `try_acquire` fails while the first lock is held.
- **`test_execution_lock_survives_across_functions`** (`tests/mutual_exclusion.rs`): Proves the lock is not released by dropping a reference in an inner scope.
- **`test_schedule_already_active_when_lock_held`** (`tests/mutual_exclusion.rs`): Proves scheduling returns `AlreadyActive` when the execution lock is held.

### Debounce Behavior Tests

- **Debounce matrix** (`tests/debounce_matrix.rs`): Uses a `MockClock` to deterministically test debounce timing. Verifies debounce window behavior, max-delay starvation prevention, and interaction with pending state transitions.
- **Worker debounce integration** (`src/auto_sync/worker.rs`): Verifies debounce returns the latest observed state and respects the configured debounce interval.

### Sync Merge Tests

- **Sync contracts** (`tests/sync_contracts.rs`): Verifies sync direction resolution (Push, Pull, Bidirectional) under all CLI-flag and config combinations. Proves CLI overrides take precedence over config.
- **Sync integration** (`tests/sync_integration.rs`): Async tests with an in-process server that verify full sync round-trips (push, pull, bidirectional) with real gRPC transport.

### Deterministic E2E Tests (Phase 05A)

- **`test_deterministic_sync_cycle`** (`tests/deterministic_e2e.rs`): Full end-to-end test proving the exact auto-sync lifecycle: local mutation -> pending generation -> worker spawn -> executor process -> server-side state change -> status success -> conditional pending clear. Uses a real in-process server, event sink for process lifecycle evidence, and exact-count assertions.

### Failure Class Contract Tests

- **Failure class matrix** (`tests/failure_class_contracts.rs`): Exhaustively tests all 11 `FailureClass` variants through the chain: `FailureClass -> ExecutorExitCode -> status file -> ScheduleDecision`. Verifies each exit code maps back to the correct failure class, status records the correct backoff, and scheduling decisions respect failure semantics.

### Mutual Exclusion Tests

- **Execution lock tests** (`tests/mutual_exclusion.rs`): Proves exclusive access to the sync execution path. Tests cover sequential acquisition, concurrent denial, cross-scope persistence, and scheduling interaction.
- **Worker lock tests** (`tests/auto_sync_concurrency.rs`): Verifies the worker-level lock prevents concurrent worker processes.

### Process Lifecycle Tests

- **Process lifecycle** (`tests/process_lifecycle.rs`): Tests worker-nothing-to-do without pending, SIGTERM/SIGKILL child reaping, child-exits-before-deline, and executor timeout enforcement. Proves process cleanup is reliable.
- **Auto-sync lifecycle** (`tests/auto_sync_lifecycle.rs`): Tests full worker lifecycle including spawn, execution, and status recording.

### Local Contracts Tests

- **Local contracts** (`tests/local_contracts.rs`): Exercises the `snp` binary for all subcommands (new, list, run, clip, select, search, edit) and verifies exit codes, output format, and field preservation. Includes the golden command corpus verifying exact-text round-trip for 24 edge cases.

### Package Evidence Tests

- **Package evidence** (`tests/package_evidence.rs`): Verifies `cargo package --list` includes required files, the binary name is `snp`, help output mentions all subcommands, and release binaries have no debug assertions.

### Persistence Unit Tests

- **Atomic write tests** (`tests/persistence_unit.rs`): Tests atomic file replacement across platforms: successful replace, write failure preserves original, rename failure preserves original, unique temp paths under concurrency (20 threads), durability classes, and permission handling.

### Identity Contract Tests

- **Identity contract** (`tests/identity_contract.rs`): Verifies snippet IDs survive edit, move, copy, delete-recreate, and library merge operations. Proves ID stability across the full snippet lifecycle.

### Backup/Validate Tests

- **Recovery integration** (`tests/recovery_integration.rs`): Tests backup snapshot creation, restore dry-run, restore merge, restore replace, repair idempotency, and validation command output.

---

## Required Fuzz Targets

The following parsers and data-processing paths should be fuzzed with `cargo-fuzz` or equivalent to discover panics, OOM, and logic errors under adversarial input. None are currently fuzzed externally.

### Snippet TOML Parser

**Target:** `src/library.rs` (`load_library`, `Snippets` deserialization)

**Rationale:** User-authored TOML files are the primary data source. Malformed or adversarial TOML can trigger parser panics, excessive allocation, or logic errors. Fuzzing should cover:
- Deeply nested arrays and tables
- Invalid UTF-8 sequences in string values
- Extremely long strings and field names
- Repeated keys
- TOML escape sequences (tabs, CRLF, backslashes)

### Variable/Default/Choice Parser

**Target:** `src/utils/variable_parser.rs` (variable expansion syntax)

**Rationale:** Snippet commands contain `{{variable}}` syntax with defaults and choices. Fuzzing should verify that the parser never panics on arbitrary input and that expansion produces valid shell output.

### Selector/Query Normalization

**Target:** `src/selector.rs`, `src/commands/get_cmd.rs`

**Rationale:** Exact selectors (`--id`, `--description-exact`, `--command-exact`) and fuzzy queries are normalized before matching. Fuzzing should verify that normalization never panics and that resolution policies (Unique, First, All) behave correctly under all inputs.

### Pending/Status/Lock/Journal Parser

**Target:** `src/auto_sync/pending.rs`, `src/auto_sync/status.rs`, `src/auto_sync/lock.rs`, `src/transaction.rs`

**Rationale:** These modules parse TOML files written by the auto-sync subsystem. Corrupted or partial files (from crashes or race conditions) must not cause panics. Fuzzing should cover truncated files, partially written content, and schema version mismatches.

### Backup Manifest/Path Validation

**Target:** `src/commands/backup_cmd.rs`, `src/commands/restore_cmd.rs`

**Rationale:** Backup manifests contain checksums and file paths. Fuzzing should verify that malformed manifests, invalid checksums, and path traversal attempts are rejected without panics.

### Encryption Frame Parser

**Target:** `src/encryption.rs` (`decrypt`)

**Rationale:** The encrypted payload format includes a Base64-encoded frame with version, salt, nonce, and ciphertext. Fuzzing should verify that truncated, malformed, or Base64-invalid inputs produce errors, not panics.

### Sync Merge Input

**Target:** `src/sync_commands.rs`

**Rationale:** Sync merge logic processes remote snippets and compares them with local state. Fuzzing should cover conflicting IDs, duplicate descriptions, empty libraries, and adversarial merge inputs.

### Server URL/Config Parser

**Target:** `src/config.rs` (`SyncSettings`, URL resolution)

**Rationale:** Server URLs and configuration values are parsed from user-editable TOML. Fuzzing should verify that malformed URLs, missing fields, and type mismatches produce errors, not panics.

---

## Properties Verified

The following invariants are verified by the existing test suite and should be maintained by any future fuzzing infrastructure.

### No Panics in Parsing

All parsers (TOML, variable syntax, selector queries, encryption frames, pending/status files) return `Result` types. Unit tests exercise malformed inputs including empty strings, invalid UTF-8, deeply nested structures, and truncated data. No parsing path is permitted to panic.

### Bounded Allocation

- **Command cap:** Snippet commands are capped at 16 MiB (`src/library.rs`). Commands exceeding this limit are rejected during deserialization.
- **gRPC message cap:** Tonic's default message size limit of 4 MiB is enforced on both client and server. Oversized messages are rejected at the transport layer.

These bounds prevent OOM from adversarial or corrupted input.

### Valid Round-Tip Stability

Every serialization path (TOML save/load, encrypt/decrypt, FailureClass code-string) is verified to produce identity round-trips. The golden command corpus in `src/library.rs` tests 24 edge cases including tabs, trailing spaces, CRLF, and backslash sequences to ensure exact byte-level preservation.

### Migration Idempotency

All migration functions are tested to produce identical output when run twice on the same input. The `test_fixture_migration_idempotency` test in `src/migration.rs` pins this invariant.

### Corruption Never Maps to Success/Current

The failure-class contract tests verify that no corruption of the status file, pending file, or lock file maps to a success state or a current-schema interpretation. Corrupted files are either migrated, repaired, or rejected.

### Encryption Tampering Fails Closed

The `test_wrong_key_fails` test in `src/encryption.rs` verifies that decryption with an incorrect key returns an error. Additional tests verify that tampered ciphertext (modified Base64 content) also fails closed.

---

## CI Integration

The existing test suite provides bounded fuzz coverage through targeted edge-case inputs. The following commands exercise these paths:

```bash
# Full test suite including property-like tests
cargo test --workspace

# Deterministic E2E (requires single-threaded PTY tests)
cargo test --test pty_integration -- --test-threads=1

# Phase 05A test suites (deterministic, contract, debounce, sync)
cargo test --test deterministic_e2e
cargo test --test failure_class_contracts
cargo test --test debounce_matrix
cargo test --test sync_contracts
cargo test --test mutual_exclusion
cargo test --test process_lifecycle
cargo test --test local_contracts
cargo test --test package_evidence

# Phase 07A test suites (persistence, identity)
cargo test --test persistence_unit
cargo test --test identity_contract

# Sync integration (async, real in-process server)
cargo test --test sync_integration
```

No long-running external fuzz service is currently integrated. All property-like coverage is achieved within the standard test suite execution time.

---

## Regression Corpus

Minimized test cases from debugging and fuzzing are embedded directly in unit tests rather than stored as external corpus files. Key examples:

- **Golden command corpus** (`src/library.rs`): 24 edge-case commands with tabs, trailing spaces, CRLF line endings, and backslash sequences. Each case has a label and expected round-trip behavior.
- **Encryption edge cases** (`src/encryption.rs`): Empty string, Unicode payloads, wrong-key rejection, different-nonce verification.
- **TOML escape handling** (`src/utils/toml_helpers.rs`): Save-then-load round-trip with escapes and CRLF preservation.
- **FailureClass exhaustive matrix** (`src/auto_sync/executor.rs`): All 11 variants tested through code-string and exit-code round-trips.

These minimized cases serve as the regression corpus. If external fuzzing is added in the future, interesting inputs should be promoted to unit tests to prevent regression.

---

## Known Gaps

### No Long-Running External Fuzz Service

The project does not use `cargo-fuzz`, `libfuzzer`, AFL, or any external fuzzing service. All property-like testing is achieved through unit tests with targeted edge cases. A dedicated fuzzing harness would provide broader coverage for:

- Parser robustness under adversarial input
- Allocation limits under extreme input sizes
- Concurrency edge cases under scheduling pressure
- Cryptographic implementation under malformed ciphertext

This is a candidate for future work, particularly for the parsers listed in [Required Fuzz Targets](#required-fuzz-targets).

### No Proptest/Quickcheck Integration

The project does not use property-testing frameworks like `proptest` or `quickcheck`. Test inputs are hand-crafted rather than generated. This limits coverage to anticipated edge cases rather than discovered ones. Integrating `proptest` for the TOML parser, variable parser, and encryption frame parser would provide automated input generation and shrinking.
