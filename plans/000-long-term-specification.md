# snip-it Long-Term Architecture and Product Specification

Status: canonical long-term implementation directive

Companion documents:

- `plans/001-terminology-and-domain-model.md`
- `plans/002-long-term-roadmap.md`
- `plans/003-planning-process.md`

This document defines the intended end state for snip-it. It establishes
product scope, crate boundaries, persistence and sync invariants, transport
policy, MCP boundaries, and acceptance criteria. The roadmap decomposes this
specification into ordered execution phases. The terminology document is
normative whenever older code, docs, or plans use overlapping terms such as
snippet, library, device, generation, journal, selector, or outcome.

The keywords MUST, MUST NOT, REQUIRED, SHOULD, SHOULD NOT, and MAY are normative.

## 1. Product definition

snip-it is a lightweight terminal snippet manager for individual developers
and small heterogeneous fleets. The `snp` binary (`snip-it` crate) owns
snippet storage, retrieval, execution, search, sync, and self-update.
`snip-sync` is one independently versioned foreground server binary owning
Tonic gRPC sync plus a two-route EggServe HTTP/1 health/metrics leaf service.
`snip-proto` owns the protobuf contract plus checked-in tonic stubs.

The same architecture MUST support three deployment forms without creating
separate products:

```text
Local only
    snp -- local library/persistence only

Self-synced
    snp <-> snip-sync (gRPC), one SyncExecutionLock owner

Fleet
    many snp installs -> prebuilt release binaries + bootstrap installers
                     -> optional snip-sync startup wrappers
                     -> read-only local MCP for agent clients
```

In all forms snip-it remains a terminal tool. It MUST NOT become a service
platform, plugin engine, database, search index, background job system, or
networked execution framework.

## 2. Primary product goals

snip-it MUST provide:

1. Fast local snippet CRUD, fuzzy search, TUI selection, and clipboard-first
   execution with stable command spellings and exit codes.
2. Durable local persistence with atomic saves, transaction journals, kernel
   file locks, and fail-closed corruption handling.
3. Byte-bounded, idempotent, conflict-typed sync against `snip-sync` with no
   resurrection and no role-dependent server-wins.
4. Binary-first distribution: prebuilt release executables, checksums,
   bootstrap installers with Cargo fallback, and binary-first self-update.
5. A read-only local stdio MCP adapter for snippet search/get plus
   client registration, without mutations or command execution.
6. A minimal `snip-sync` lifecycle (`serve`, `stop`, `restart`, `croncheck`,
   `/health`) reusable by systemd/launchd/cron/Task Scheduler wrappers.
7. Operator-observable behavior: exact exit codes, machine-output rules,
   bounded transfers, typed scheduling errors, and JSON-lines test events
   only under `SNP_TEST_EVENTS_DIR`.

## 3. Non-goals

snip-it is not:

- a package manager (no apt/dnf/Homebrew/Winget/Chocolatey/Snap/Flatpak);
- an auto-update daemon, notification service, differential updater, or
  signing infrastructure;
- a generic agent execution tool or networked MCP service;
- a service/repository abstraction, dependency-injection framework,
  rule/plugin engine, or retry middleware framework;
- a database, search index, generic query language, or background job system;
- an enterprise identity provider or CI definition system.

snip-it MAY integrate with those systems (e.g. OS startup wrappers, agent
MCP clients) where they support snippet workflows. It MUST NOT absorb their
scope.

## 4. Crate and module ownership

- `snip-proto`: `proto/sync.proto` plus checked-in `snip_proto.rs`. `protoc`
  is required only for explicit regen. Proto changes REQUIRE bumping the
  proto version in both dependents.
- `snip-it` binary `snp` (`src/main.rs`): `commands/` (one module per
  command, canonical `*Args` beside handler; `snp data` is an alias with a
  single-path `handle_*`), `library/` (`model`/`persistence`/`manager`,
  canonical read-only resolver plus `inspect_library_index`), `selector.rs`
  (fuzzy `SearchFields`/`searchable_text`, mutating `resolve_selector` vs
  side-effect-free `resolve_selector_readonly`), `sync.rs` (gRPC client,
  single `retry_grpc_unified!` macro), `sync_commands.rs` (merge),
  `transaction.rs` + `local_data.rs` + `process_file_lock.rs` (journal plus
  lock hierarchy), `auto_sync/` (detached `auto-sync-worker`, same
  `SyncExecutionLock` as manual sync), `mcp/` (read-only, stdio-only,
  non-executing), `ui/` (ratatui), `config/` (`sync_settings`/`toml_cache`),
  `update.rs` (lean eggfetch), `error.rs` (`SnipError`/`SnipResult`, no
  credentials), `outcome.rs` (`CliOutcome` to exit codes per
  `docs/EXIT_CODES.md`).
- `snip-sync`: Tonic gRPC on its own listener plus one concrete two-route
  EggServe leaf HTTP service in `snip-sync/src/http.rs` (`eggserve-server`
  plus `eggserve-primitives` only). No Axum/Tower-HTTP, no generic
  router/middleware layer.
- Tests: `TempDir` plus `XDG_CONFIG_HOME` override, `sqlite::memory:` for
  servers, `tests/support/` (`TestEnvironment`, `RecordingServer`,
  `EventSink`); never touch real config/keychain/ports.

## 5. Durability invariants

- `gate_mutation_on_interrupted_transactions()` runs before every local
  mutation. One journal means auto-rollback; multiple or incomplete journals
  mean refuse and direct to `snp repair`. Journals live in
  `<config>/.transaction/`; transaction APIs take `.transaction`,
  pending-marker APIs take the state dir.
- Kernel locks are authoritative: `flock`/`LockFileEx` for auto-sync locks
  and the server singleton. `Drop` releases without unlinking; lock files
  may hold stale metadata. `kill(pid,0)`: only `ESRCH` proves absence
  (`EPERM`/unknown means live). Linux start tokens use `/proc/<pid>/stat`
  field 22.
- The save path does NOT post-process `toml::to_string_pretty`. The golden
  corpus (tabs, trailing spaces, CRLF) MUST survive save/load.
  `write_schema_version` MUST use `toml::Table`, not `toml::Value`, to
  preserve array-of-tables.
- Malformed library/`libraries.toml` fails closed (best-effort backup plus
  error, never synthesize a writable empty); missing/empty files give
  defaults. Missing-library recovery uses atomic `<library>.sync_recovery`
  state: preserve corrupt markers, one normalized remote-name match only,
  fail on ambiguity, remove the marker only after relink plus retry sync
  are durable, with linkage plus `last_sync` reset in one save.

## 6. Sync invariants

- Conflict identity is `(updated_at, device_id, SHA-256(synced fields))`;
  never role-dependent server-wins. Deletion beats live content even with
  an older timestamp (no resurrection). `output`/`folders`/`favorite` are
  local-only, excluded from the fingerprint; `output` is not in
  `ProtoSnippet`, and `snp edit --output` requires `--filter`.
- Uploads are byte-bounded via Prost `encoded_len()` (client 3.5 MiB below
  the server 4 MiB gRPC limit); `PushSnippets` is idempotent by snippet
  identity; multi-batch errors preserve the original `SyncFailureKind` via
  `add_batch_context()`.
- Scheduling errors are typed and MUST never collapse pending-read/spawn
  failures into `NoPending`/`SpawnNow`/success. Pending generations are
  monotonic (lower generation is corrupt: preserve the marker, no spawn),
  except lower-generation plus strictly newer timestamp clears and
  re-records the marker as new work.
- `sync.rs` RPCs take `&mut self`; the retry macro expands inline so
  `self.client.<rpc>()` reborrows work. A closure-based generic retry
  helper MUST NOT be reintroduced.

## 7. Search, selector, and MCP invariants

- `snp get --query`, `snp list --filter`, and MCP `snippets_search` share
  `selector::searchable_text` (description plus command always, tags by
  default, output/notes only with `list --search-output` / `search_output`);
  folders/favorite/sync metadata/credentials are never searchable. MCP
  `snippet_get` takes ID (case-sensitive) / description / command
  (case-insensitive), exactly one required.
- MCP stays read-only, stdio-only, and non-executing. No mutations, no
  command execution, no long-running MCP service.
- Do not sanitize snippet commands by design. Removing CLI flags is
  breaking (deprecate first). `commands/mod.rs`
  (`load/save_snippets`, `run_snippet_selection`) and `ui/mod.rs`
  re-exports affect all TUI commands; clipboard side effects go through
  `copy_to_clipboard()` in `clip_cmd.rs`.

## 8. Transport and runtime policy

- `snp update` uses in-process `eggfetch-core =0.2.0`
  (`standard-http1` plus `redirects` plus `tls-rustls` plus
  `tls-native-roots` only; strict eggfetch redirects plus native
  `Timeout.total`; snip-it keeps the initial-HTTPS guard).
  `snip-sync update` keeps external `curl`: the fresh 0.2.0 lean-profile
  trial grew the server from 3,833,152 to 5,145,224 bytes (+34.23%), past
  the 10% gate. `tests/architecture.rs` pins the no-`curl`/lean-profile/
  delegated-timeout properties.
- `snip-sync` HTTP keeps the pre-bound listener, explicit body rejection,
  disabled total connection lifetime, typed EggServe shutdown completion,
  external TLS termination, Basic-auth constant-time comparison,
  configured CORS policy, and three security headers. Proven wire parity
  (locked in `tests/snip_sync_lifetime.rs`): router 404/405 are empty with
  no content-type (metrics-disabled 404 keeps `"Not found"`), known routes
  answer preflight/unsupported methods with `Allow: GET, HEAD`, every
  non-allow-all response carries `Vary: origin` (allow-all omits it), and
  preflight never carries the security headers.
- Tokio: the global `RUNTIME` exists only for async commands (`run`,
  `clip`, `search`, `sync`, `register`, `premade`, `update`); local-only
  commands never init it. `run_snippet_selection` takes
  `Option<&Runtime>` (`None` when `do_sync` is false). The detached worker
  uses `new_current_thread()`; keep the client's `rt-multi-thread`
  feature. Keep `keyring = "4"` default features (platform stores) or
  persistence silently falls back to the mock store.
- Argon2 parameter changes break all existing encrypted payloads (version
  first).

## 9. Distribution and release invariants

- Publish is manual local; no CI token/workflow publishes. Order is
  `snip-proto` then `snip-sync` then `snip-it`.
- `snip-it` and `snip-sync` have independent crate versions and independent
  exact tags (`vX.Y.Z` vs `snip-sync-vA.B.C`). Published releases are
  immutable; never retrofit binaries onto a historical release.
- One release workflow only; no matrix duplication. Integrity failure
  (checksum mismatch, wrong candidate version, malformed release data, TLS
  failure, GitHub 5xx) is a hard failure with no Cargo fallback. Never
  silently invoke `sudo`/elevation.
- Version bump plus `CHANGELOG.md` in one PR (`CONTRIBUTING.md`,
  `RELEASING.md`). Topic branches squash-merge to `main`; imperative mood,
  first line under 72 chars.

## 10. Verification and evidence rules

- `bash scripts/check.sh` is authoritative (same as Linux CI).
  `bash scripts/release-check.sh verify` is manual pre-release and requires
  a clean tree. `SNP_ALLOW_PLAINTEXT_API_KEY=true` on all test commands;
  never remove that seam. `set_var` in tests needs `unsafe` (edition 2024).
- Tests assert exact counts (not `>= 1`), prove server-side effects, and
  verify pending-clear ordering; helpers emit JSON-lines lifecycle events
  only when `SNP_TEST_EVENTS_DIR` is set.
- Integration/PTY/lock/barrier tests are serial (`--test-threads=1` for
  `pty_integration`, `*_concurrency`, `sync_multibatch`, `*_barriers`,
  `repair_transactions`). Deep crash/restore/manifest suites run only in
  `release-check.sh verify`, not CI.
- No milestone is complete merely because code landed. Completion requires
  the closure evidence defined by its implementation plan and subsystem
  roadmap.
