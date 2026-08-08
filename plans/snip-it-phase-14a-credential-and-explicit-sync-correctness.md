# Phase 14A — Credential Backend and Explicit-Sync Correctness

Status: READY FOR IMPLEMENTATION

Parent roadmap: `plans/snip-it-phase-14-correctness-simplification-roadmap.md`

Reviewed code baseline: `f0ebd1a2246976217bf48260c2dbddd31163533d`

Reviewed repository head before planning: `c7a326f19afc77c9dd37e54448f9837fa494de04`

Roadmap commit: `60c7a241499ff80bfc6136b10650ce710041ef83`

Date: 2026-08-08

Execution target: smaller coding models operating sequentially with narrow context.

## 1. Purpose

This is the first Phase 14 implementation pass because it addresses two user-visible correctness defects without changing product scope:

1. the root client declares `keyring = "3"` but does not enable a real platform credential-store feature;
2. exact-selector run/clip paths do not honor `--sync` with the same semantics as their TUI paths.

Do not mix binary-size cleanup, command-dispatch refactors, persistence migration, auto-sync redesign, or transaction changes into this phase.

## 2. Upstream keyring fact to preserve

Keyring v3 requires clients to select credential-store features explicitly. The upstream 3.6.3 documentation gives the cross-platform example:

```toml
keyring = { version = "3", features = ["apple-native", "windows-native", "sync-secret-service"] }
```

The relevant stores are:

- macOS: `apple-native`;
- Windows: `windows-native`;
- Linux/Unix desktop: `sync-secret-service`.

Without a supported store feature, keyring uses its mock store as the default. That is not acceptable as the production credential persistence mechanism for `snp`.

Do not add `vendored`, async Secret Service, a new keychain crate, or a custom encrypted credential file unless the documented native feature set is proven unusable for a supported target.

## 3. Allowed files

Primary production files:

```text
Cargo.toml
Cargo.lock
src/config.rs
src/main.rs
src/commands/mod.rs
src/commands/run_cmd.rs
src/commands/clip_cmd.rs
```

Tests may be added or amended only where they directly prove these two defects. Prefer existing selector/sync/config tests over a new test harness.

Documentation after production behavior is final:

```text
AGENTS.md
architecture/sync.md
.skills/keychain-integration.md
README.md              # only if user-facing credential behavior text is inaccurate
```

Do not touch:

```text
src/transaction.rs
src/auto_sync/pending*.rs
src/sync.rs batching/merge logic
snip-sync/src/orchestration.rs
snip-proto/**
.github/**
```

unless a compile failure demonstrates a direct dependency on the requested change.

## 4. Workstream A — Enable real keyring stores

### 4.1 Manifest change

Start with the upstream-documented cross-platform feature set on the existing dependency:

```toml
keyring = {
    version = "3",
    features = ["apple-native", "windows-native", "sync-secret-service"]
}
```

If formatting conventions favor one line, keep it one line. The important requirement is the feature set, not layout.

Do not enable every keyring feature. In particular, do not add both async and sync Secret Service implementations.

### 4.2 Linux build caveat

`sync-secret-service` may require system DBus development/linking support at build time. If Linux CI fails because the runner lacks the required system package:

1. record the exact build error;
2. determine whether the supported release/build environment already provides the required DBus library;
3. prefer the smallest CI package-install adjustment if the runtime target is a normal Linux desktop with Secret Service;
4. do not immediately use keyring's `vendored` feature, because that increases binary/build surface;
5. do not silently fall back to the mock store.

If Linux credential persistence is intentionally unsupported on headless installations, keep the existing explicit `SNP_ALLOW_PLAINTEXT_API_KEY=true` escape hatch and document that limitation. Do not make plaintext the default.

### 4.3 Config behavior

Retain the current storage contract in `src/config.rs`:

- real API key in memory;
- `@keychain` marker in `sync.toml` when native storage succeeds;
- keychain lookup on load;
- plaintext only when explicitly opted into;
- API key zeroized on drop;
- existing migration from legacy plaintext to keychain.

Do not introduce a second credential abstraction in this phase.

### 4.4 Verification

Required compile verification:

```text
cargo check -p snip-it
cargo test -p snip-it config
```

Platform CI must compile the client on Linux, macOS, and Windows after the feature change.

Manual native-store smoke is desirable on at least one interactive supported desktop:

```text
save a temporary test credential through the same keyring crate/config helper
read it back
remove it
```

Do not make CI depend on an unlocked desktop keychain or Secret Service session.

### 4.5 Acceptance criteria

- [ ] `Cargo.toml` intentionally enables supported keyring store features.
- [ ] `Cargo.lock` contains the target backend dependencies expected by those features.
- [ ] No production path relies on the keyring mock store as durable persistence.
- [ ] Explicit plaintext fallback behavior is unchanged.
- [ ] Linux/macOS/Windows compile coverage remains green.
- [ ] No new credential-storage dependency is added.

## 5. Workstream B — Create one canonical explicit-sync helper

### 5.1 Baseline problem

`run_snippet_selection()` contains the real explicit-sync sequence:

1. acquire the shared sync execution lock;
2. observe pending generation;
3. run canonical sync;
4. retain pending state on failure;
5. clear only the observed generation on success.

The exact run path instead calls `notify_mutation(SnippetRun, ...)`, which schedules/records auto-sync rather than honoring the requested immediate `--sync` operation.

The exact clip path ignores the parsed `sync` flag.

### 5.2 Preferred helper shape

Extract only the repeated explicit-sync mechanics into one helper, preferably in `src/commands/mod.rs` or `src/sync_commands.rs` if that location already owns the needed primitives.

Conceptual shape:

```rust
fn run_explicit_sync(runtime: &tokio::runtime::Runtime) -> ExplicitSyncResult
```

The helper must:

1. derive the canonical auto-sync state directory;
2. wait for `execution_lock` using the existing timeout;
3. observe pending generation after lock acquisition and immediately before sync;
4. call `sync_commands::run_default_sync(runtime)`;
5. call `clear_pending_after_explicit_sync(observed, succeeded)` exactly once;
6. preserve the current user-facing failure policy used by TUI `--sync` paths.

Do not create a generic sync executor trait or a second lock type.

### 5.3 Preserve current exit semantics

This phase fixes parity; it does not redefine `--sync` exit codes.

If current TUI `run --sync`/`clip --sync` logs a failed explicit sync but returns the local operation result, exact-selector paths must match that behavior.

Do not make exact-selector sync failures fatal unless the TUI path is intentionally changed in the same narrow helper and existing documented behavior proves that was the intended contract.

## 6. Workstream C — Fix exact `run --sync`

Primary files:

```text
src/main.rs
src/commands/run_cmd.rs
```

Required behavior:

```text
snp run --id <id> --sync
snp run --description-exact <desc> --sync
snp run --command-exact <cmd> --sync
```

must execute the selected command and then use the same explicit-sync helper as TUI `snp run --sync`.

Remove the current substitution of:

```text
notify_mutation(MutationKind::SnippetRun, ...)
```

for explicit sync. Running a snippet changes local usage metadata, not synced snippet content; the `--sync` flag means perform sync now, not schedule the post-mutation worker.

If variable expansion is cancelled or the command never executes, preserve the current exact-run cancellation behavior and do not add a new sync attempt solely because `--sync` was present.

If child execution fails, match the existing TUI path's decision about whether explicit sync still runs. Do not invent a different rule for exact targeting.

## 7. Workstream D — Fix exact `clip --sync`

Primary files:

```text
src/main.rs
src/commands/clip_cmd.rs
```

Required behavior:

```text
snp clip --id <id> --sync
snp clip --description-exact <desc> --sync
snp clip --command-exact <cmd> --sync
```

must copy the expanded command and then use the canonical explicit-sync helper.

The `sync` argument may no longer be discarded by exact dispatch.

Do not duplicate the full clipboard operation just to add sync. A deeper clip deduplication is Phase 14C; this phase may make the smallest signature/call-site change needed for correct parity.

## 8. Focused tests

Add direct regression coverage for both exact paths.

Required cases:

1. exact run with `--sync` reaches canonical explicit sync;
2. exact clip with `--sync` reaches canonical explicit sync;
3. exact run without `--sync` does not initiate network sync;
4. exact clip without `--sync` does not initiate network sync;
5. failed explicit sync preserves pending generation just as the TUI path does;
6. run exact no longer records a fake `SnippetRun` mutation solely to implement `--sync`.

Prefer existing in-process sync server/test helpers. Do not add an external live-wire dependency.

If process-level clipboard testing is not reliable in headless CI, test the exact dispatch/helper boundary below the OS clipboard call rather than adding a fake system clipboard framework.

## 9. Routine verification

After focused tests pass:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check.sh
```

Do not run `release-check.sh verify` until the implementation commit is stable and the working tree is clean.

## 10. Stop conditions

Stop and amend the plan instead of broadening scope if:

- a supported platform cannot compile any intended keyring native backend;
- using a real keyring backend requires replacing the existing credential model;
- exact sync parity exposes a pre-existing TUI semantic contradiction that cannot be resolved without changing documented exit behavior;
- the only proposed test requires new global mocks, serial-test dependencies, or a new process supervisor.

## 11. Final acceptance criteria

- [ ] Native credential-store features are enabled intentionally.
- [ ] Existing explicit plaintext fallback remains opt-in only.
- [ ] TUI and exact run use one explicit-sync implementation.
- [ ] TUI and exact clip use one explicit-sync implementation.
- [ ] Exact `run --sync` no longer substitutes auto-sync notification for explicit sync.
- [ ] Exact `clip --sync` no longer drops the flag.
- [ ] Pending generation is only cleared after successful canonical sync and only if unchanged.
- [ ] No new daemon, queue, credential subsystem, or sync implementation is introduced.
- [ ] Focused tests pass.
- [ ] `bash scripts/check.sh` passes.

## 12. Suggested implementation commit

```text
phase-14a: fix native credentials and exact sync parity
```

After implementation, record the implementation SHA in this plan and change status only when the listed acceptance criteria are actually satisfied.
