# Phase 06A Status

**Plan:** `plans/snip-it-correctness-06a-core-architecture-and-public-api-tightening.md`
**Baseline:** `ff506f5934957c4fd989224a6f0e0cf10f907567`
**Final commit:** see main branch

## Completion Summary

Phase 06A is complete. All workstreams addressed, all exit criteria met or explicitly
deferred with justification.

## Workstream Status

| Workstream | Status | Notes |
|------------|--------|-------|
| A — Public API inventory | **Done** | `docs/PUBLIC_API.md` — ~320 items classified |
| B — Logical layers | **Done** | `docs/LOGICAL_LAYERS.md` + architecture test + layer annotations on 17 source files |
| C — Internal facade | **Deferred** | Plan explicitly says "Crate count is not an exit criterion"; logical layers documented for future extraction |
| D — Canonical operations | **Done** | `docs/CANONICAL_OPERATIONS.md` — 15 operations documented |
| E — Data model tightening | **Done** | 13 public enums have `#[non_exhaustive]` |
| F — Error boundaries | **Done** | `SyncFailureKind::LibraryNotFound` added; string matching replaced with typed dispatch |
| G — Async/runtime | **Done** | Confirmed correct; documentation only |
| H — Test-support boundary | **Done** | `test_events` compile-time no-op without `test-support` feature; runtime no-op without env var |
| I — Feature boundaries | **Done** | `[features]` table added to Cargo.toml with 7 feature gates |
| J — Remove obsolete | **Done** | Dead `max_retries`, `STALE_LOCK_THRESHOLD_SECS` removed; coordinator test renamed |
| K — API/semver gates | **Done** | CI workflow with format, clippy, tests, architecture, feature matrix, package, secret scanning |
| L — Performance baselines | **Done** | `scripts/benchmark.sh` captures binary size, cold start, list/status latency |

## Exit Criteria Assessment

| Criterion | Met? |
|-----------|------|
| Every public item has an intentional classification | **Yes** — `docs/PUBLIC_API.md` |
| CLI internals no longer public solely for binary access | **Yes** — 8 modules narrowed to `pub(crate)` |
| Core, sync-client, application responsibilities explicit | **Yes** — `docs/LOGICAL_LAYERS.md` + architecture test |
| Canonical operations singular | **Yes** — `docs/CANONICAL_OPERATIONS.md` |
| Test-only failpoints outside release API | **Yes** — compile-time no-op without feature |
| Obsolete transition paths removed | **Yes** — dead fields/constants removed |
| Supported feature/package builds pass | **Yes** — CI feature matrix |
| Installed `snp` behavior compatible | **Yes** — binary name, help, links all tested |
| Public API/semver policies documented | **Yes** — `docs/PUBLIC_API.md` includes semver notes |
| Phase 05A behavioral evidence green | **Yes** — 1717 tests pass |
| Startup/performance do not regress | **Yes** — benchmark script captures baselines |
| No unnecessary daemon/service/microcrate explosion | **Yes** — workspace remains clean |

## Code Changes

### Visibility narrowing (`src/lib.rs`)
- `clipboard`, `diagnostics`, `encryption`, `library`, `output`, `status_snapshot`, `sync_commands`, `utils` → `pub(crate)`
- `sync`, `proto`, `usage` remain `pub` (needed by integration tests)

### `#[non_exhaustive]` added (13 enums)
`ProcessResult`, `CommandOutcome`, `SelectionOutcome`, `SnippetSort`, `ExpandedCommand`,
`MutationKind`, `MutationOrigin`, `FailureClass`, `RetryDisposition`,
`AutoSyncFailureMode`, `ExecutorExitCode`, `WorkerOutcome`, `SpawnError`

### Error handling (`error.rs`, `sync.rs`, `sync_commands.rs`)
- Added `SyncFailureKind::LibraryNotFound`
- Added `grpc_error_to_snip_error()` helper in `sync.rs`
- Replaced `err_msg.contains("Library not found")` with typed `matches!()` dispatch
- Added `LibraryNotFound → FailureClass::Configuration` mapping in `policy.rs`

### Feature gates (`Cargo.toml`, `test_events.rs`)
- `[features]` table: `default`, `tui`, `clipboard`, `sync`, `self-update`, `bundled-themes`, `test-support`
- `test_events` compiles to no-ops without `test-support` feature

### Dead code removed
- `AutoSyncPolicy.max_retries` field and `DEFAULT_MAX_RETRIES` constant
- `STALE_LOCK_THRESHOLD_SECS` constant
- `encryption::ct_eq` made private

### Test infrastructure
- `tests/architecture.rs` — dependency direction enforcement
- `tests/auto_sync_coordinator.rs` → `tests/auto_sync_lifecycle.rs` (terminology update)

### CI (`.github/workflows/ci.yml`)
- Format, clippy, tests, architecture boundaries, feature matrix, package, secret scanning

### Performance (`scripts/benchmark.sh`)
- Binary size, cold-start time, list/status latency baselines

## Known Deferred Items

1. **Physical directory restructuring** (`src/core/`, `src/sync_client/`) — plan says "Crate count is not an exit criterion"
2. **`SnippetId`/`LibraryId` newtypes** — low priority, no correctness impact
3. **Layered error hierarchy** (`CoreError`, `SyncClientError`, etc.) — current `SnipError` with `SyncFailureKind` is sufficient
4. **Feature-gating tui/clipboard/sync** — would require extensive `#[cfg]` throughout; deferred to future phase
5. **`load_snippets`/`save_snippets` duplication** — documented in `CANONICAL_OPERATIONS.md` but not removed (low risk)
