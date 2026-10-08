# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Documentation
- **Agent-facing docs re-verified against source**: Audited every `.skills/`
  file, `architecture/` deep-dive, and `docs/` contract against the current
  codebase, and corrected claims that no longer held. Notable corrections:
  sync encrypts only `{description, command, tags}` (metadata `id`,
  `created_at`, `updated_at`, `device_id`, `deleted` travel in plaintext) — the
  skills previously implied the whole `ProtoSnippet` was encrypted; snippet IDs
  are deterministic `legacy-<sha256 hex>`, not UUID v4; `snip-sync` defaults are
  `<config>/snip-sync/snippets.db`, 5 connections, and an absolute
  `premade-libraries` path; and the keychain seam is protected by a
  `#[cfg(feature = "test-support")]` gate rather than a guard test (a second
  seam, `SNP_TEST_CREDENTIAL_FILE`, was undocumented).
- **`docs/` pruned and indexed**: `SECURITY_AUDIT`, `FEATURE_BOUNDARIES`,
  `CANONICAL_OPERATIONS`, and `FUZZING_AND_PROPERTY_TESTS` moved to
  `docs/archive/` with headers stating exactly why each is stale — they
  referenced the pre-split `src/library.rs` / `src/config.rs` and pre-convention
  "Phase 06A"-style planning labels. New `docs/README.md` classifies every
  reference doc as current contract or archived snapshot.
- **`LOGICAL_LAYERS.md` aligned with its enforcer**: the document claimed
  "no file moves yet" while `tests/architecture.rs` enforces the post-split
  module layout. It now lists the exact enforced module sets, the two known
  cross-layer exceptions, and the `config/` exclusion.
- **Contracts corrected**: `JSON_SCHEMAS.md` (status/doctor envelopes,
  `restore` mode casing, dry-run shape), `LIBRARY_SCOPE.md` (`snp list
  --library all` does not exist; only the selector path understands `"all"`),
  `COMMAND_CONTRACTS.md` (exit-2 legend, `repair` exit 10, `edit --output`
  writes stderr, startup-recovery table), `IDENTITY_CONTRACT.md`, and
  `PUBLIC_API.md` (3 real rustdoc warnings, verified via `cargo doc`).
- **Two new skills**: `.skills/selector-and-search-parity.md` (the three
  surfaces sharing `searchable_text`, the readonly-vs-mutating resolver split,
  and the read-only stdio MCP boundary) and
  `.skills/persistence-and-toml-fidelity.md` (the "never post-process
  serialized TOML" rule, the 24-case golden corpus, durability classes, and
  fail-closed parsing) — both covering invariants that previously existed only
  in `AGENTS.md` prose or not at all.
- **`cargo doc` is warning-free**: fixed the three remaining `private_intra_doc_links`
  warnings (`execution_lock.rs` `process_alive`, `validate_cmd.rs` `run`,
  `selector.rs` `resolve_selector_readonly`) by converting links to `pub(crate)`
  items into plain code-span text. `docs/PUBLIC_API.md` records the rule: only
  items re-exported from `src/lib.rs` may be intra-doc-linked from a public doc
  comment.

### Fixed
- **Silent permanent data loss on the sync server.** `snippets.id` is a
  *global* primary key, so pushing a snippet whose id already exists under a
  different user or library wrote zero rows. SQLite reports the discarded
  `ON CONFLICT … DO UPDATE … WHERE` as a no-op with no error, and the handler
  counted it as `accepted`, replying `success: true` with empty `skipped_ids`.
  Identical content in two libraries of one install yields the same
  deterministic `legacy-<sha256>` id, so this was reachable by copying a
  snippet between libraries. `upsert_snippet_in_tx` now returns an
  `UpsertOutcome`; a cross-scope collision counts as rejected in `push` and
  appears in `skipped_ids` in `sync`. A genuine last-write-wins dedupe still
  reports as accepted.
- **Sync no longer exits 0 when records were dropped.** `skipped_count > 0`
  (client-side encryption failures, server-refused rows, or payloads that
  failed to decrypt) only incremented `conflicts`, so a run that silently lost
  snippets still reported success. Skips now count as `failed`, which returns
  `PartialSyncFailure`. `conflicts` counts only snippets genuinely overwritten
  by another device, and `pushed` now also increments for bidirectional sync
  (which uploads too). The client also folds the server's `skipped_ids` into
  its own accounting instead of discarding them.
- **Libraries could become permanently undownloadable.** Downloads were paged
  by row count only, while the 4 MiB gRPC encode ceiling is enforced on
  encoding, where tonic fails with `OUT_OF_RANGE` rather than truncating. A
  large page could be pushed successfully and then never downloaded by any
  client. Response pages are now trimmed to the encoded-byte budget (with a
  64 KiB envelope reserve, `skipped_ids` bytes, and always at least one row)
  and set `has_more`. Encrypted payloads are also bounded by a new
  `max_encrypted_payload_length` (default 1 MiB) instead of being unlimited,
  and `OutOfRange` is no longer retried.
- **A 0-byte `local-data.lock` hung every mutation forever.** The acquisition
  loop retried the empty-record arm without a bound, and a failed `write_all`
  (or a SIGKILL between `create_new` and `write_all`) leaves exactly that file
  behind. `save_library`, `snp new`, and `snp edit` then hung with no output
  and no timeout, and no in-code path could clear it. The arm now mirrors the
  transaction lock's ladder — bounded retries, then quarantine, then a
  deadline error — and a failed `write_all` deletes the file it just created.
- **Stale-lock reclaim could steal a live lock.** `quarantine_local_data_lock`
  renamed the lock file without re-checking that the record it had observed was
  still there, so a concurrent acquirer that reclaimed and re-created the lock
  first could have its live lock renamed away, admitting two writers into the
  critical section. The reclaim now verifies the record still matches before
  renaming.
- **A crashed transaction writer's partial record could be quarantined while
  still live.** The malformed arm quarantined immediately, even though the arm
  above it exists because a half-written record is observable. Malformed and
  partially written records now share the same bounded retry ladder before
  quarantine.
- **Corrupt auto-sync state reported as "nothing to do".** `preflight_check`
  returned `Err(String)` for three different conditions and the caller mapped
  all of them to `NothingToDo` (exit 0), so corruption never incremented
  `consecutive_failures`, never engaged backoff, and was invisible in
  `snp status`. It now returns a typed `PreflightError`; rollback and
  unreadable/corrupt markers go through `record_failure`. A pending-state read
  error now records a failure too, instead of exiting nonzero with no durable
  record and no backoff.
- **`snp premade list/get/search/update` panicked with an empty API key.** Those
  four subcommands checked only `enabled`, so a headerless legacy `sync.toml`
  (`api_key` is `#[serde(default)]`) reached `add_api_key_metadata`'s
  `debug_assert!`. They now require both `enabled` and a non-empty key, and
  every authenticated `SyncClient` method fails fast with a typed error naming
  `snp register --force` instead of sending an unauthenticated request.
- **An older server tombstone could regress a local snippet's `updated_at`.**
  `choose_version` returns `Remote` for (live, deleted) without comparing
  timestamps, so writing the tombstone's `updated_at` verbatim lowered the
  local value below a watermark other clients had already synced past. The
  remote-tombstone arm now uses `.max(..)`, matching the local-tombstone arm.
  Deletion still wins outright — no resurrection.
- **Progress counter could not reach its denominator.** Libraries completed by
  the recovery-marker path skipped the increment, so the last line printed was
  e.g. `[1/2]`.
- **Unit tests no longer mutate the real `~/.config/snp/`.** `save_library`
  derives its gate and local-data-lock paths from the config directory rather
  than the target library path, so library and `run_exact` unit tests wrote
  `audit.log`, `usage.toml` (counters accumulated across every run) and
  `.transaction/` into the developer's home directory, and contended on one
  process-wide lock file — which made
  `test_serialization_matrix_makefile_leading_tabs_with_trailing_newline` fail
  on a 30 s local-data-lock timeout. Tests now bind a per-test, thread-local
  config-directory override (`ScopedConfigDir`, `#[cfg(test)]` only), so unit
  tests are hermetic and parallel-safe again.
- **`/metrics` auth failures were invisible.** The HTTP leaf returned 401
  without incrementing `snip_sync_auth_failures_total`, so the metric could
  never observe brute force against a password-protected endpoint.
  `Basic` auth-scheme matching is now case-insensitive per RFC 9110 §11.1
  (`basic …` was rejected).
- `cargo fmt --check` failed on committed code (`tests/tui_requires_terminal.rs`),
  so `scripts/check.sh` aborted at its second step and Linux CI was red on
  `main`.
- **Interactive commands no longer abort without a terminal.** `snp select`,
  `run`, `clip`, and `search` entered the ratatui selector unconditionally;
  without a tty, `ratatui::init()` panicked and the process died with SIGABRT
  (exit 134) plus a Rust backtrace note. The selector now guards its single
  `ratatui::init()` call site and returns exit 1 with a message pointing at
  `snp get --query <text> --field command` and `snp list --json`. Covered by
  `tests/tui_requires_terminal.rs`; the interactive path is still covered by
  `tests/pty_integration.rs`.
- **A local deletion could be silently erased and then resurrected.** The
  merge retained a local record only when it was not a tombstone, so a
  tombstone missing from the server response was dropped from the library and
  persisted. The server response is a delta (`updated_at >= since`), so this is
  guaranteed in pull-only mode, where nothing is uploaded yet the merge still
  ran: the deletion never reached the server *and* the record of the intent was
  erased, so no later sync could propagate it. A local tombstone is now retained
  until the server acknowledges it (returns it with `deleted == true`), at which
  point the `Equivalent` arm retires the record. Covered by
  `test_unacknowledged_local_tombstone_is_preserved` and
  `test_acknowledged_local_tombstone_is_retired`.
- **Bidirectional sync reported skipped records as success.** `skipped_count >
  0` incremented `conflicts` and recorded the library as *succeeded*, so the run
  exited 0, the pending generation was cleared, and refused records were never
  retried. Bidirectional is the default direction, and the pull-only branch
  already handled the same condition correctly. Both branches now increment
  `failed`, record the library as failed, and never count skips as merge
  conflicts.
- **Missing-library recovery advanced `last_sync` and deleted its marker despite
  dropped records.** `success` does not imply a clean upload — the server
  reports refused rows in `skipped_ids` without flipping it — and a freshly
  re-created library returns no server rows, so a retry in which *every* record
  was refused still reported success. The partial failure was reported as
  "Re-linked and synced" with no durable trace. The watermark and the
  `<library>.sync_recovery` marker are now gated on `skipped_count == 0`.
- **Libraries skipped by the server pre-flight never incremented `failed`.** A
  failed `create_library`, a failed `link_server_library`, a missing library
  file, an unlinked library, and a fail-closed `load_library` all just
  `continue`d. None touched `status.failed`, so `run_sync_with_limits` returned
  `Ok(())`, exited 0, cleared the pending generation, and the library was never
  uploaded or retried. Each now records a failed result.
- **One refused record made a multi-batch upload a permanent library failure.**
  `push_snippets_batch` returned `Err` whenever the server reported
  `success: false`, which it sets for `rejected != 0`. The `?` then aborted the
  batch loop, so remaining batches were never sent, and the same record was
  re-sent on every subsequent sync. Per-record refusals (validation failure,
  cross-scope id collision) are now folded into `skipped_count` and the run
  continues, matching how the single-batch `Sync` transport already handled the
  identical condition.
- **`snp list` silently migrated a legacy checkout on disk.** It resolved its
  library through `get_library_path`, which calls the *mutating*
  `ensure_library_mode()`. On a config dir holding only `snippets.toml` with no
  `libraries/`, a plain `snp list` created `libraries/`, copied the legacy file,
  wrote `libraries.toml`, and bumped the generation — and `fs::copy` never
  removed the source, leaving `snippets.toml` as a second, now-inert copy that
  silently lost later edits. `list` now resolves through the canonical read-only
  `library::readonly_library_sources`, which performs no migration. This also
  restores the documented invariant that read-only paths never call the
  mutating resolver (`snp get --query` and MCP already wrote nothing).
- **`--library all` failed on `snp list`.** `get_library_path` did a plain
  `get_library_by_filename("all")` lookup, bypassing the single definition of
  the keyword in `LibraryScope::from_filter_arg`, so `list --library all` and
  `search --library all` both failed with a misleading "Library 'all' does not
  exist" while `get --library all` worked. `list` now resolves through the
  read-only path and supports the scope; `search` takes the single-library path
  (its selector edits and deletes inside one library file) and now rejects
  `--library all` with an error naming the supported alternatives instead of
  claiming the library does not exist.
- **A library could be named `all`.** `validate_library_name` had no reserved-name
  check, so `snp library create all` succeeded and then `--library all` meant
  two different things depending on the surface — and the wrong scope was
  returned silently on the resolver paths. `all` is now rejected, and
  `docs/LIBRARY_SCOPE.md` updated to match.
- **The transaction lock could hang forever on `PermissionDenied`.** The
  acquisition loop retried read failures with `sleep(1ms); continue` and no
  deadline, and the Windows-motivated `PermissionDenied` retry was applied
  unconditionally. On Unix that condition is stable, not transient (a lock file
  owned by another UID, or a `.transaction` directory that is not traversable),
  so the CLI hung with no error and no timeout — including `snp repair`,
  `snp restore`, and the mutation gate used by `save_library`. It now uses the
  same platform-gated `is_transient_lock_contention` helper as `local_data.rs`,
  plus a 30 s deadline as defense in depth.
- **A failed `write_all` left a 0-byte `transaction.lock`.** `create_new`
  succeeded, the write failed, and the `?` propagated with the empty file left on
  disk, so every later acquirer ran the empty-content ladder to exhaustion. The
  file is now removed before the error is reported, mirroring `local_data.rs`.
- **The gRPC rate limiter was keyed on the unvalidated API key.** Every
  authenticated RPC passed the raw, request-supplied key to the limiter as the
  bucket identity *before* `get_user_by_api_key` validated it, so every distinct
  attacker-chosen key got a fresh `rate_limit_per_minute` budget and the limiter
  imposed no aggregate request ceiling. It now keys on the peer address, reusing
  the trusted-proxy / `x-forwarded-for` handling already applied to `register`.
  (This is not an Argon2 amplification DoS: `db.rs` does an indexed
  `api_key_prefix` lookup first, so random keys never reach Argon2id.)
- **Read-only devices reported phantom overwrites.** `conflicting_ids` was
  computed from "server row has a foreign `device_id`" before the merge and never
  cross-checked against `choose_version`, so `"N snippets overwritten by another
  device"` and `status.conflicts` were emitted even when the local copy won or
  the content was byte-identical. The conflict list is now derived from the merge
  — only ids where `choose_version` returned `Remote` — and the superseded
  `sync::detect_device_conflict` was removed.
- **`/metrics` returned 401 without `WWW-Authenticate`.** RFC 9110 §11.1 requires
  the challenge header; without it a password-protected `/metrics` was
  unreachable through standard HTTP auth flows. The header is now sent on the 401
  and asserted in `tests/snip_sync_lifetime.rs`.
- **`PRAGMA foreign_keys=ON` was applied to one pooled connection.**
  `foreign_keys` is a per-connection SQLite setting (unlike `journal_mode`,
  which lives in the file header), so with no `after_connect` hook it was active
  on at most 1 of 5 connections, leaving the schema's declared FKs unenforced on
  the rest. No current exploitable consequence — there are no hard `DELETE`s and
  every query is scoped by `user_id` — but the pragma now moves to
  `SqlitePoolOptions::after_connect`.
- **Certificate regeneration could leave the directory with neither key nor
  cert.** `generate_dev_certs` deleted the old pair first and, when the second
  rename failed, explicitly removed the newly installed key — contradicting the
  comment promising to keep working material available. The old pair is now
  staged as sibling backups and restored on any failure.
- **`IntegrityMismatch` reported stored and computed values swapped.** The
  constructor was called with the stored value as `expected` and the recomputed
  one as `got`, so the diagnostic pointed an operator debugging marker corruption
  at the wrong value.
- **"gRPC server listening" was logged after the server had stopped.** The
  `tracing::info!` sat after the `.await` on `serve_with_incoming_shutdown`, so
  it fired once the gRPC server had terminated, duplicating the correct bind-time
  log on every run.
- **`tests/pty_integration.rs` used fixed sleeps and failed under host load.** The
  helper slept a hard-coded delay before writing keys, assuming the TUI had
  installed its input reader. Under load the keys were written before anyone read
  them and the TUI waited forever until the fixed 10 s exit timeout — the only
  failure in the full serial suite, and it passed 3/3 in isolation. The helper now
  waits for the TUI to render and go quiet before sending keys, and the exit
  timeout is a named 30 s constant.

- `snp --help` listed exit codes 0–9; exit 10 (`UNSAFE_REPAIRS`) is now listed.
- `src/utils/config.rs` module docs claimed per-platform config resolution
  (AppData on Windows, Application Support on macOS). There is no `#[cfg]`
  branch: every platform uses `$XDG_CONFIG_HOME`/`~/.config` plus `/snp`, with
  a one-way macOS legacy migration.

### Documentation
- **README restructured around the quickstart**, cut from 348 to 233 lines. It
  now opens with install and a runnable 60-second example, keeps a full command
  table, and defers detail to `USER_GUIDE.md`, `docs/`, and
  `snip-sync/README.md` instead of duplicating it.
- **Wrong command spelling fixed in three user-facing files.** `--push-only`,
  `--pull-only`, and `--dry-run` are flags of the `sync run` subcommand, so the
  documented `snp sync --push-only` failed with `unexpected argument found`.
  Corrected to `snp sync run --push-only` in `README.md`, `USER_GUIDE.md`, and
  `snip-sync/README.md`. Note that `SyncCommands::Run` is documented as the
  default when no subcommand is given, and bare `snp sync` does work, but the
  flags themselves are only accepted after the explicit `run` word.
- **Keychain guidance corrected in the user docs.** `USER_GUIDE.md` described
  `SNP_ALLOW_PLAINTEXT_API_KEY=true` as permitting plaintext storage "when
  keychain storage fails" and recommended it for headless provisioning. There is
  no such fallback: a release build refuses the save outright, and the env var
  is compiled in only under `test-support`, so it is ignored entirely.
- `README.md` config table no longer implies every file exists on a fresh
  install (a new install creates only `snippets.toml` and `logs/`) and now
  documents `logs/`.

### Fixed
- **snip-sync HTTP parity (server-lifecycle-http M004)**: Restored the pre-migration wire
  contract proven by exercising the Axum server over real sockets. Router
  404/405 responses are empty with no content-type (the metrics-disabled 404
  keeps its `"Not found"` payload), known routes answer preflight and
  unsupported methods with `Allow: GET, HEAD`, every non-allow-all response
  carries `Vary: origin`, and preflight responses carry CORS metadata without
  the ordinary security headers. Socket regression tests lock the contract;
  the release binary is byte-identical at 3,898,408 bytes.

### Changed
- **snip-sync HTTP runtime (server-lifecycle-http M003)**: Replaced the Axum/Tower-HTTP
  surface with one direct EggServe HTTP/1 health/metrics service. Tonic remains
  on its separate listener and TLS remains external. The controlled release
  build measured 3,898,408 bytes, 1.70% above the original Axum runtime and
  1.65% smaller than the EggServe/Axum adapter trial.
- **Self-update transport (updater-transport M005; supersedes M004)**:
  `snp update` now pins `eggfetch-core` 0.2.0 on the unchanged lean
  `standard-http1` + `redirects` + Rustls native-roots profile. Strict
  bounded redirects, the native request/body `Timeout.total`, 1 MiB metadata /
  256 MiB streamed-binary bounds, and 404-only Cargo fallback remain
  unchanged. A fresh same-host release build stayed byte-identical at
  6,776,320 bytes. The `snip-sync` updater remains on external `curl`:
  its fresh 0.2.0 lean-profile trial grew the server binary 34.23%, past the
  10% material-growth gate.

## [1.3.9] - 2026-09-16

### Changed
- **Argon2 0.5 → 0.6** (`snip-it` 1.3.9, `snip-sync` 0.1.6):
  password-hash 0.6 breaking changes adapted — server uses
  `hash_password_with_salt` with raw bytes and `phc::PasswordHash`,
  client KDF uses `hash_password_into` directly. Stored PHC hashes and
  encrypted payloads verify byte-identically across the upgrade.

### Fixed
- **Pipe-to-shell bootstrap**: `curl .../packaging/install.sh | bash` no
  longer fails with `BASH_SOURCE[0]: unbound variable` under `set -u`; the
  entry-point guard treats an empty `BASH_SOURCE` as direct execution.
- **macOS bootstrap checksum compare**: `packaging/install.sh` no longer uses
  `${var,,}` expansion (unsupported by the system Bash 3.2 on macOS);
  SHA-256 comparison uses portable `tr`-based case normalization.

## [1.3.8] - 2026-09-16

### Fixed
- **Bootstrap installer verification fails closed**: `packaging/install.sh`
  now explicitly returns a hard failure when checksum/identity verification
  rejects a downloaded candidate instead of relying on `set -e` propagation.
  Integrity/identity failures still never trigger Cargo fallback.
- **PowerShell installer testability**: `packaging/install.ps1` exposes its
  target/asset/source-only/version helpers as pure functions and skips the
  install entry point when dot-sourced, so the new
  `scripts/tests/installers.ps1` contract suite can verify mapping, checksum,
  identity, fallback-boundary, destination, and `Both` ambiguity behavior
  without performing an install. `Get-CargoCandidate` now resolves its binary
  name explicitly.
- **Delta-sync watermark off-by-one** (snip-sync): incremental queries now use
  `updated_at >= since` so a snippet written in the same second as the previous
  sync's watermark is no longer skipped by every later incremental sync.
- **Flaky cross-process lock test**: the poller waits for non-empty outcome
  content and the helper publishes outcomes via temp+rename, eliminating a
  create/truncate write race observed on APFS/ext4.
- **`snp edit --output --filter` ambiguity**: substring filters that match
  multiple snippets now fail with a candidate list instead of silently editing
  the first fuzzy match's local-only output field.
- **Batch-context error flattening**: `add_batch_context` no longer re-wraps
  non-`SyncFailure` errors (e.g. gRPC `internal` → `Runtime`) as transient
  `SyncRequestFailed`; persistent server faults now escalate to
  attention-required instead of retrying indefinitely.
- **Plaintext-mode credential seam**: `SNP_ALLOW_PLAINTEXT_API_KEY=true` with an
  `@keychain` marker in `sync.toml` now fails fast instead of authenticating
  with the literal marker string as the API key.
- **Schema-version parsing fails closed**: out-of-range `schema_version`
  integers (negative or > u32::MAX) produce an error instead of wrapping to a
  wrong-but-valid version via `as u32`.
- **Durability surfacing in `write_private_atomic`**: a failed parent-directory
  fsync after rename is now an error instead of being discarded.
- **Lock-quarantine growth**: stale/malformed lock quarantine files older than
  7 days are garbage-collected opportunistically when a new quarantine file is
  created.
- **Status snapshot fallback**: a partially written auto-sync status file is
  reported conservatively (retry scheduled) instead of `Succeeded`.
- **Debounce generation-reset race**: a pending marker cleared by an explicit
  sync and immediately re-recorded at generation 1 (fresh creation timestamp)
  is adopted by the debouncing worker instead of recording spurious
  internal-failure telemetry.

### Changed
- `UsageIndex::prune()` uses a `HashSet` for O(n+m) pruning.
- Manual sync pagination enforces a hard page bound (10,000 pages) against
  misbehaving servers that never clear `has_more`.

## [1.3.7] - 2026-08-18

### Changed
- **Auto-sync correctness closure (Phase 01)**
  - Worker `NothingToDo` no longer clears the pending marker — pending is
    preserved for the next cycle, recovery, or manual sync (Phase 01
    invariant).
  - Disabled-policy worker exits with `NothingToDo` before touching pending
    state, avoiding pointless cycle loops.
  - Executor subprocess documentation clarified: it does NOT acquire the
    `SyncExecutionLock` (worker-owned for the cycle); it only invokes the
    canonical `crate::sync_commands::run_sync`.
  - Architecture deep-dives updated to reflect the truthful invariant
    (worker-owned lock, executor-doesn't-reacquire).
  - New regression test file `tests/auto_sync_closure.rs` (15 tests)
    covering real-server end-to-end, negative paths, lock ownership,
    direction parity, and worker outcome invariants.

### Added
- **Detached one-shot auto-sync worker (Release 5D corrective)**
  - Replaced in-process debounce coordinator with a hidden `auto-sync-worker` subcommand re-execed by the parent. The worker is fully detached via `setsid` on Unix and `DETACHED_PROCESS | CREATE_NO_WINDOW` on Windows, with `stdin`/`stdout`/`stderr` routed to `null`. The parent returns immediately after spawning — no in-process latency for the user.
  - Restructured `src/auto_sync.rs` into a directory module under `src/auto_sync/` with focused submodules: `policy.rs`, `pending.rs`, `lock.rs`, `spawn.rs`, `worker.rs`, `notification.rs`, `mod.rs`.
  - New pending marker schema (v2) with monotonic `generation`, CRC32 integrity, conditional clear keyed on observed generation. v1 markers migrate transparently on load.
  - New worker lock format (`auto-sync-worker.lock`) with `pid`, `started_at_unix_ms`, `nonce` fields. Stale detection via `kill -0 pid` plus 5-minute age threshold.
  - Hidden `Commands::AutoSyncWorker { state_dir, nonce }` registered with `hide = true`. Exits with internal exit code 0; outcome is logged, not propagated.
  - Added `libc = "0.2"` (Unix-only) for `setsid`.
  - `startup_recover_pending()` runs at startup for non-worker subcommands, re-spawning the worker if recent pending state is found or clearing markers older than 5 minutes.
  - `snp doctor --compatibility` rewritten to use the new `paths::{state_dir, pending_marker, worker_lock}` helpers and to surface lock liveness via `lock::process_alive`.
  - Security guarantees: no command payloads, credentials, or encryption material ever appear in worker argv, env, pending markers, lock files, or `auto-sync-worker.<nonce>.done` sentinels. All artifacts written with `0o600` permissions on Unix.
  - Architecture deep-dive updated (`architecture/auto_sync.md`): new trigger matrix, detached-worker data flow, schema v2 format, worker lifecycle, design rationale for replacing the in-process coordinator.
- **Auto-sync integration hardening and closure (Release 5D)**
  - Created `architecture/auto_sync.md` deep-dive documenting the canonical data flow, trigger matrix, debounce state machine, durable pending state, cross-process locking, retry/backoff, failure policy, and safety invariants.
  - Updated `architecture/overview.md` to include auto-sync in the sync infrastructure section and deep-dives table.
  - Updated `docs/PET_COMPATIBILITY.md`: Release 5 auto-sync status changed from "Planned" to "Implemented".
  - Updated `docs/CLI_EXITCODE_STREAM_POLICY.md`: documented auto-sync error exit code (post-commit nonzero exit when `auto_sync_failure = "error"`).
  - Reconciled trigger matrix across implementation, tests, and documentation (12 command types).
  - Architecture reconciliation: all mutation commands route through central `notify_mutation()` — no ad-hoc auto-sync logic outside the coordinator.
  - Security audit: no command payloads, credentials, or encryption material in lock files, pending markers, or status files.
  - Documentation reconciled: README, USER_GUIDE, AGENTS.md, CHANGELOG, PET_COMPATIBILITY, architecture docs all aligned with shipped behavior.

### Changed
- **Release 2 final serialization corrective — exact TOML round-trips for tabs, trailing spaces, and CRLF**
  - Removed `quote_strings_containing_backslashes` from the save pipeline (`save_library`, `save_snippets`, `save_config`, `save_sync_settings`). The helper silently corrupted tabs, trailing whitespace, and CRLF: its regex could not distinguish triple-quoted multi-line strings from ordinary double-quoted strings, and its single-quoted output preserved TOML escape sequences like `\t` as literal two-character pairs. The `toml::to_string_pretty` serializer already picks the correct quoting and escapes for every character.
  - Rewrote `fix_invalid_toml_escapes` as a hand-written TOML token scanner that correctly recognizes triple-quoted strings (`"""..."""` and `'''...'''`), single-quoted literal strings, line/block comments, and single-line basic strings. The previous regex-based implementation greedily consumed triple-quoted delimiters, corrupting multi-line TOML content.
  - **Release 2 closure pass — secure editor tempfiles, editor command parsing, and unified exact-source validation**
  - Editor temporary files are created atomically via `tempfile::Builder` in the OS temp directory with `0600` permissions and RAII cleanup. The previous hand-rolled PID/timestamp filename generation is gone.
  - `snp new --editor` now prefers `$VISUAL` over `$EDITOR`. The editor command specification is parsed with `shell-words`, so values like `code --wait`, `nvim -f`, or quoted paths containing spaces work without invoking a shell.
  - `snp new --from-file` now follows symlinks and validates the resolved target is a regular file. Broken symlinks, directories, FIFOs, sockets, and device nodes are rejected.
  - Stdin, file, and editor sources share a single `validate_exact_command_bytes` validator: 16 MiB cap, valid UTF-8, no NUL bytes, no empty/whitespace-only content.
  - Bash `snp_new_previous` now defensively checks for the `fc -ln` formatter prefix (`\t `) before stripping it; legitimate leading whitespace in the captured command is preserved.
  - Golden command corpus expanded from 15 to 24 entries including tabs, trailing spaces, CRLF, mixed newlines, and combinations with quotes and backslashes.

### Added
- Sort and ranking system (`--sort` and `--favorites-first` flags) for run, clip, search, select, and list commands
  - Sort modes: relevance (default), recent, last-used, most-used, description, command
  - `--favorites-first` groups favorited snippets before others
- Local-only usage tracking: records use count and last-used timestamp on successful run/clip
- Usage metadata stored in `~/.config/snp/usage.toml` (atomic writes, fail-open on corruption)
- Shared sort model in `src/sort.rs` with deterministic tie-break chain
- TUI sort indicators for all modes including `[used]` and `[freq]`
- Integration tests for sort flags, favorites-first, and CSV/JSON sort output
- **Output / notes presentation (Release 4B)**
  - Shared output presentation model (`src/output.rs`) with `OutputPresentation` type: safe terminal rendering, summary truncation, multiline bounding, ANSI/OSC sanitization, and fuzzy-search scoring budget.
  - TUI preview panel shows output below command with `--- Output / Notes ---` separator when present.
  - `snp edit --output <text>`, `--output-stdin`, `--clear-output` for structured output editing (requires `--filter`).
  - `snp list --search-output` includes the output field in fuzzy search matching.
  - Default `list` output hides empty output fields to reduce noise.
  - JSON and CSV output always include the raw `output` field exactly as stored.
  - `select`, `run`, and `clip` continue to act on `command` only; output is never emitted or executed.
  - Output content is treated as untrusted text: no eval, no shell execution, no ANSI interpretation during display.
  - 36 new tests covering JSON/CSV preservation, edit set/clear/stdin, search-output flag, multiline roundtrip, tab/special char roundtrip, ANSI preservation, conflict flags, no-eval security, and help text.
- **Explicit pet import command (Release 3B)**
  - `snp import pet <path>` creates a native named library from a pet TOML file. Source files are never modified.
  - Options: `--library <name>`, `--merge`, `--replace`, `--dry-run`, `--strict`, `--report human|json`, `--report-file <path>`.
  - Atomic writes via temp-file-and-rename; existing libraries are backed up before merge/replace.
  - Duplicate detection: exact duplicates (same command + description) are skipped during merge; semantic warnings for same-command-different-description and same-description-different-command.
  - Diagnostics: unknown TOML fields, missing description, empty command, choice variables, output fields preserved.
  - Human-readable report to stderr; JSON report to stdout; `--report-file` for persistent JSON output.
  - Library name derived from source filename when `--library` is omitted.
  - 35 new tests: 20 integration tests (default create, explicit name, collision, merge, dry-run, source untouched, JSON report, error cases, strict/permissive, replace, command preservation, choice variables, mixed aliases, help, flag conflicts) and 15 unit tests (name derivation, duplicate detection, TOML parsing, entry conversion).
- **Compatibility diagnostics (Release 3C)**
  - `snp doctor --pet-file <path>` performs read-only analysis of pet snippet files: TOML parse status, unknown fields, missing required fields, empty commands, choice variables, duplicates, output fields, normalization preview, and recommended import command.
  - `snp doctor --compatibility` audits the installed snp environment: binary version, config directory, library directory, primary library, sync config, shell availability, shell init syntax validation (bash -n/zsh -n/fish --no-execute), editor configuration ($EDITOR/$VISUAL), legacy paths, Release 1 select availability, Release 2 acquisition flags, and Release 3 choice-variable parser.
  - `snp doctor --library <name>` analyzes a specific library file using the same analysis as --pet-file.
  - `snp doctor --check-shell <bash|zsh|fish>` validates `snp shell init` output syntax for the specified shell.
  - Shared diagnostic model (`src/diagnostics.rs`) with `SourceSpan` type for byte-offset source positions, used by both import and doctor: `CompatibilityDiagnostic`, `DoctorReport`, `PetImportReport` with stable machine-readable codes (E-/W-/I- prefix convention).
  - Options: `--strict` (treat warnings as errors), `--report human|json` (output format).
  - Exit codes: 0 (no errors), 1 (operational failure), 2 (error diagnostics found).
  - Human-readable report to stderr; JSON report to stdout (same stream convention as `snp import`).
  - Doctor never mutates source, destination, config, or library state.
  - 29 integration tests and 18 unit tests covering file analysis, JSON output, compatibility audit, strict mode, non-mutation, command execution prevention, variable expansion prevention, API key leakage prevention, config preservation, and import/doctor consistency.
- **Auto-sync mutation trigger integration (Release 5C)**
  - Central mutation notification API: `notify_mutation(kind, origin)` and `notify_local_mutation(policy, context)`.
  - All syncable mutation commands now trigger auto-sync after successful local commit: `snp new` (all sources), `snp edit` (editor), TUI delete, `snp import pet` (once per import), `snp library create/delete`.
  - Output-only edits (`snp edit --output/--clear-output`) do NOT trigger sync (output is local-only).
  - Explicit sync (`--sync` flag, `snp sync`) clears pending auto-sync state to prevent duplicate delayed sync.
  - Sync-origin writes (`MutationOrigin::SyncMerge`) never trigger auto-sync (prevents feedback loops).
  - `run_auto_sync()` creates its own Tokio runtime internally — callers don't need to pass one.
  - 10 new unit tests covering notification API: disabled policy, sync-merge suppression, user/import origins, all mutation kinds, AccountConfig, library ID, clear-after-explicit-sync, result Debug/PartialEq, MutationContext construction.
  - 12 new integration tests covering auto-sync trigger behavior: pending marker creation, disabled policy, stdin/file creation triggers, output-only edit exclusion, library create/delete triggers, import dry-run exclusion, import success trigger, failed sync local preservation, and explicit sync interaction.
- **Pet multiple-choice variable compatibility (Release 3A)**
  - Variable parser recognizes Pet `<name=|_opt1_||_opt2_||_opt3_||>` syntax and parses it into `VariableKind::Choices`.
  - TUI variable prompt renders choice variables as a navigable list selector (arrow keys / j/k).
  - `expand_command` expands choice variables with the selected value, just like required variables.
  - Raw command text is preserved in storage — choices are only expanded during interactive prompting.
  - Parser diagnostics warn on malformed choice syntax and duplicate variable names. Diagnostics now include machine-readable `code` and optional `suggested_fix` fields.
  - Repeated variables with the same name are deduplicated in the prompt — the user is prompted once and the value is reused for all occurrences.
  - Non-interactive fallback: `prompt_variables` returns an error when no controlling terminal is available instead of panicking.
  - Fuzz tests (500 iterations) verify the parser, expansion, and choice extraction never panic on arbitrary input. One subtraction-with-overflow bug in `extract_choices` was found and fixed.
  - 65+ new unit and integration tests covering choice parsing, prompting, expansion, serialization roundtrips, PTY end-to-end selection/default/cancel/dedup/restore, and edge cases.
- New unit tests in `src/commands/new_cmd.rs`: stdin rejection of empty/whitespace input, oversize-input rejection via the shared validator, symlink-following and broken-symlink behavior, FIFO/character-device rejection, `shell-words` parsing of editor specs, and ten multiline-prompt tests.
- New integration tests in `tests/integration.rs`: editor-source golden corpus round-trip, multiline terminator limitation, exact select-storage round-trip, backup-preserves-command, sync round-trip preservation, and run-storage plumbing.
- New Bash behavioral tests: `snp_new_previous` preserves leading tabs and quoted/backslash content.

## [1.3.1] - 2026-07-09

### Changed
- Bump `toml` 0.8 → 1.1 (unifies with snip-sync's toml)
- Bump `signal-hook` 0.3 → 0.4
- Bump `clipboard-win` 4.5 → 5
- Bump `prometheus` 0.13 → 0.14 (snip-sync)
- Relax `time` constraint to `<0.4`
- Bump GitHub Actions: `action-gh-release` v2→v3, `docker/login-action` v3→v4, `docker/build-push-action` v5→v7

## [1.3.0] - 2026-07-09

### Fixed
- Preserve the real in-memory API key when migrating legacy plaintext
  `sync.toml` values into the OS keychain; the saved config gets the
  `@keychain` marker, but the current operation continues with the
  actual credential.
- Verify `sync.toml` integrity checks against the exact saved TOML body,
  preserving trailing newlines and later user-authored comments.
- **Harden crates.io release.** Remove `themes/` from published package (embeds default theme directly in generated Rust source). Shrink public API surface to 9 modules. Replace `getrandom` jitter with `SystemTime::now().subsec_nanos()`. Split `get_config_dir()` into pure getter + `ensure_config_dir()` — fixes macOS legacy migration (could never run because `get_config_dir()` eagerly created the new directory). Harden `list_libraries` pagination loop against buggy servers. Use `fs::rename` instead of `fs::copy` for atomic backup restore.
- Move `subtle` to `[dev-dependencies]` (only used in tests).

### Changed
- Align release-facing documentation with the current Rust 1.94 MSRV and
  gRPC authorization metadata behavior.

### Removed
- Remove inert --non-interactive sync flag

### Added
- **Halloy theme support.** `snp` now ships 50 themes adapted from [Halloy](https://themes.halloy.chat). Press `e` in normal mode to open the theme picker; use `j`/`k` (or arrow keys) to preview themes live, `i` to filter, and `Enter` to save. Bundled themes are extracted to `~/.config/snp/themes/` on first launch; the active theme is persisted to `~/.config/snp/themes.toml`. The `SNP_THEME` env var is still honored for backward compatibility.
- **Build pipeline for bundled themes.** `scripts/build_themes.py` LZMA-compresses and base64-encodes every `.toml` under `themes/`, emitting `src/ui/_generated_bundled_themes.rs`. The build hook in `build.rs` re-invokes the script when the source themes are newer than the generated file, keeping the binary lean.
- New dependency: `lzma-rs = "0.3"` (pure-Rust LZMA decoder; no C toolchain required).

## [1.2.0] - 2026-06-05

### Added
- `rust-toolchain.toml` pinning Rust 1.88 with required components (`rustfmt`, `clippy`, `llvm-tools-preview`).
- `assets/demo.tape` for regenerating the README demo GIF with [vhs](https://github.com/charmbracelet/vhs).
- `.github/workflows/link-check.yml` running `lychee` weekly and on PRs.
- `.github/ISSUE_TEMPLATE/security.md` redirecting security reports to email.
- Dependabot grouping for minor/patch updates to reduce PR noise.
- `repo-hygiene` CI job that fails if any `.DS_Store`/`Thumbs.db`/`desktop.ini` is tracked.
- `msrv` CI job now exercises all three crates (snp, snip-sync, snip-proto).

### Changed
- README rewritten for end-user audience: lead with tagline, demo, install matrix, security callout.
- USER_GUIDE.md table of contents now complete; added "Migrating from pet", "Reset and Recovery", "Keychain Issues" subsections.
- SECURITY.md expanded: threat model, key derivation parameters, known API-key-in-body limitation, server deployment checklist.
- CONTRIBUTING.md expanded: full release process, MSRV policy, branching rules, dependency list.
- crates.io metadata: added `homepage`, expanded `keywords` and `categories`, set `documentation = "https://docs.rs/snp"`, added author email.
- `docs.rs` config: added `rustdoc-args = ["--cfg", "docsrs"]`; explicit target list (now includes `aarch64-unknown-linux-gnu` and `aarch64-apple-darwin`).
- `dependencies` audit: removed unused crates; pruned feature flags.

## [1.1.0] - 2026-06-05

### Changed
- Bump MSRV to 1.88 and edition to 2024
- Mark snip-proto and snip-sync as non-publishable (`publish = false`)
- Remove unused dependencies (prost, rustls-native-certs, rand from root crate; tower, hyper, http, async-trait from snip-sync)
- Simplify uuid features to v4-only (removed unused v7)
- Default server URL changed from http:// to https://
- CI MSRV check updated to Rust 1.88
- CI server-test job now runs `cargo test -p snip-sync` instead of `cargo test` from snip-sync directory

### Added
- Configurable sync network timeouts via `SNP_SYNC_CONNECT_TIMEOUT` and `SNP_SYNC_REQUEST_TIMEOUT` environment variables
- Theme-aware syntax highlighting colors (string and escape colors adapt to dark/bright theme)
- TUI draw errors are now logged instead of silently discarded
- Mouse capture disable failure is now logged
- Doc comments on SnipError constructors, SnippetData, ProcessResult
- Config corruption now creates a backup before returning defaults

### Fixed
- TUI: pressing `/` in insert mode now correctly clears the filter (was desyncing display from actual matching)
- TUI: cursor position overflow protection for very long input text
- TUI: scrollbar thumb and variable prompt now use theme colors instead of hardcoded Cyan/Yellow
- Sync: `sync_cmd::run()` now returns errors instead of silently swallowing them with exit code 0
- Sync: `merge_and_save` now returns `SnipResult` instead of `Result<_, String>`
- Sync: pull-only sync now checks for failures before advancing `last_sync` timestamp
- Library: `delete_library` now saves config before deleting file (atomicity improvement)
- Signal handlers now log errors gracefully instead of panicking with `expect()`
- Removed `.github/.DS_Store` from tracking, added recursive .DS_Store to .gitignore

## [1.0.0] - 2026-06-04

### Added
- Terminal UI (TUI) with fuzzy search, syntax highlighting, and visual multi-select mode
- Variable expansion system with `<name=default>` syntax for dynamic snippet parameters
- Cross-platform clipboard integration (macOS, Linux, Windows)
- End-to-end encrypted cloud sync via gRPC (AES-256-GCM + Argon2id key derivation)
- Multiple snippet libraries with primary library support
- Premade community snippet library downloads
- Automated periodic sync via cron integration
- Shell keyword expansion (`$HOME`, `~`, `$(date)`, `$PWD`, `$RANDOM`)
- Audit logging for snippet operations
- Dark and bright theme support
- Command execution with configurable timeouts
- Snippet import/export
- OS keychain integration for API key storage
- Snip-sync server with SQLite storage, rate limiting, and Prometheus metrics

### Security
- AES-256-GCM authenticated encryption for sync data
- Argon2id key derivation with OWASP-recommended parameters
- API keys stored in OS keychain by default (plaintext fallback requires explicit opt-in)
- Server-side API key hashing with Argon2id
- TLS enforcement for all sync connections
- Path traversal protection for premade libraries
- Parameterized SQL queries throughout
- No unsafe code in the codebase

### Fixed
- Encryption key cleanup now uses `std::mem::take` for proper zeroization
- Clipboard auto-clear failures log at warn level instead of debug
- Visual mode copy now copies commands (not descriptions)
- Sync merge uses `>=` for timestamp comparison (server wins on ties)
- Push-only sync counter increments regardless of failures
- Premade library TOCTOU race condition resolved
- Health check RPC verifies database connectivity
- Deleted snippets filtered from TUI display
- Sync error propagation to callers
- Premade sync returns error on failure

[1.3.9]: https://github.com/eggstack/snip-it/releases/tag/v1.3.9
[1.3.8]: https://github.com/eggstack/snip-it/releases/tag/v1.3.8
[1.3.7]: https://github.com/eggstack/snip-it/releases/tag/v1.3.7
[1.3.6]: https://github.com/eggstack/snip-it/releases/tag/v1.3.6
[1.3.1]: https://github.com/eggstack/snip-it/releases/tag/v1.3.1
[1.3.0]: https://github.com/eggstack/snip-it/releases/tag/v1.3.0
[1.2.0]: https://github.com/eggstack/snip-it/releases/tag/v1.2.0
[1.1.0]: https://github.com/eggstack/snip-it/releases/tag/v1.1.0
[1.0.0]: https://github.com/eggstack/snip-it/releases/tag/v1.0.0
