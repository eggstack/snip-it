# AGENTS.md

## Verify (authoritative)

```bash
bash scripts/check.sh   # same as Linux CI: installers.sh, fmt, clippy, lib + focused integration
bash scripts/release-check.sh verify   # manual pre-release, requires clean tree
bash scripts/release-check.sh dry-run snip-it   # per-crate publish dry-run (snip-proto|snip-sync|snip-it)
bash scripts/ci/test-production-seams.sh   # test-only env vars inactive in prod builds
```

```bash
cargo fmt --all -- --check   # or: cargo fmt
cargo clippy --workspace --all-targets -- -D warnings   # NOT --all-features; test-only code lints via explicit targets below
cargo test --workspace --lib   # parallel-safe (isolated TempDir per test)
cargo test --test <name> --features test-support [-- --test-threads=1]   # single integration target; copy flags from check.sh
cargo test --workspace --all-features -- --test-threads=1   # full suite, serial
cargo test -p snip-sync --features test-helpers
```

- `check.sh` passes `--features test-support` to all focused targets. Only `repair_transactions`, `process_lock_concurrency`, `local_data_lock_barriers`, `identity_contract`, `library_manager_contracts`, `restore_manifest_gate` (plus the `process_lock_helper` bin) gate compilation on it (`required-features` in `Cargo.toml`); `identity_contract` needs the `test-support`-only `ScopedConfigDir` export, or `save_library` resolves the mutation gate and the local-data lock against the developer's real `~/.config/snp/`. Those targets are **never linted by `check.sh`** (it does not pass `--all-features`) — lint them locally with `--all-features` before landing changes.
- Integration/PTY/lock/barrier tests are serial: always `--test-threads=1` for `pty_integration` (real pty pairs), `*_concurrency`, `sync_multibatch`, `*_barriers`, `repair_transactions`. Deep crash/restore/manifest suites run only in `release-check.sh verify`, not CI — and `cleanup_crash_failpoints` / `restore_crash_failpoints` are invoked by **no script at all**; run them by hand before merging a restore or cleanup change.
- **Never run `cargo build`/`cargo test`/`cargo clippy` while a test run is in flight.** The integration tests execute `target/debug/snp` (`env!("CARGO_BIN_EXE_snp")`); a rebuild replaces that binary mid-run and produces false failures — most visibly the `test-support`-gated crash suites, whose failpoint seam silently disappears when the binary is rebuilt with default features.
- Full test/seam rules (isolation, the eight seams, `required-features`, assertion discipline, what CI skips): `.skills/testing-and-seams.md`.
- macOS/Windows CI only runs `cargo check --workspace --all-targets` + `platform_smoke`; Windows additionally runs `pwsh -NoProfile -File scripts/tests/installers.ps1` (needs `pwsh`). Linux `check.sh` runs the bash equivalent.
- `cargo deny check` is a local gate only — no CI step runs it (`deny.toml`).

## Toolchain & platform

- Rust 1.94, edition 2024 (`rust-toolchain.toml`); `rustfmt.toml`: `max_width=100`, 4-space, Unix newlines.
- `.cargo/config.toml` sets Windows MSVC link `/STACK:8388608` — the large `Commands` enum overflows the 1 MB default. Do not remove.
- Linux CI installs `libdbus-1-dev` and `pkg-config` (see `.github/workflows/ci.yml`). No OpenSSL: the tree is rustls-only (`tonic/tls-ring`, `eggfetch-core/tls-rustls`) and `Cargo.lock` has no `openssl-sys`.
- `clippy.toml` relaxes `too-many-arguments` (10) and type complexity (350) — don't "fix" violations against default thresholds.

## Layout

- `snip-it/` binary `snp` (`src/main.rs`); `snip-proto/` protobuf + checked-in tonic stubs; `snip-sync/` server (Tonic gRPC + EggServe HTTP/1 leaf runtime); `tests/` (51 top-level targets); `scripts/check.sh`, `release-check.sh`; `themes/` Halloy TOML (50).
- `src/`: `commands/` (26 modules; only 16 define a canonical `*Args` — the group commands `sync_cmd`/`import_cmd`/`library_cmd`/`premade_cmd`/`shell_cmd` keep their Clap schema in `src/main.rs`, so flag edits there touch `main.rs`), `library/` (`model`/`persistence`/`manager`, canonical read-only resolver `readonly_library_sources` + `inspect_library_index`), `selector.rs` (fuzzy `SearchFields`/`searchable_text` + `resolve_exact_target`; mutating `resolve_selector` vs side-effect-free `resolve_selector_readonly`), `sync.rs` (gRPC client, single `retry_grpc_unified!` macro), `sync_commands.rs` (merge), `sync_failure.rs` (`SyncFailureKind`/`FailureClass`), `status_snapshot.rs`, `transaction.rs` + `local_data.rs` + `process_file_lock.rs` (journal + lock hierarchy), `auto_sync/` (detached `auto-sync-worker`, same `SyncExecutionLock` as manual sync), `mcp/` (read-only, stdio-only, non-executing *on the `serve` path*), `ui/` (ratatui), `config/` (`sync_settings`/`toml_cache`), `update.rs`, `error.rs` (`SnipError`/`SnipResult`), `outcome.rs` (`CliOutcome` → exit codes, see `docs/EXIT_CODES.md`), `test_failpoints.rs` (test-only seams, compiled out without `test-support`).
- `snp data` is a **mirror group** of the top-level `validate`/`backup`/`restore`/`repair`/`status`/`delete` spellings, not a Clap alias. Those six are the only commands with single-path `handle_*` functions in `main.rs`; the other ~20 dispatch inline inside `dispatch_command`. Adding a `snp data <x>` needs three edits: a `DataCommands` variant, a dispatch arm, and a `command_behavior()` arm.
- Tests use `TempDir` + `XDG_CONFIG_HOME` override, `sqlite::memory:` for servers, `tests/support/` (`TestEnvironment`, `RecordingServer`, `EventSink`); never touch real config/keychain/ports. `SNP_ALLOW_PLAINTEXT_API_KEY=true` on all test commands — never remove that seam. `set_var` in tests needs `unsafe` (edition 2024).

## Generated code — never edit

- `src/ui/_generated_bundled_themes.rs` ← `python3 scripts/build_themes.py`.
- `snip-proto/src/snip_proto.rs` is checked in; `protoc` needed only for an explicit regen after editing `proto/sync.proto`.

## Gotchas that break builds or corrupt state

Each cluster of traps below has one authoritative section in
`architecture/overview.md` § 7 (Cross-Cutting Invariants), or a deep dive.
Follow the link rather than re-deriving the rule.

| Trap cluster | Authoritative section |
|--------------|-----------------------|
| Read-only paths that mutate; `resolve_selector` vs `resolve_selector_readonly` | [overview § 7](architecture/overview.md#read-only-paths-must-not-mutate), [selector.md](architecture/selector.md) |
| Mutation gate, journals, recovery classes | [overview § 7](architecture/overview.md#gate-every-local-mutation), [transactions-and-auto-sync.md](.skills/transactions-and-auto-sync.md) |
| Durability classes, atomic write, fsync | [overview § 7](architecture/overview.md#writes-go-through-the-atomic-layer-with-named-durability-classes), [persistence.md](architecture/persistence.md) |
| Lock hierarchy, kernel locks, stale metadata | [overview § 7](architecture/overview.md#locks-are-kernel-backed-and-ordered), [process_file_lock.md](architecture/process_file_lock.md) |
| Client merge order, server's weaker rule | [overview § 7](architecture/overview.md#client-conflicts-are-never-role-dependent), [sync.md](architecture/sync.md) |
| Global `snippets.id`, cross-library push dropped | [overview § 7](architecture/overview.md#server-snippetsid-is-global-and-a-cross-library-push-is-dropped), [server.md](architecture/server.md) |
| Search parity across the three surfaces | [overview § 7](architecture/overview.md#search-has-three-surfaces-and-one-definition), [selector.md](architecture/selector.md) |
| TOML fidelity, golden corpus | [overview § 7](architecture/overview.md#the-save-path-does-not-post-process-toml), [persistence-and-toml-fidelity.md](.skills/persistence-and-toml-fidelity.md) |
| Deterministic snippet IDs | [overview § 7](architecture/overview.md#snippet-ids-are-deterministic), [IDENTITY_CONTRACT.md](docs/IDENTITY_CONTRACT.md) |
| Tokio init, Windows stack size | [overview § 7](architecture/overview.md#tokio-is-initialized-only-where-it-is-needed), [overview § 7](architecture/overview.md#windows-stack-size) |
| Upload ceilings, typed scheduling errors | [overview § 7](architecture/overview.md#uploads-are-byte-bounded), [overview § 7](architecture/overview.md#scheduling-errors-are-typed) |
| Exit codes, stdout/stderr split, `snp data` mirror | [outcome.md](architecture/outcome.md), [cli-and-outcome-contract.md](.skills/cli-and-outcome-contract.md) |

- `gate_mutation_on_interrupted_transactions()` before every local mutation. One journal = auto-rollback; multiple/incomplete = refuse, direct to `snp repair`. Corrupt journals and `RecoveryClass::UnsafeFailed` also refuse (fail-closed) before the one-journal path. Journals live in `<config>/.transaction/`; transaction APIs take `.transaction`, pending-marker APIs take the state dir.
- Kernel locks are authoritative: `flock`/`LockFileEx` for auto-sync locks and server singleton. `Drop` releases without unlinking; lock files may hold stale metadata. `kill(pid,0)`: only `ESRCH` proves absence (`EPERM`/unknown = live). Linux start tokens use `/proc/<pid>/stat` field 22.
- Save path does NOT post-process `toml::to_string_pretty`. Golden corpus (tabs, trailing spaces, CRLF) must survive save/load. The persisted library format carries NO schema version; introducing one is a schema change that requires a migration and a version bump. Corpus + details: `.skills/persistence-and-toml-fidelity.md`.
- Snippet IDs are **deterministic**, not UUID v4: `legacy-<sha256 hex>` from content + occurrence index (`normalize_snippet_ids`, `src/library/persistence.rs`). The one exception is `import_cmd` (`src/commands/import_cmd.rs:145`), which assigns `Uuid::new_v4()` and discards the source ID. See `docs/IDENTITY_CONTRACT.md`.
- Read-only paths use `resolve_selector_readonly`, never `resolve_selector` — the latter runs `ensure_library_mode()` and can migrate/rewrite metadata. `resolve_exact_target` is in the **mutating** family (it calls `resolve_selector`), so `snp run/clip/edit --id` can rewrite metadata while `snp get` and MCP cannot.
- `sync.rs` RPCs take `&mut self`; the retry macro expands inline so `self.client.<rpc>()` reborrows work. Do not reintroduce a closure-based generic retry helper (fails with "captured variable cannot escape `FnMut`").
- `is_retryable_grpc_error` is deliberate: `OutOfRange` (size verdict) is **not** retryable, `ResourceExhausted` (server rate limit) **is**. Don't "simplify" the match — one way loops forever on oversized payloads, the other stops retrying rate limits.
- Split transports by measurement, don't unify: `snp update` uses in-process lean `eggfetch-core` (see `Cargo.toml`), `snip-sync update` keeps external `curl` (embedded trial bloated the small server +34%, past the 10% gate). `tests/architecture.rs` pins this; transport tests live in `src/update.rs` (`test-support`) with a std-only loopback fixture.
- `snip-sync` HTTP is one concrete two-route EggServe leaf in `snip-sync/src/http.rs` (`/health`, `/metrics`) — no Axum/Tower-HTTP, no generic router/middleware; Tonic stays on its separate listener. Read `tests/snip_sync_lifetime.rs` before touching status codes, `Allow`/`Vary`, or security headers (wire parity is locked there). Detail: `.skills/server-module.md`, `architecture/server.md`.
- Tokio: global `RUNTIME` only for async commands (`run`, `clip`, `search`, `sync`, `register`, `premade`, `update`); local-only commands never init it. `run_snippet_selection` takes `Option<&Runtime>` (`None` when `do_sync` false). Detached worker uses `new_current_thread()`; keep client's `rt-multi-thread` feature. Keep `keyring = "4"` with its default features — keyring 4's `default` is `v1`, which hard-enables the three platform stores (Apple Keychain, Windows Credential Manager, zbus Secret Service). `keyring::Entry` is gated on `v1`, so `default-features = false` is a **compile error**, not a silent downgrade; there is **no mock store**.
- Redaction is **Display-layer only, and only for `SnipError::Command`**. `SnipError::Runtime { detail }` and `SnipError::SyncFailure { detail }` print their detail verbatim — never put a credential in `runtime_error(msg, detail)`.
- **Every** test seam is `test-support`-gated and compiled out of production builds: `SNP_TEST_EVENTS_DIR`, `SNP_TEST_FAILPOINT`, `SNP_TEST_INJECT_ERROR`, `SNP_TEST_MUTATION_BARRIER_DIR`, `SNP_SKIP_WORKER_SPAWN`, `SNP_ALLOW_DIR_FSYNC_FAILURE`, `SNP_ALLOW_PLAINTEXT_API_KEY`, `SNP_TEST_CREDENTIAL_FILE`. Setting the env var in a production binary does nothing — `scripts/ci/test-production-seams.sh` proves it by grepping the production binary for the variable name. Detail: `.skills/testing-and-seams.md`.
- `snp mcp serve` binds no port and never executes stored commands; `snp mcp install` **does** spawn an external client process (`src/mcp/client_install.rs`).
- `tests/architecture.rs` only scans crate-root `crate::` path prefixes. `selector.rs`, `process_file_lock.rs`, `utils/*`, `transaction.rs`, `local_data.rs` are governed by convention, not by the scanner. Update `docs/LOGICAL_LAYERS.md` and the test's constant lists together.
- Do not sanitize snippet commands (by design); removing CLI flags is breaking (deprecate first); Argon2 parameter changes break all existing encrypted payloads (version first); `commands/mod.rs` (`load/save_snippets`, `run_snippet_selection`) and `ui/mod.rs` re-exports affect all TUI commands; clipboard side effects go through `copy_to_clipboard()` in `clip_cmd.rs`.
- The CLI surface is a published contract: exit codes, stdout/stderr split, and the `snp data` mirror are locked by tests. Every new subcommand needs a `command_behavior()` entry (startup-recovery policy) and a `docs/COMMAND_CONTRACTS.md` row. See `.skills/cli-and-outcome-contract.md`.
- `CliOutcome::ExecutionFailed { child_code }` propagates the child's exit code verbatim, so the process exit code is not confined to 0–10; exit code 10 is the one code with no `CliOutcome`. See `docs/EXIT_CODES.md` + `architecture/outcome.md`.

## Sync & persistence invariants

- Client merge conflict order: `(updated_at, device_id, SHA-256(synced fields))`; never role-dependent server-wins. Deletion beats live content even with an older timestamp (no resurrection) — **client-merge only** (`choose_version`, `src/sync_commands.rs`).
- The **server** uses a different rule: SQL last-write-wins upsert on `(updated_at, device_id, command, description)` with no SHA-256, and `deleted` is an ordinary column (`snip-sync/src/db.rs`). A newer live row therefore overwrites a tombstone on the server. Do not assume the server guarantees no resurrection.
- Server `snippets.id` is a global `TEXT PRIMARY KEY` (not per user/library) and the upsert is guarded by `WHERE excluded.user_id = snippets.user_id AND excluded.library_id = snippets.library_id`. A push with the same content-derived ID under a different library writes nothing — reported as `UpsertOutcome::ForeignScope` (rejected on push, `skipped_ids` on sync), but the write is still discarded and `last_sync` does not advance while `skipped_count > 0`, so a content-duplicated library cannot complete a sync until the content changes. Scope probe: `snip-sync/src/db.rs:749-770`. Detail: `architecture/overview.md#server-snippetsid-is-global-and-a-cross-library-push-is-dropped`.
- `output`/`folders`/`favorite` are local-only, excluded from the fingerprint; `output` is not in `ProtoSnippet` (`snip_proto::Snippet`), and `snp edit --output` requires `--filter` unless an exact selector (`--id`/`--description-exact`/`--command-exact`) is given.
- Uploads byte-bounded via Prost `encoded_len()`: client ceiling `DEFAULT_CLIENT_REQUEST_CEILING` = 3.5 MiB (`src/sync.rs`) < server `DEFAULT_GRPC_MAX_MESSAGE_SIZE` = 4 MiB (`snip-sync/src/lib.rs`). `PushSnippets` is idempotent by snippet identity (server upsert on `id`); multi-batch errors preserve the original `SyncFailureKind` via `add_batch_context()`.
- Scheduling errors are typed — never collapse pending-read/spawn failures into `NoPending`/`SpawnNow`/success. Pending generations are monotonic (lower generation = corrupt, preserve marker, no spawn), except lower-generation + strictly newer timestamp = marker cleared and re-recorded, adopt as new work.
- Malformed library/`libraries.toml` fails closed (best-effort backup + error, never synthesize writable empty); missing/empty files give defaults. Missing-library recovery uses atomic `<library>.sync_recovery` state: preserve corrupt markers, one normalized remote-name match only, fail on ambiguity, remove marker only after relink + retry sync are durable, linkage + `last_sync` reset in one save.
- Search parity: `snp get --query`, `snp list --filter`, MCP `snippets_search` share `selector::searchable_text` (description + command always, tags by default, output/notes only with `list --search-output` / `search_output`); folders/favorite/sync metadata/credentials never searchable. **Adding a field changes three surfaces with no compiler error** — see `.skills/selector-and-search-parity.md`. MCP `snippet_get`: ID (case-sensitive) / description / command (case-insensitive), exactly one required.
- Tests assert exact counts (not `>= 1`), prove server-side effects, verify pending-clear ordering; helper emits JSON-lines lifecycle events only when `SNP_TEST_EVENTS_DIR` is set **and** `test-support` is enabled.

## Release & branches

- Publish is manual local, no CI token/workflow publishes: order `snip-proto` → `snip-sync` → `snip-it`; proto changes require bumping its version in both dependents. Version bump + `CHANGELOG.md` in one PR (`CONTRIBUTING.md`, `RELEASING.md`). Topic branches squash-merge to `main`; imperative mood, first line <72 chars.

## Pointers (don't duplicate)

- `AGENTS.override.md` — session pitfall notes; consult it.
- `architecture/overview.md` — **start here.** Layer map, workspace crates, cross-cutting invariants, and the **Deep-Dive Index** covering all 61 deep dives (62 documents including the overview). Use it to reach any component; prefer extending that index over adding prose here. `architecture/commands/*.md` is one file per `src/commands/` module (26).
- `.skills/` — task-shaped agent reference docs (14). The table below is the index; each skill links to the deep dive that owns the detail, so a fact is stated once.
- `docs/README.md` — index of every reference doc, split into **current contracts** vs **archived snapshots** in `docs/archive/`. Check the status column before trusting a doc; archived ones describe code that no longer exists.
  `docs/USER_GUIDE.md` is the end-user walkthrough; `docs/COMMANDS.md` is the full subcommand reference; `docs/EXIT_CODES.md` is the exit-code table.
- `plans/registry.md` is the authoritative planning status — check it before assuming any roadmap state. Canonical direction: `plans/000-long-term-specification.md`, `001-terminology-and-domain-model.md`, `002-long-term-roadmap.md`; governance: `plans/003-planning-process.md`. Subsystem roadmaps live in `plans/subsystems/`, handoff plans in `plans/implementation/<subsystem>/`, completion gates in `plans/closure/<subsystem>/`, predecessors in `plans/archive/`.

### Skills (`.skills/`)

| Skill | Covers |
|-------|--------|
| `architecture-review.md` | How to review architecture docs against code; key-file map; evergreen-reference checklist |
| `planning.md` | The `plans/` convention: roadmaps, handoff plans, closure records, registry, status vocabulary |
| `cli-and-outcome-contract.md` | CLI surface: dispatch chain, exit codes, startup-recovery policy, stdout/stderr split, `snp data` mirror, deprecation policy |
| `selector-and-search-parity.md` | `snp get --query` / `snp list --filter` / MCP `snippets_search` share one function; readonly-vs-mutating resolvers; read-only stdio MCP boundary |
| `persistence-and-toml-fidelity.md` | Save path never post-processes TOML; golden corpus; durability classes; gate-before-mutation |
| `sync-module.md` | gRPC client, merge/conflict order, retry policy, failure classification, status snapshot |
| `transactions-and-auto-sync.md` | Journals, lock hierarchy, mutation gate, auto-sync worker contracts |
| `server-module.md` | `snip-sync` EggServe leaf, env vars, gRPC surface, curl-vs-eggfetch split |
| `encryption-module.md` | Argon2id + AES-256-GCM, what sync actually encrypts, key cache |
| `keychain-integration.md` | `keyring` usage, migration, the two test-only credential seams |
| `ui-module.md` | TUI event loop, themes, syntax highlighting, re-export contract |
| `remediation-patterns.md` | Cross-cutting fix patterns: atomicity, locks, dead public items, validation-first repair |
| `testing-and-seams.md` | Writing and running tests: isolation, the eight `test-support` seams, serial targets, what CI skips |
| `documentation-authoring.md` | Which doc owns which fact; citation discipline; adding a command; archive vs. prune |
