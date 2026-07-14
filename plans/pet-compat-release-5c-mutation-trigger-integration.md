# Release 5C Plan: Mutation Trigger Integration and Local-First Auto-Sync

## Purpose

Wire successful syncable local mutations into the Release 5B auto-sync coordinator while preserving command-specific behavior, local-first persistence, cancellation semantics, and existing manual/scheduled synchronization.

This phase is where auto-sync becomes operational. It must use the Release 5A typed policy and Release 5B coordinator rather than embedding sync calls independently in each command.

## Product Invariants

1. Auto-sync triggers only after a complete successful local mutation.
2. A failed or cancelled mutation emits no request.
3. Dry-run and read-only commands emit no request.
4. One logical mutation emits at most one request, even if it performs multiple internal writes.
5. Remote failure never rolls back local state.
6. Sync-merge writes do not recursively trigger auto-sync.
7. Manual `snp sync` and scheduled sync retain current behavior.
8. Auto-sync remains disabled by default.
9. Machine-facing stdout remains unchanged.
10. The request contains stable identity and mutation metadata only, never snippet content.

## Workstream A: Central Mutation Notification API

Provide one narrow helper used by commands after commit:

```rust
pub fn notify_local_mutation(
    policy: &AutoSyncPolicy,
    context: MutationContext,
) -> AutoSyncNotificationResult
```

Suggested context:

```rust
pub struct MutationContext {
    pub kind: MutationKind,
    pub origin: MutationOrigin,
    pub library_id: Option<String>,
    pub committed_at: i64,
}
```

The helper should:

- no-op when disabled;
- suppress sync-origin mutations;
- submit to the coordinator;
- return scheduling status without exposing snippet content;
- apply failure policy only to scheduling/available results;
- never own local rollback.

Avoid direct calls to `run_default_sync()` scattered across mutation commands.

## Workstream B: Command-by-Command Trigger Matrix

Audit every mutating command and explicitly classify it.

### B1. Snippet creation

Cover all acquisition sources:

- positional;
- interactive prompt;
- multiline;
- command stdin;
- from-file;
- editor;
- shell current-buffer/history helpers.

Emit one `SnippetCreate` request only after atomic save succeeds.

Metadata prompt cancellation, validation failure, editor failure, or save failure emits none.

### B2. Snippet editing

Cover:

- command/description/tag edits through editor workflow;
- output/notes set;
- output stdin;
- clear output;
- favorite/folder changes if supported by edit/TUI paths.

Emit one `SnippetUpdate` request after save.

If `output` remains local-only and is not part of the sync protocol, decide whether output-only edits should trigger remote sync. Preferred behavior: do not schedule sync for a mutation that changes only local-only fields. Encode this in the mutation matrix and tests.

### B3. Delete/tombstone

Emit one `SnippetDelete` request after tombstone persistence.

Deletion cancellation or failed save emits none.

Preserve existing explicit `--sync` behavior. If a command already requests immediate sync, avoid additionally scheduling a debounced attempt unless the coordinator can deduplicate it reliably. Define precedence:

```text
explicit immediate sync > auto-sync scheduling
```

### B4. Import

For `snp import pet`:

- dry-run: no request;
- create: one request after destination library and metadata are committed;
- merge: one request only if the destination changed;
- replace: one request after replacement and backup complete;
- strict abort/collision/parse failure: none.

Do not emit one request per imported snippet.

### B5. Library operations

Classify:

- create;
- delete;
- rename;
- set-primary;
- link/unlink sync mapping;
- premade install.

Only trigger when the operation changes data represented remotely under the current protocol. Local-only library metadata such as primary selection should generally not trigger sync.

Document rationale for each exclusion.

### B6. TUI mutation paths

Cover delete/favorite/folder/edit actions performed inside selectors.

Ensure sorted/display row identity maps to the correct stable snippet/library identity before emitting a mutation request.

### B7. Sync writes

Any local write resulting from:

- pull/merge;
- conflict resolution;
- remote deletion application;
- recovery from server state;

must use `MutationOrigin::SyncMerge` and never schedule another automatic sync.

## Workstream C: Logical Transaction Boundaries

Several commands may write:

- library TOML;
- library registry metadata;
- backup files;
- sync mapping/config;
- audit logs.

Define the authoritative commit point per command. Auto-sync should be submitted only after all state required for a consistent local view has committed.

Backup failure policy must remain consistent with existing behavior. Do not schedule sync if the primary mutation later returns an error indicating the operation did not complete.

## Workstream D: Explicit Sync Flags and Precedence

Audit existing `--sync` or post-operation sync options.

Define:

- whether explicit sync runs immediately;
- whether auto-sync is suppressed after explicit success;
- what happens after explicit failure;
- whether debounce markers are cleared after manual/explicit sync;
- whether manual sync consumes pending intent.

Recommended behavior:

1. successful explicit/manual sync clears matching pending auto-sync state;
2. explicit failure leaves local mutation committed and pending intent available according to policy;
3. no duplicate immediate plus delayed sync for the same mutation generation.

## Workstream E: User Feedback

Default successful mutations should not become noisy.

### Disabled

No new output.

### Scheduled

Prefer no stdout output. Optional concise stderr/status message only if existing UX conventions support it.

### Scheduling failure

Apply failure mode:

- ignore: silent;
- warn: `Local change saved; automatic sync could not be scheduled: <bounded reason>`;
- error: nonzero outcome only under the exact Release 5A/5B contract, always stating local success.

### Background remote failure

Do not inject asynchronous text into an unrelated terminal. Record status for doctor/status inspection and bounded logs.

## Workstream F: Idempotency and Deduplication

Assign a mutation generation or request timestamp sufficient for coordinator coalescing.

Do not persist per-snippet payloads.

Test repeated notifications caused by retries, command wrapper layers, or TUI loops. The same logical mutation must not generate multiple independent syncs.

## Workstream G: Multi-Library Semantics

Determine how current sync maps libraries:

- one global account/state;
- per-library mapping;
- primary library only;
- linked libraries.

Requests must target the correct scope. Do not sync a different library because primary selection changed between mutation and coordinator execution.

Use stable mapping identity captured after commit. If the existing sync command is global, coalesce multiple library mutations into one global sync safely.

## Workstream H: Audit and Logging

Keep existing audit behavior unchanged. Auto-sync notification logs may include:

- mutation kind;
- stable opaque library ID;
- scheduling/result class;
- timestamps/durations.

They must not include:

- command;
- description;
- output;
- tags;
- source file contents;
- credentials;
- encryption material;
- unredacted sensitive URLs.

## Workstream I: Tests

### Unit tests

Cover trigger decisions for every mutation class:

1. enabled/disabled;
2. user versus sync origin;
3. local-only field mutation;
4. create/update/delete;
5. dry-run;
6. no-op merge;
7. explicit-sync precedence;
8. exactly-one notification;
9. commit failure;
10. failure-mode rendering.

### Integration tests with fake coordinator/executor

Cover:

1. `snp new` each source schedules once after save;
2. validation/cancel schedules none;
3. edit schedules once;
4. output-only edit follows documented local-only policy;
5. delete schedules once;
6. import create/merge/replace schedules once;
7. import dry-run schedules none;
8. no-op import merge schedules none;
9. explicit sync prevents duplicate delayed attempt;
10. manual sync clears pending marker;
11. remote merge writes do not recurse;
12. local file exists with expected content before fake executor is called;
13. background failure leaves local content intact;
14. stdout schemas remain unchanged;
15. command/secret sentinel absent from coordinator state/logs.

### PTY tests

Cover TUI delete/edit/favorite paths where applicable:

- cancellation;
- identity after sorting/filtering;
- one notification for selected snippet;
- terminal restoration;
- no asynchronous warning injected into alternate screen.

### Concurrency tests

Run multiple mutation processes rapidly and prove:

- all local writes complete correctly under existing locking semantics;
- coordinator coalesces requests;
- no sync storm;
- pending intent remains after executor failure.

## Workstream J: Documentation

Update:

- README concise opt-in description;
- USER_GUIDE configuration and trigger matrix;
- architecture sync/mutation flow;
- CLI exit-code/stream policy;
- doctor/status documentation;
- CHANGELOG;
- PET compatibility matrix;
- AGENTS.md.

Include a precise sequence diagram:

```text
user command
  -> validate
  -> local atomic write
  -> audit/local success
  -> notify coordinator
  -> debounce/coalesce
  -> existing encrypted sync
```

Emphasize that auto-sync is convenience, not a replacement for backups or a guarantee of immediate remote durability.

## Acceptance Criteria

Release 5C is complete when:

- all syncable user mutation paths use one notification API;
- triggers occur strictly after commit;
- dry-run/cancel/failure/no-op paths emit none;
- local-only mutations are treated according to explicit protocol scope;
- explicit/manual sync does not cause duplicate delayed sync;
- sync-origin writes cannot recurse;
- local state survives every remote/scheduling failure;
- tests prove exactly-once logical notification and clean stdout;
- auto-sync remains disabled by default.

## Non-Goals

- Per-snippet remote event streaming.
- Synchronizing usage or local-only output metadata.
- New conflict resolution semantics.
- Triggering remote work before local save.
- Guaranteed remote completion before an interactive command returns.
