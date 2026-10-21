# Testing & Test Seams Skill

## Purpose
Guide agents through writing and running tests in this workspace: the
isolation rules that keep tests out of the developer's real state, the
`test-support` feature gate that keeps test-only code out of production
binaries, which targets must run serially, and which suites CI does *not*
run.

Deep dive: [`../architecture/test-infrastructure.md`](../architecture/test-infrastructure.md).
This file is the task-shaped shortlist; the deep dive has the full target
inventory, the PTY handshake, and the failpoint catalogue.

---

## Rule 1: isolation is not optional

A test that touches local state resolves its config directory from
`crate::config::derive_sync_state_dir()` — **not** from the library path it is
writing to. Get this wrong and a unit test writes to the developer's real
`~/.config/snp/`, contends on the real local-data lock, and can be refused
outright by the mutation gate.

| Test shape | How to isolate |
|-----------|----------------|
| Subprocess (`snp` binary) | `TempDir` + set `XDG_CONFIG_HOME` to its parent (`tests/support/mod.rs::snp_in`) |
| In-process library call | `ScopedConfigDir` — the thread-local override |
| Spawned helper thread | `XDG_CONFIG_HOME` (a thread-local override does **not** propagate to child threads) |

`ScopedConfigDir` is `#[cfg(any(test, feature = "test-support"))]`
(`src/utils/config.rs:35`) and is re-exported from the crate root as
`snip_it::ScopedConfigDir` under the same gate (`src/lib.rs:95-97`). **An
integration test can only reach it when `test-support` is enabled** — which is
why `tests/identity_contract.rs` is gated on that feature.

Bind the guard to the `TempDir`'s lifetime so a panicking test still restores
the previous value:

```rust
// src/library/tests.rs:17-20 — the canonical pattern
struct TestDir {
    tmp: TempDir,
    _config_dir: crate::utils::config::ScopedConfigDir,
}
```

---

## Rule 2: the eight test seams are `test-support`-gated

Every test-only environment variable is read inside a
`#[cfg(feature = "test-support")]` block, so the **variable name is absent
from a production binary** — not merely ignored.

| Seam | Effect |
|------|--------|
| `SNP_TEST_FAILPOINT` | Abort the process at a named failpoint |
| `SNP_TEST_INJECT_ERROR` | Return a recoverable error at a named point |
| `SNP_TEST_MUTATION_BARRIER_DIR` | Block a mutation at a barrier point |
| `SNP_SKIP_WORKER_SPAWN` | Do not spawn the detached auto-sync worker |
| `SNP_TEST_EVENTS_DIR` | Emit JSON-lines lifecycle events to `<dir>/test-events.jsonl` |
| `SNP_ALLOW_DIR_FSYNC_FAILURE` | Tolerate a parent-dir fsync failure |
| `SNP_ALLOW_PLAINTEXT_API_KEY` | Bypass **and forbid** keychain access |
| `SNP_TEST_CREDENTIAL_FILE` | Read the API key from a file |

Two consequences:

- **Setting a seam in a production binary does nothing.**
  `scripts/ci/test-production-seams.sh` proves this mechanically: it builds
  `snp` without `test-support` and greps the binary for the variable names.
  Scenario 3 is the strongest — it asserts the binary does not even *contain*
  `SNP_ALLOW_DIR_FSYNC_FAILURE`.
- **A new seam needs the `#[cfg(feature = "test-support")]` gate at every read
  site.** A runtime check is not enough; the cfg attribute is the protection.

`SNP_ALLOW_PLAINTEXT_API_KEY=true` is set on every test command that can
reach the credential store — it keeps the real OS keychain out of the test
path. Never remove it.

---

## Rule 3: `required-features` gates four targets and one bin

Exactly **five** `Cargo.toml` entries carry `required-features =
["test-support"]`:

| Entry | Why |
|-------|-----|
| `repair_transactions` | needs the public transaction recovery API |
| `process_lock_concurrency` | needs the same API |
| `local_data_lock_barriers` | needs the same API |
| `identity_contract` | needs `ScopedConfigDir` to redirect the config dir |
| `process_lock_helper` (bin) | helper binary for the kernel-lock race |

The other 47 targets compile with default features.

> **Clippy gap.** `cargo clippy --workspace --all-targets -- -D warnings` does
> **not** pass `--all-features`, so it does **not** lint those four targets.
> `scripts/check.sh` covers `platform_smoke`, `destination_permissions`, and
> `auto_sync_closure` with `--features test-support`, but
> `repair_transactions`, `process_lock_concurrency`, `local_data_lock_barriers`,
> and `identity_contract` are never linted by CI. Run
> `cargo clippy --workspace --all-targets --all-features -- -D warnings`
> locally before landing changes to them.

---

## Rule 4: serial targets are serial for a reason

Always `-- --test-threads=1` for:

| Target | Why |
|--------|-----|
| `pty_integration` | real pty pairs; concurrent sessions contend for terminal resources |
| `auto_sync_concurrency` | cross-process timing with real detached workers |
| `backup_snapshot_concurrency` | real writers paused mid-mutation while a backup takes the lock |
| `process_lock_concurrency` | kernel `flock`/`LockFileEx` contention across real subprocesses |
| `local_data_lock_barriers` | barrier protocol — the writer blocks while the backup contends |
| `repair_transactions` | barrier protocol plus `unsafe set_var` on process-global state |
| `sync_multibatch` | multi-batch upload ordering against one server |
| `snip_sync_lifetime` | real TCP ports and HTTP wire-byte assertions |
| `transaction_crash_recovery` | release-profile crash windows |

---

## Rule 5: assert exact outcomes

- **Exact counts, not `>= 1`** — `assert_eq!(count, 1, ...)`. Two documented
  exceptions exist (`transaction_crash_recovery.rs` asserts `>= 1` detected
  transaction; `recording_telemetry.rs` uses `>= 1` only as a polling guard
  inside a wait loop).
- **Prove server-side effects** — assert the remote state, not just the
  client's exit code.
- **Prove read-only paths by a filesystem snapshot** — a recursive byte
  snapshot of the config directory before and after
  (`tests/readonly_library_resolution.rs:17-35`), not the absence of an error.
- **Assert pending-clear ordering** — the remote effect happens *before* the
  pending marker is cleared; a pending clear is impossible when the canonical
  sync fails.

---

## Rule 6: know what CI does *not* run

`scripts/check.sh` (the Linux CI list) runs the installer contract, fmt,
clippy, `--workspace --lib`, and **six** integration targets:
`platform_smoke`, `destination_permissions`, `auto_sync_closure`,
`auto_sync_concurrency`, `sync_multibatch`, `snip_sync_lifetime`.

Two deep suites are invoked by **no script at all**:

```bash
cargo test --test restore_crash_failpoints --features test-support -- --test-threads=1
cargo test --test cleanup_crash_failpoints --features test-support -- --test-threads=1
```

They kill the real `snp restore` binary at typed failpoint boundaries. **A
change to the restore or cleanup state machine needs one of these run locally
before merge.** Do not assume CI covered it.

`scripts/release-check.sh verify` runs the deep recovery suite, the
`--ignored` HTTP lifetime tests, and the production-seam proof — but it
requires a clean tree.

---

## Adding a test — checklist

1. **Isolation.** `TempDir` + `XDG_CONFIG_HOME` for a subprocess test;
   `ScopedConfigDir` for an in-process call. No exceptions.
2. **Seams.** Set only the seam you need. Never remove the `env_remove` block
   in `snp_cmd()` / `base_snp_cmd()` — it stops a stray variable in the
   developer's shell from changing test behavior.
3. **Credentials.** `SNP_ALLOW_PLAINTEXT_API_KEY=true` on every command that
   can reach the credential store.
4. **Ports.** In-process servers bind `127.0.0.1:0`. Negative tests point at
   `127.0.0.1:1`, not a non-routable address, so failure is fast and local.
5. **Edition 2024.** `std::env::set_var` is `unsafe` — wrap it in an explicit
   `unsafe { }` with a SAFETY comment.
6. **Shared helpers.** Prefer `tests/support/` (`snp_cmd`, `setup_test_env`,
   `snp_in`, `TestEnvironment`, `RecordingServer`) over a new local copy.
   `golden_corpus()` (`tests/support/mod.rs:157`) holds 24 edge-case commands
   for TOML byte fidelity.
7. **Feature gate.** If the test needs `ScopedConfigDir` or the public
   transaction API, add a `[[test]]` entry with
   `required-features = ["test-support"]` in `Cargo.toml`.
8. **Serialization.** If the test touches a real pty, a kernel lock, a
   barrier, or a fixed port, it is serial — document that.

---

## Common mistakes

- Writing a unit test that calls `save_library` without a `ScopedConfigDir`.
- Assuming `cargo test --workspace --lib` covers integration behavior — it
  does not.
- Believing `check.sh` lints the `required-features` targets. It does not.
- Assuming CI runs the crash/failpoint suites. It does not.
- Removing the `env_remove` block from the shared command helpers.
- Using `>= 1` where an exact count is available.
- Proving a read-only path by "no error" instead of a filesystem snapshot.
- Running `flock`-based lock tests in an unprivileged Docker container —
  `flock` silently degrades to `fcntl` and mutual exclusion is then **not**
  proven. Needs `SYS_ADMIN` or `--privileged`.
