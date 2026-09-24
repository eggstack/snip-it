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

- `check.sh` passes `--features test-support` to all focused targets (`platform_smoke`, `destination_permissions`, `auto_sync_closure`, plus `-- --test-threads=1` for `auto_sync_concurrency`, `sync_multibatch`). Only `repair_transactions`, `process_lock_concurrency`, `local_data_lock_barriers` (plus the `process_lock_helper` bin) gate compilation on it (see `required-features` in `Cargo.toml`).
- Integration/PTY/lock/barrier tests are serial: always `--test-threads=1` for `pty_integration` (real pty pairs), `*_concurrency`, `sync_multibatch`, `*_barriers`, `repair_transactions`. Deep crash/restore/manifest suites run only in `release-check.sh verify`, not CI.
- Windows CI runs `pwsh -NoProfile -File scripts/tests/installers.ps1` (needs `pwsh`); Linux `check.sh` runs the bash equivalent.

## Toolchain & platform

- Rust 1.94, edition 2024 (`rust-toolchain.toml`); `rustfmt.toml`: `max_width=100`, 4-space, Unix newlines.
- `.cargo/config.toml` sets Windows MSVC link `/STACK:8388608` — the large `Commands` enum overflows the 1 MB default. Do not remove.
- Linux needs `libdbus-1-dev`, `pkg-config`, OpenSSL headers (CI installs the first two).
- `clippy.toml` relaxes `too-many-arguments` (10) and type complexity (350) — don't "fix" violations against default thresholds.

## Layout

- `snip-it/` binary `snp` (`src/main.rs`); `snip-proto/` protobuf + checked-in tonic stubs; `snip-sync/` server (Tonic gRPC + EggServe HTTP/1 leaf runtime); `tests/` (~50 targets); `scripts/check.sh`, `release-check.sh`; `themes/` Halloy TOML.
- `src/`: `commands/` (one module per command, canonical `*Args` beside handler; `snp data` is an alias, single-path `handle_*` in `main.rs`), `library/` (`model`/`persistence`/`manager`, canonical read-only resolver + `inspect_library_index`), `selector.rs` (fuzzy `SearchFields`/`searchable_text` + `resolve_exact_target`; mutating `resolve_selector` vs side-effect-free `resolve_selector_readonly`), `sync.rs` (gRPC client, single `retry_grpc_unified!` macro), `sync_commands.rs` (merge), `transaction.rs` + `local_data.rs` + `process_file_lock.rs` (journal + lock hierarchy), `auto_sync/` (detached `auto-sync-worker`, same `SyncExecutionLock` as manual sync), `mcp/` (read-only, stdio-only, non-executing), `ui/` (ratatui), `config/` (`sync_settings`/`toml_cache`), `update.rs`, `error.rs` (`SnipError`/`SnipResult`, no credentials), `outcome.rs` (`CliOutcome` → exit codes, see `docs/EXIT_CODES.md`).
- Tests use `TempDir` + `XDG_CONFIG_HOME` override, `sqlite::memory:` for servers, `tests/support/` (`TestEnvironment`, `RecordingServer`, `EventSink`); never touch real config/keychain/ports. `SNP_ALLOW_PLAINTEXT_API_KEY=true` on all test commands — never remove that seam. `set_var` in tests needs `unsafe` (edition 2024).

## Generated code — never edit

- `src/ui/_generated_bundled_themes.rs` ← `python3 scripts/build_themes.py`.
- `snip-proto/src/snip_proto.rs` is checked in; `protoc` needed only for an explicit regen after editing `proto/sync.proto`.

## Gotchas that break builds or corrupt state

- `gate_mutation_on_interrupted_transactions()` before every local mutation. One journal = auto-rollback; multiple/incomplete = refuse, direct to `snp repair`. Journals live in `<config>/.transaction/`; transaction APIs take `.transaction`, pending-marker APIs take the state dir.
- Kernel locks are authoritative: `flock`/`LockFileEx` for auto-sync locks and server singleton. `Drop` releases without unlinking; lock files may hold stale metadata. `kill(pid,0)`: only `ESRCH` proves absence (`EPERM`/unknown = live). Linux start tokens use `/proc/<pid>/stat` field 22.
- Save path does NOT post-process `toml::to_string_pretty`. Golden corpus (tabs, trailing spaces, CRLF) must survive save/load; `write_schema_version` must use `toml::Table`, not `toml::Value`, to preserve array-of-tables.
- `sync.rs` RPCs take `&mut self`; the retry macro expands inline so `self.client.<rpc>()` reborrows work. Do not reintroduce a closure-based generic retry helper (fails with "captured variable cannot escape `FnMut`").
- `snp update` uses in-process `eggfetch-core =0.2.0` (`standard-http1`+`redirects`+`tls-rustls`+`tls-native-roots` only; strict eggfetch redirects + native `Timeout.total`, snip-it keeps the initial-HTTPS guard); `snip-sync update` keeps external `curl`: the fresh 0.2.0 lean-profile trial grew the server from 3,833,152 to 5,145,224 bytes (+34.23%), past the 10% gate. `tests/architecture.rs` pins the no-`curl`/lean-profile/delegated-timeout properties; transport tests live in `src/update.rs` (`test-support`) with a std-only loopback fixture.
- `snip-sync` HTTP is one concrete two-route EggServe leaf service in `snip-sync/src/http.rs`, using only `eggserve-server =0.2.1` and `eggserve-primitives =0.2.0`; do not restore Axum/Tower-HTTP or add a generic router/middleware layer. Tonic remains on its separate gRPC listener. Keep the pre-bound listener, explicit body rejection, disabled total connection lifetime, typed EggServe shutdown completion, external TLS termination, Basic-auth constant-time comparison, configured CORS policy, and three security headers.
- Tokio: global `RUNTIME` only for async commands (`run`, `clip`, `search`, `sync`, `register`, `premade`, `update`); local-only commands never init it. `run_snippet_selection` takes `Option<&Runtime>` (`None` when `do_sync` false). Detached worker uses `new_current_thread()`; keep client's `rt-multi-thread` feature. Keep `keyring = "4"` default features (platform stores) or persistence silently falls back to mock store.
- Do not sanitize snippet commands (by design); removing CLI flags is breaking (deprecate first); Argon2 parameter changes break all existing encrypted payloads (version first); `commands/mod.rs` (`load/save_snippets`, `run_snippet_selection`) and `ui/mod.rs` re-exports affect all TUI commands; clipboard side effects go through `copy_to_clipboard()` in `clip_cmd.rs`.
- `AGENTS.override.md` has session pitfall notes — consult it.

## Sync & persistence invariants

- Conflict: `(updated_at, device_id, SHA-256(synced fields))`; never role-dependent server-wins. Deletion beats live content even with an older timestamp (no resurrection). `output`/`folders`/`favorite` are local-only, excluded from the fingerprint; `output` is not in `ProtoSnippet`, `snp edit --output` requires `--filter`.
- Uploads byte-bounded via Prost `encoded_len()` (client 3.5 MiB < server 4 MiB gRPC limit); `PushSnippets` idempotent by snippet identity; multi-batch errors preserve the original `SyncFailureKind` via `add_batch_context()`.
- Scheduling errors are typed — never collapse pending-read/spawn failures into `NoPending`/`SpawnNow`/success. Pending generations are monotonic (lower generation = corrupt, preserve marker, no spawn), except lower-generation + strictly newer timestamp = marker cleared and re-recorded, adopt as new work.
- Malformed library/`libraries.toml` fails closed (best-effort backup + error, never synthesize writable empty); missing/empty files give defaults. Missing-library recovery uses atomic `<library>.sync_recovery` state: preserve corrupt markers, one normalized remote-name match only, fail on ambiguity, remove marker only after relink + retry sync are durable, linkage + `last_sync` reset in one save.
- Search parity: `snp get --query`, `snp list --filter`, MCP `snippets_search` share `selector::searchable_text` (description + command always, tags by default, output/notes only with `list --search-output` / `search_output`); folders/favorite/sync metadata/credentials never searchable. MCP `snippet_get`: ID (case-sensitive) / description / command (case-insensitive), exactly one required.
- Tests assert exact counts (not `>= 1`), prove server-side effects, verify pending-clear ordering; helper emits JSON-lines lifecycle events only when `SNP_TEST_EVENTS_DIR` is set.

## Release & branches

- Publish is manual local, no CI token/workflow publishes: order `snip-proto` → `snip-sync` → `snip-it`; proto changes require bumping its version in both dependents. Version bump + `CHANGELOG.md` in one PR (`CONTRIBUTING.md`, `RELEASING.md`). Topic branches squash-merge to `main`; imperative mood, first line <72 chars.

## Pointers (don't duplicate)

- `AGENTS.override.md`, `architecture/` (per-command deep-dives), `.skills/` (sync, transactions-and-auto-sync, server, encryption, remediation, UI), `docs/` — check headers: evergreen refs (`EXIT_CODES`, `PERSISTENCE_INVENTORY`, `THREAT_MODEL`, `COMMAND_CONTRACTS`) vs historical snapshots (`SECURITY_AUDIT`, `FEATURE_BOUNDARIES`).
