# Locking Protocol and Ordering Convergence Milestone 001 — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/locking-convergence/001-owned-record-lock-protocol-unification.md`

Source subsystem roadmap:

- `plans/subsystems/locking-convergence-roadmap.md`

Repository baseline reviewed: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Implementation commits or pull requests:

- Implementation commit `IMPLEMENTATION_COMMIT_SHA`, relative to baseline
  `dca5d31`. Change set: 1 new shared module
  (`src/record_lock.rs`), 2 rewired callers, 3 new static guards, 2
  architecture documents updated.

## 1. Executive finding

The milestone's infrastructure boundary is complete. The owned-record lock
protocol — `create_new` + ownership record + conservative reclaim — now
exists in exactly one module (`src/record_lock.rs`), and the difference
between the transaction lock and the local-data lock is an explicit,
**required** wait-policy parameter rather than two divergent copies of the
protocol. Every observable behavior of both locks is unchanged: the lock file
names, the on-disk record format, the conservative ownership rule, the
ownership-verified quarantine, the bounded retry ladder, and every
user-facing error message.

The plan's claim that the two copies "differ only in their wait policy" was
approximately but not exactly right: the code differed in three arms, all of
them wait-policy-shaped. Those three are reconciled inside the policy, and
the difference is recorded in §10.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| The owned-record acquisition protocol exists exactly once | `src/record_lock.rs` is the only `src/` module containing `create_new(true)` **and** a record write **and** `sync_all` **and** a `ProcessIdentity::observe` call — the detector used by the static guard | pass | `create_new(true)` also appears in `commands/mod.rs`, `commands/run_cmd.rs`, `transaction.rs` (staging), `update.rs`, `utils/atomic.rs`; none of those observes a lock owner. |
| The wait policy is a required parameter | `OwnedLockWaitPolicy<R>` (`src/record_lock.rs:234`) has two variants and no `Default` implementation | pass | `transaction.rs` selects `FailFast`; `local_data.rs` selects `WaitForProgress`. Neither can inherit the other's contract by omission. |
| The transaction lock keeps failing fast | `test_acquire_lock_conflict` asserts the exact text `Runtime error: Transaction lock held: Another transaction (op1) is in progress (PID <pid>). Wait for it to complete.` | pass | Unchanged from baseline. |
| The local-data lock keeps waiting for progress | `test_local_data_lock_timeout_error_text` asserts both timeout texts verbatim | pass | `…the lock did not change hands for 30 seconds.` and `…the lock record stayed empty for 30 seconds, which usually means a writer was interrupted between creating and writing the lock file.` |
| The on-disk record format is byte-identical | `test_baseline_transaction_lock_record_parses_and_serializes_identically` and its local-data counterpart parse a baseline literal and re-serialize to the baseline TOML | pass | Fields asserted: `schema_version = 1`, `pid`, `nonce`, `created_at_unix_ms`, `operation` (transaction record only), `start_token`. A lock written by the baseline build parses under the new implementation and vice versa. |
| The conservative ownership rule is preserved | Reclaim requires the recorded **and** observed start tokens to both be present and differ; a missing recorded token, a missing observed token, or matching tokens is a live owner | pass | Covered by the shared module's unit tests. |
| The quarantine stays ownership-verified against the observed record | `quarantine_owned_lock` re-reads the record and compares it to the observed one before renaming | pass | A concurrent acquirer that recreated the lock between the rename attempt and the retry is not quarantined. |
| A failed record write removes the created file | Shared implementation removes the just-created file and returns the error | pass | Preserves the "no 0-byte lock file survives" property. |
| `is_transient_lock_contention` exists once | `src/record_lock.rs:770` — one definition, used by both callers | pass | Previously byte-identical in two places. |
| A static guard prevents a second copy | `tests/architecture.rs` — `owned_record_lock_protocol_exists_in_exactly_one_module`, `owned_record_lock_protocol_detector_reports_a_second_copy` (negative case proven on a synthetic copy: the detector count becomes 2), `transient_lock_contention_helper_is_defined_in_exactly_one_module` | pass | The guard does not hard-code a module path. |
| `Drop` semantics are unchanged | Both owned-record `Drop` impls performed an ownership-verified removal (`remove_owned_lock_file`: nonce + PID + start token) at baseline and still do | pass | The plan and roadmap text saying the owned-record locks "release on `Drop` without unlinking" is **inaccurate** — see §10. The behavior was preserved, not changed. |

## 3. Production implementation evidence

**New.** `src/record_lock.rs` (1190 lines) owns the protocol body once:
`acquire_owned_lock` (create with `create_new` → write the caller-supplied
record to the open handle → `sync_all`, warn-only on failure → remove the
created file and error if the write fails), the record classification
(empty ladder, malformed ladder, dead owner, live owner),
`ProcessIdentity::observe`, the conservative ownership rule,
`quarantine_owned_lock` (ownership-verified against the observed record),
`prune_quarantine_files`, `QUARANTINE_RETENTION`, and the `OwnedLockGuard<R>`
`Drop`-release step. The platform start-token parsers moved here from
`src/transaction.rs`; `snip_it::transaction::{ProcessIdentity,
current_process_identity}` are preserved as re-exports.

**Rewired.** `src/transaction.rs` (4306 → 3935 lines) and
`src/local_data.rs` (631 → 451 lines) are now thin wrappers that select their
wait policy and their own error constructors. `TransactionLock` and
`LocalDataLock` are aliases of the shared guard.

**Explicitly not implemented (other milestones).** The kernel-lock protocol
(`src/process_file_lock.rs`, `snip-sync/src/server_lock.rs`,
`snip-sync/src/process.rs`, the `croncheck.lock` copy) is M002. The
acquisition-order contract and its static guard, the `WorkerLock` tier
retirement, and the `execution_lock.rs` self-duplication are M003.

## 4. Verification executed

### Commands run

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --lib
cargo test --test architecture --features test-support
cargo test --test local_data_lock_barriers --features test-support -- --test-threads=1
cargo test --test repair_transactions --features test-support -- --test-threads=1
cargo test --test process_lock_concurrency --features test-support -- --test-threads=1
cargo test --test local_contracts
cargo test --test cleanup_crash_failpoints --features test-support -- --test-threads=1
cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1
cargo test --test restore_transactions --features test-support -- --test-threads=1
cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1
cargo test --test restore_manifest_gate --features test-support -- --test-threads=1
```

### Results

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass (exit 0, no output) |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass (0 errors, 0 warnings) |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass (0 errors, 0 warnings) |
| `cargo test --workspace --lib` | pass — snip-it 1211 passed / 0 failed / 8 ignored; snip-sync 187 passed / 0 failed |
| `cargo test --test architecture --features test-support` | pass — 11 passed / 0 failed (includes the three new lock guards) |
| `cargo test --test local_data_lock_barriers --features test-support -- --test-threads=1` | pass — 15 passed / 0 failed |
| `cargo test --test repair_transactions --features test-support -- --test-threads=1` | pass — 41 passed / 0 failed |
| `cargo test --test process_lock_concurrency --features test-support -- --test-threads=1` | pass — 14 passed / 0 failed |
| `cargo test --test local_contracts` | pass — 23 passed / 0 failed |
| `cargo test --test cleanup_crash_failpoints --features test-support -- --test-threads=1` | pass — 19 passed / 0 failed |
| `cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1` | pass — 21 passed / 0 failed |
| `cargo test --test restore_transactions --features test-support -- --test-threads=1` | pass — 24 passed / 0 failed |
| `cargo test --test transaction_crash_recovery --features test-support -- --test-threads=1` | pass — 26 passed / 0 failed |
| `cargo test --test restore_manifest_gate --features test-support -- --test-threads=1` | pass — 11 passed / 0 failed |

**Scope note.** The crash and restore suites that no script invokes
(`cleanup_crash_failpoints`, `restore_crash_failpoints`) **were** run by
hand, because this milestone rewrites the transaction lock protocol that the
crash-recovery and restore paths acquire — `AGENTS.md` requires them by hand
before merging a change that touches a restore or cleanup path. The
serial suites that exercise the owned-record lock
(`local_data_lock_barriers`, `repair_transactions`,
`process_lock_concurrency`, and the five crash/restore suites above) were run
with `--test-threads=1` as required. The full `scripts/check.sh` suite was
not run; verification was deliberately scoped to the targets this change can
affect, and it remains the pre-merge gate.

## 5. Invariant review

| Invariant | Evidence | Result |
|---|---|---|
| Kernel locks are authoritative | Untouched; `src/process_file_lock.rs` is not in this milestone's scope | pass |
| `kill(pid, 0)` proves absence only for `ESRCH` | The shared implementation reuses `ProcessIdentity::observe` unchanged | pass |
| Linux start tokens come from `/proc/<pid>/stat` field 22 | The parsers moved with the protocol and were not rewritten | pass |
| The conservative ownership rule | Reclaim only when the recorded and observed start tokens are both present and differ; an unverifiable owner is a live owner | pass |
| The quarantine is ownership-verified against the observed record | `quarantine_owned_lock` re-reads and compares before renaming | pass |
| The empty-file window is minimized and a failed write removes the file | Preserved in the shared implementation | pass |
| A record `sync_all` failure is a warning, not an error | Preserved | pass |
| Acquisition stays bounded | No error kind can wedge the loop: each arm is bounded by a deadline, a retry counter, or the policy's wait window | pass |
| The mutation gate and the fail-closed behavior on corrupt journals are untouched | `src/transaction.rs` changes are confined to the lock protocol | pass |
| No diagnostic message contains a credential | No new message text; all error strings are the pre-existing ones | pass |
| Lock files may hold stale metadata | Unchanged — the `Drop` release is still ownership-verified | pass |

## 6. Failure and recovery review

- **Duplicate delivery / idempotency** — not applicable: the lock protocol is
  not a delivery path.
- **Cancellation races** — a cancelled acquisition leaves the created file
  removed by the failed-write path; unchanged.
- **Restart** — a stale lock file left by a killed process is still detected
  and reclaimed through the same ladder; unchanged.
- **Partial persistence failure** — the record write failing after
  `create_new` removes the created file before returning the error, so no
  0-byte lock survives; unchanged and now covered by one shared code path
  rather than two.
- **Stale generation or lease** — the PID-reuse reclaim still requires two
  present, differing start tokens; unchanged.
- **Contention and resource release** — the two wait policies are the
  observable difference and both are asserted by tests; the release path is
  one `Drop` implementation now instead of two.
- **Malformed or unauthorized input** — a malformed lock record still takes
  the bounded ladder before quarantine; the ladder bounds are unchanged.
- **Bounded event and artifact behavior** — quarantine files keep the same
  7-day retention and the same `prune_quarantine_files` behavior.

## 7. Migration and compatibility review

- **Schema migration** — none required. The on-disk lock record format is
  byte-identical to the baseline, proven by the cross-version parse tests.
- **Backward compatibility** — a lock file written by a baseline build is
  parsed by the new implementation and re-serialized identically; a file
  written by the new implementation is parsed by a baseline build. No
  protocol, CLI, or persisted-format change.
- **Protocol negotiation** — not applicable.
- **Configuration behavior** — unchanged.
- **Rollback limitations** — rolling back restores two copies of the
  protocol; the lock file format is unchanged, so the rollback is safe.
- **Legacy-path status** — the legacy single-file library migration is
  untouched.

## 8. Security review

- **Authorization enforcement** — unchanged; the lock protocol is a
  mutual-exclusion mechanism, not an authorization boundary.
- **Secret handling** — no credential is read, written, or logged. The lock
  record carries only a PID, nonce, timestamp, start token, and (for the
  transaction record) the operation name.
- **Path validation** — unchanged; the lock paths are derived exactly as
  before.
- **Privilege boundaries** — unchanged.
- **Denial-of-service bounds** — every arm remains bounded; the wait-for-
  progress policy keeps its 30 s base window, 300 s hard cap, and 250 ms poll
  ceiling, and the fail-fast policy keeps its flat 30 s deadline.
- **Redaction** — no error text was added; no credential entered a diagnostic.
- **Audit behavior** — unchanged, except that one informational reclaim log
  line lost its record-specific `operation` field (the shared protocol cannot
  log a caller-specific field); the message text is unchanged. See §10.

## 9. Documentation and operations

Updated architecture documents: `architecture/process_file_lock.md` (the
owned-record section was rewritten to describe the single shared
implementation and the wait policy as a parameter),
`architecture/persistence.md` (the transaction-lock and local-data-lock
sections now point at `src/record_lock.rs`, name `quarantine_owned_lock`,
and state the two policies), `architecture/overview.md` §7 and the §5.2
module table, `architecture/commands/repair_cmd.md` (the transaction lock is
taken through the shared protocol with the fail-fast policy).

Updated contract documents: `docs/LOGICAL_LAYERS.md` (new `src/record_lock.rs`
row, the convention list, the `src/transaction.rs` and `src/local_data.rs`
rows, and two new verification-checklist items),
`docs/PUBLIC_API.md` (the `pub(crate)` module count), `CHANGELOG.md` under
`### Changed`.

Static guards: `tests/architecture.rs` — three new assertions (single
protocol body, single `is_transient_lock_contention`, with a proven negative
case for the protocol detector).

Operator diagnostics: the lock error texts are unchanged, so any operator
runbook that quotes them stays valid.

Recovery instructions: none changed. The recovery paths (quarantine, reclaim,
repair) are the same operations under a new implementation.

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| low | The plan (§4, §14) and the roadmap state that the owned-record locks "release on `Drop` without unlinking". That is true of the kernel-backed tier only; both owned-record `Drop` impls have always performed an ownership-verified removal via `remove_owned_lock_file`. | Documentation inaccuracy in the plan and roadmap text. The implementation deliberately preserved the real behavior rather than "fixing" it to match the wording. | Correct the wording in the locking-convergence roadmap (M003 documentation pass) and in the plan's §4/§14. |
| low | The plan's §3 evidence claimed the two copies "differ only in their wait policy". The code differed in three arms: the live-owner arm, the read-error arm (the transaction lock enforces a flat deadline there, the local-data lock does not), and the empty-record arm (the local-data lock enforces a deadline there, the transaction lock does not). | The plan understated the surface that had to be reconciled. All three are wait-policy-shaped and are now explicit in the policy. | None for this milestone. The plan's evidence section should not be read as exhaustive. |
| low | The plan's §3 evidence claimed the transaction quarantine was **not** ownership-verified against the observed record. The code has always verified it (both locks were symmetric). | The plan's premise about the weaker invariant was wrong; the invariant was in fact held and is preserved. The stale claim in `architecture/process_file_lock.md` and `architecture/overview.md` was corrected in this milestone's owned documents. | None. |
| low | The transaction lock's reclaim log line lost its record-specific `operation` field when the protocol body moved into the shared module (`tracing::info!(pid, operation = %existing.operation, …)` → a message without that field). | The message text is unchanged; only a structured field is gone from one info log. The local-data lock never had it. | Acceptable for this milestone; M003 may restore it if the shared protocol gains a caller-supplied log-field hook. |
| low | `empty_retries += 1` (transaction) and `empty_retries = empty_retries.saturating_add(1)` (local-data) were unified to `saturating_add`. | No observable difference: the counter is bounded far below `i64::MAX`. | None. |
| low | `src/lib.rs` needed a module declaration for the new module (outside the plan's stated file list). | Mechanical, required for compilation. | None. |

## 11. Roadmap disposition

Milestone closed; the next dependency may proceed. M002 (kernel-lock protocol
convergence) and M003 (acquisition-order contract, `WorkerLock` retirement,
`execution_lock.rs` self-duplication) are now unblocked and remain to be
planned. The documentation corrections listed in §10 are assigned to M003.

## 12. Registry updates

- `plans/registry.md`: the locking-convergence roadmap moves from `ready` to
  `active`; this milestone moves from `not started` to `closed`; the plan row
  moves from `ready` to `closed`.
- `plans/subsystems/locking-convergence-roadmap.md`: §12 milestone status
  table updated — M001 `closed`, with the closure record linked; the
  milestone's completion note records the three plan-premise corrections.
