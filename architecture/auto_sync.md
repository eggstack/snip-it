# Auto-Sync Architecture

Auto-sync is optional and disabled by default. After a successful local
mutation, the command records durable pending intent and attempts to detach one
`snp auto-sync-worker` helper. The parent never waits for network work.

## Runtime model

```text
mutation
  -> local atomic commit
  -> record pending generation G
  -> spawn detached auto-sync-worker
       -> acquire shared SyncExecutionLock
       -> debounce and re-read pending state
       -> run sync_commands::run_sync directly
       -> clear only generation G after Ok(())
       -> preserve newer generations and run a bounded follow-up cycle
       -> record status/backoff and exit
```

There is no child sync process, daemon, queue database, IPC channel, or
service-manager integration. The helper is opportunistic and bounded by both
the worker lifetime and `auto_sync_timeout_seconds` for each automatic-sync
attempt. The sync client caps requests, retries, and retry sleeps by the
remaining attempt deadline; local filesystem operations are not
force-cancelled.

## Contracts

- Local mutation succeeds before remote work begins.
- `SyncExecutionLock` serializes automatic, manual, explicit `--sync`, and cron
  sync operations. The helper owns it for the complete cycle.
- Pending generations are monotonic. Lower or corrupt state fails closed.
- `clear_if_generation_matches` is the automatic acknowledgement boundary.
  `GenerationChanged` preserves newer work; `Missing` is treated as already
  cleared; clear errors preserve recoverability and record failure.
- Authentication/configuration failures require attention. Transient failures
  retain pending intent and durable backoff. A failed helper exits without an
  immediate follow-up attempt, even if a newer generation appeared.
- Persistent lock metadata is diagnostic only; kernel-backed ownership is the
  authority.

## Module layout

- `policy.rs` — policy, failure classification, retry disposition, and
  direction resolution.
- `pending.rs` / `pending_lock.rs` — durable generation marker and its short
  transaction lock.
- `execution_lock.rs` — shared kernel-backed sync lock.
- `schedule.rs` / `notification.rs` — pending recording and detached spawn.
- `spawn.rs` — current-executable lookup, platform detachment, and stream
  routing for the single helper.
- `worker.rs` — debounce, preflight, direct canonical sync, exact-generation
  clear, status, and bounded follow-up loop.
- `status.rs` — durable result, backoff, and operator-attention state.

## Hidden command

`auto-sync-worker --state-dir <path>` is an internal, hidden command. It is the
only helper process used by auto-sync. It is not a public command surface and
is suppressed from startup recovery recursion.

## Configuration

`auto_sync_debounce_seconds` controls the quiet period and
`auto_sync_max_delay_seconds` prevents starvation. Manual sync and cron retain
their existing timeout behavior. Automatic sync uses
`auto_sync_timeout_seconds` as a per-attempt network/retry budget; a deadline
records `TransientTimeout`, preserves pending intent, and does not promise
cancellation of local I/O. Auto-sync defaults and command surface are
unchanged.

## Related design

See [sync.md](sync.md) for the canonical sync operation and
[overview.md](overview.md) for subsystem boundaries.
