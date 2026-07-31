# Post-Phase-11L — Lightweight Lock Repair and PID Compatibility Closure

Status: COMPLETE

Baseline: `535e9057b63f8602d0d1c9dbb48981d0a24be960`

This plan supersedes the **remaining work and closure requirements** in `plans/snip-it-post-11l-kernel-lock-and-pid-lifecycle-corrective-pass.md`. It does not revert the useful kernel-lock, server-lock, PID-record, or editor-change work that already landed.

Phase 11L remains complete. This is a small follow-up pass for two concrete post-11L defects and truthful closure bookkeeping.

Final implementation SHA: pending the documentation/status commit below.

---

## 1. Reassessment

The prior post-11L plan was too broad for this project. Snip-it is a small local TUI and a local/self-hosted sync service. It needs basic correctness and predictable behavior, but it does not need a production-grade lock verification program, a large failpoint framework, or an extensive evidence apparatus.

Most of the earlier corrective work is already sufficient:

- kernel-backed worker, execution, and pending locks exist;
- canonical lock files remain persistent;
- stale metadata no longer blocks normal lock acquisition;
- a killed process releases its kernel lock;
- the local sync server has a singleton kernel lock;
- structured and legacy PID parsing exists;
- PID publication is atomic;
- Windows process liveness is implemented;
- `snp edit` notifies only when bytes changed.

Do not rewrite these components again.

Only two behavioral issues remain worth fixing:

1. `snp sync repair` can still classify persistent lock files from metadata and delete them. Deleting a canonical kernel-lock pathname can split the lock namespace while another process still holds the old inode/handle.
2. `snip-sync stop` and `restart` still start from `read_pid_record()`, so they do not handle a numeric legacy PID file even though the parser supports it.

Everything else identified in the prior review is either already adequate or disproportionate for this tool.

---

## 2. Explicitly dropped requirements

The following are **not required** for this closure pass:

- no new lock abstraction;
- no rewrite of `ProcessFileLock`;
- no second server-lock implementation;
- no publication-interruption failpoint;
- no kill-between-every-write test matrix;
- no 500-cycle wrapper stress test;
- no new subprocess helper binary;
- no full two-server crash harness;
- no inode-generation or file-identity protocol;
- no new database, daemon, queue, lease, or lock registry;
- no new CI matrix;
- no evidence registry;
- no automated release flow;
- no requirement to prove every OS failure mode;
- no requirement to run every test target repeatedly;
- no changes to transaction recovery, sync protocol, or TUI behavior.

The existing `process_lock_concurrency` test is enough to cover basic cross-process exclusion. Keep it; do not expand it.

---

## 3. Complexity budget

Implementation must remain within this budget unless a compile error forces a small adjacent edit:

### Production files expected

- `src/commands/sync_cmd.rs`
- `snip-sync/src/main.rs`
- optionally `snip-sync/src/process.rs` for one small helper

### Test files expected

Prefer tests in existing modules. At most one small new integration test file may be added if the existing test layout cannot exercise the CLI behavior cleanly.

### Prohibited expansion

- no new crate;
- no new dependency;
- no new public command;
- no new feature flag;
- no new test helper binary;
- no new architecture document unless an existing statement becomes inaccurate;
- no generalized repair framework;
- no refactor of unrelated sync code;
- no broad renaming or formatting-only pass.

Target implementation size: roughly 100–250 changed lines excluding tests and plan/status text.

---

## 4. Small-model execution rules

1. Work in the exact commit order below.
2. Change only the named behavior in each commit.
3. Do not revisit the kernel-lock implementation unless the focused tests expose a direct regression.
4. Do not add a generalized abstraction for a branch used only by `stop`/`restart`.
5. Prefer deleting obsolete repair behavior over replacing it with a more elaborate repair protocol.
6. Prefer a typed `match` over new traits, callback layers, or state machines.
7. Tests must be deterministic but small; do not add stress loops or timing-sensitive races.
8. Keep manual crates.io release and the current lightweight CI topology unchanged.
9. Record one final implementation SHA after the focused tests pass.

---

# Workstream A — Stop repairing persistent lock files

## Goal

Make `snp sync repair` leave worker, execution, and pending lock files alone.

Persistent lock files are expected artifacts. Their metadata can be stale or malformed without preventing a later kernel-lock acquisition. Repairing them by pathname deletion is unnecessary and unsafe.

## Required production change

In `src/commands/sync_cmd.rs`, remove lock-file repair actions for:

- `auto-sync-execution.lock`;
- `auto-sync-worker.lock`;
- `auto-sync-pending.lock`.

Remove or stop invoking logic that produces these actions:

```text
remove stale lock
remove malformed lock
```

Remove the corresponding `apply_repair_action` branches that call `quarantine_and_remove` for these canonical lock files.

Do not replace this with a new acquire-and-rewrite repair protocol. The simplest correct policy is:

> `sync repair` does not manage persistent kernel-lock files.

It may continue repairing unrelated status and temporary artifacts.

## Required behavior

After this change:

- lock-file existence is not reported as a repair problem;
- stale PID metadata is not reported as a repair problem;
- malformed lock metadata is not reported as a repair problem;
- `--apply` never copies, renames, truncates, unlinks, or changes permissions on canonical lock files;
- the next real acquirer remains responsible for overwriting stale metadata after obtaining the kernel lock.

## Correct code shape

Prefer deletion of the lock scan and apply branches.

Conceptually:

```rust
// Status and orphan temp repair remain.
// Persistent lock files are intentionally ignored.
```

Do not add code resembling:

```rust
if process_dead(metadata.pid) {
    remove_file(lock_path)?;
}
```

Do not add code resembling:

```rust
if let Ok(_guard) = try_acquire(...) {
    rewrite_or_delete_lock_file(...);
}
```

Even that is unnecessary for a local tool. A future lock owner already rewrites metadata.

## Focused tests

Add or update only the following tests:

### A1. Malformed lock file is ignored

1. Create the state directory.
2. Write malformed bytes to all three canonical lock paths.
3. Run the repair action collector or `snp sync repair --dry-run`.
4. Assert no action targets any of the three lock files.

### A2. Apply preserves lock files

1. Write distinct sentinel bytes to all three canonical lock paths.
2. Run `snp sync repair --apply` in an otherwise clean fixture.
3. Assert each path still exists.
4. Assert the bytes are unchanged.

This is enough. Do not add race harnesses, inode checks, or hundreds of cycles.

## Acceptance criteria

- `sync repair` never schedules lock-file deletion;
- `sync repair --apply` preserves all three canonical lock files byte-for-byte;
- existing status/temp repair still works;
- no new lock-repair abstraction is introduced;
- no production code outside `sync_cmd.rs` changes for this workstream.

## Commit

Suggested commit:

```text
post-11L: stop sync repair from deleting persistent lock files
```

---

# Workstream B — Minimal legacy PID support for stop and restart

## Goal

Allow an older numeric `snip-sync.pid` file to participate in the existing Unix stop/restart flow without adding a second process-management subsystem.

## Required production change

Use the existing typed parser:

```rust
ParsedPidFile::Structured(record)
ParsedPidFile::LegacyPid(pid)
ParsedPidFile::Empty
ParsedPidFile::Malformed(message)
```

Update `cmd_stop` so it begins from `parse_pid_file(&paths::pid_path())` instead of `read_pid_record()`.

### Structured record

Keep the existing behavior unchanged:

- verify liveness and start token;
- verify process name unless `--force`;
- signal on Unix;
- wait for exit;
- acquire server singleton lock before cleanup;
- remove only if the record still matches.

Do not refactor the structured path beyond what is required to share the final signal/wait flow.

### Legacy numeric record on Unix

Use this minimal behavior:

1. If the PID is not running:
   - acquire the server singleton lock;
   - reread the PID file;
   - remove it only if it is still `LegacyPid(the_same_pid)`;
   - report stale PID cleanup and return success.
2. If the PID is running and `--force` is not set:
   - require `validate_process_name(pid)`;
   - refuse without signaling if the name does not look like `snip-sync`.
3. Signal and wait using the existing Unix stop logic.
4. After exit, acquire the server singleton lock.
5. Reread the PID file.
6. Remove it only if it is still the same numeric PID.

Do not invent a start token or nonce for a legacy record.

### Legacy numeric record on Windows

`stop` is already unsupported on Windows. Keep that policy. Return the existing unsupported-platform error and preserve the PID file.

Do not add Windows process termination in this pass.

### Empty or malformed file

Keep behavior simple:

- `Empty`: report no usable PID record and return the existing not-running error;
- `Malformed`: report that the PID file is malformed and must be removed or replaced; do not silently treat it as absence.

No automatic quarantine directory is required.

## Restart behavior

Update `cmd_restart` minimally:

- if the parsed file is `Structured` or `LegacyPid`, call `cmd_stop(force)`;
- if it is `Empty`, continue to `serve()`;
- if it is `Malformed`, return the same explicit malformed-file error rather than starting over ambiguous state.

Do not duplicate stop logic inside restart.

## Recommended small helper

A private helper is acceptable only if it removes duplicated cleanup checks, for example:

```rust
fn remove_pid_if_unchanged(expected: &ParsedPidFile) -> Result<(), String>
```

It must stay private and narrowly scoped. Do not add a new public PID lifecycle API.

## Focused tests

Use existing unit-test facilities where possible.

### B1. Dead legacy PID cleanup

1. Write a numeric PID that is known not to exist.
2. Invoke the stop decision/helper path.
3. Assert success.
4. Assert the numeric PID file is removed.

### B2. Live unrelated PID is preserved

Unix only:

1. Write the current test process PID as the numeric legacy PID.
2. Call the legacy stop preflight without `--force`.
3. `validate_process_name` should not identify the test process as `snip-sync`.
4. Assert refusal.
5. Assert the PID file remains unchanged.

Do not test `--force` against the test process.

### B3. Changed legacy PID file is not removed

1. Begin cleanup expecting legacy PID A.
2. Replace the file with legacy PID B before the final removal check.
3. Assert PID B remains.

This can be a direct helper/unit test. No subprocess is needed.

### B4. Restart recognizes legacy state

A small unit test may verify that `LegacyPid` selects the stop-then-serve branch. Do not start a real network server in this test.

## Acceptance criteria

- `stop` recognizes structured and numeric PID files;
- dead numeric PID files are cleaned safely;
- live numeric PIDs are not signaled unless the process name matches or `--force` is supplied;
- cleanup removes a legacy record only when the numeric PID is unchanged;
- restart delegates to stop for a legacy PID record;
- malformed PID data produces an explicit error;
- Windows stop behavior is unchanged;
- no new process-management framework is introduced.

## Commit

Suggested commit:

```text
post-11L: support legacy numeric PID files in stop and restart
```

---

# Workstream C — Minimal closure and truthful status

## Goal

Close the pass without creating another documentation or verification project.

## Required status updates

Update:

```text
plans/snip-it-post-11l-kernel-lock-and-pid-lifecycle-corrective-pass.md
```

Its top status should state that its remaining requirements were superseded by this lightweight plan. Preserve its history; do not delete or rewrite the implementation record.

After implementation and verification, update this plan with:

- `Status: COMPLETE`;
- one exact final implementation SHA;
- a short verification list;
- no claimed test counts unless directly observed;
- no claim that every conceivable crash or platform failure mode is proven.

## Verification required

Run only this focused set:

```bash
cargo fmt --all -- --check
cargo test --workspace --lib --all-features -- --test-threads=1
cargo test --test process_lock_concurrency --features test-support -- --test-threads=1
cargo test -p snip-sync --lib -- --test-threads=1
bash scripts/check.sh
```

Also run any new focused integration test target if one was added.

Existing CI remains unchanged. The final implementation commit should pass the existing Linux correctness, macOS smoke, and Windows smoke jobs. Do not add jobs or matrices.

No publish dry-run is required for this source-only closure pass. Release remains manual.

## Commit

Suggested commit:

```text
post-11L: close lightweight lock repair and PID compatibility pass
```

---

## 5. Ordered implementation sequence

### Commit 1 — Repair no longer owns lock files

Files:

- `src/commands/sync_cmd.rs`
- existing relevant tests

Closure for this commit:

- no lock deletion actions;
- apply preserves lock bytes;
- unrelated repair behavior still passes.

### Commit 2 — Legacy PID stop/restart support

Files:

- `snip-sync/src/main.rs`
- optionally `snip-sync/src/process.rs`
- existing relevant tests

Closure for this commit:

- dead legacy cleanup works;
- live unrelated PID is refused and preserved;
- changed PID record is not removed;
- restart recognizes a legacy record.

### Commit 3 — Status and focused verification

Files:

- this plan;
- prior post-11L plan status header;
- documentation only if an existing statement is now false.

Closure for this commit:

- exact implementation SHA recorded;
- focused commands pass;
- existing CI is green;
- no new infrastructure exists.

---

## 6. Explicit non-acceptance patterns

The pass is not acceptable if it introduces any of the following:

- deletion or rename of a canonical kernel-lock file;
- PID-liveness-based lock ownership;
- a new stale-lock reclamation path;
- a new quarantine directory for lock files;
- a new failpoint framework;
- a new subprocess helper binary;
- stress loops added solely to inflate proof;
- Windows process termination support;
- a generalized PID state machine;
- a new CI workflow or matrix;
- automated crates.io publishing;
- broad refactoring outside the named files;
- reopening Phase 11L.

---

## 7. Verification record

The implementation was recorded in these commits:

- `093e8db` — stop sync repair from deleting persistent lock files;
- `0a27cd9` — support legacy numeric PID files in stop and restart;
- final documentation/status commit — exact SHA recorded above after commit.

Verification completed locally:

- `cargo fmt --all -- --check`;
- `cargo test --workspace --lib --all-features -- --test-threads=1`;
- `cargo test --test process_lock_concurrency --features test-support -- --test-threads=1`;
- `cargo test -p snip-sync --lib -- --test-threads=1`;
- focused `recovery_integration` sync-repair tests;
- `bash scripts/check.sh`.

The existing CI workflow and manual release process remain unchanged. The
GitHub link-check could not be run locally because `lychee` is not installed.

## 8. Final closure criteria

All statements below must be true:

- [ ] `sync repair` ignores worker, execution, and pending lock metadata;
- [ ] `sync repair --apply` preserves the three canonical lock files byte-for-byte;
- [ ] no repair branch deletes or quarantines a canonical lock file;
- [ ] existing status and temporary-file repairs still work;
- [ ] `snip-sync stop` accepts a structured PID record;
- [ ] `snip-sync stop` accepts a numeric legacy PID record on Unix;
- [ ] a dead legacy PID record is removed only after obtaining the server lock;
- [ ] a live unrelated PID is refused without `--force`;
- [ ] a changed replacement PID record is not removed;
- [ ] `restart` delegates through the legacy-aware stop path;
- [ ] malformed PID content is reported explicitly;
- [ ] Windows stop behavior is unchanged;
- [ ] no new dependency, crate, feature flag, helper binary, or CI job was added;
- [ ] the existing kernel-lock implementation was not rewritten;
- [ ] focused tests pass;
- [ ] `scripts/check.sh` passes;
- [ ] existing Linux, macOS, and Windows CI jobs pass on the final implementation commit;
- [ ] manual release remains unchanged;
- [ ] this plan records one exact final implementation SHA;
- [ ] Phase 11L remains closed.

When these criteria are satisfied, stop. Do not add additional hardening work to this line.
