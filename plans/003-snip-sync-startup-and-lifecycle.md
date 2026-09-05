# Plan 003: snip-sync startup and lifecycle management

Status: ready

Depends on: Plan 001

## Objective

Turn the existing `snip-sync` process/lifecycle primitives into a small cross-platform deployment interface for boot startup and fleet administration.

Do not introduce a second daemon, supervisor library, background manager, or HTTP control API.

The desired CLI is:

```text
snip-sync startup install
snip-sync startup install --method systemd|launchd|cron|task-scheduler
snip-sync startup instructions
snip-sync startup instructions --method ...
snip-sync startup uninstall
snip-sync restart
snip-sync croncheck
```

`startup instructions` is always read-only and should print exact commands/files for manual execution.

## Existing code to reuse

Reuse rather than replace:

- `serve()` and `/health`;
- kernel-backed `ServerLock` ownership metadata;
- `cmd_stop` identity validation;
- `cmd_restart` flow;
- `cmd_croncheck` health probe, serialization, and detached spawn;
- `snip_sync::paths` and bootstrap/config creation.

Likely files:

```text
snip-sync/src/cli.rs
snip-sync/src/main.rs
snip-sync/src/startup.rs       new, preferred home for deployment logic
snip-sync/src/process.rs       narrow Windows stop/maintenance-lock corrections
snip-sync/src/lib.rs           module export as needed
snip-sync/README.md
packaging/systemd/...          optional canonical rendered fixture
packaging/launchd/...          optional canonical rendered fixture
```

Keep `serve` unaware of supervisor implementation details.

## Startup method model

Use a small enum:

```text
Systemd
Launchd
Cron
TaskScheduler
Direct              # internal/unmanaged state, not necessarily installable
```

CLI method argument adds `Auto`.

Auto detection:

```text
Linux + actual systemd environment -> systemd
Linux without systemd              -> cron
macOS                               -> launchd
Windows                             -> task-scheduler
other                               -> cron only if a POSIX crontab is actually available; otherwise instructions/error
```

### Correct systemd detection

Do not detect systemd merely because `systemctl` exists.

Use the lightweight pattern already proven in Gregg:

- `/run/systemd/system` exists;
- `/proc/1/comm` is `systemd` where readable;
- a short bounded `systemctl` probe only as a secondary signal when needed.

If auto detects a real systemd host but installation lacks permission, print the exact elevated systemd command. Do not silently install cron instead.

## Systemd behavior

Prefer a generated unit using absolute paths resolved at installation time.

The system service should run under the intended non-root account, not as root. If invoked through `sudo`, preserve the original account from `SUDO_USER` where available.

The generated unit should include only reasonable service basics:

```text
Type=simple
ExecStart=<absolute snip-sync> serve
Restart=on-failure
RestartSec=<small bounded value>
WantedBy=multi-user.target
```

Do not copy Gregg's extensive hardening directives mechanically; this is a small local sync service and those directives can conflict with user-owned config/data paths. Add only settings that are clearly correct for the actual snip-sync paths.

`startup install --method systemd` should:

1. ensure layout/config exist without overwriting config;
2. render/write the unit;
3. run `systemctl daemon-reload`;
4. enable and start/restart the unit;
5. check `/health` with a bounded deadline;
6. print the unit path and useful status command.

If root is required and unavailable, return nonzero after printing the exact `sudo <current-exe> startup install --method systemd` command.

## launchd behavior

Use a small LaunchDaemon plist or an explicitly documented LaunchAgent if implementation proves user-level launch is preferable.

Baseline preference for fleet boot startup is a LaunchDaemon using an absolute program path and arguments `serve`.

As with systemd, do not duplicate server logic in the plist. KeepAlive/restart behavior should be simple and bounded.

Non-root invocation must print the exact elevated command rather than falling back to cron on macOS.

## cron behavior

Cron is the fallback for Linux hosts without systemd and other POSIX-like hosts where `crontab` is available.

Install idempotent entries equivalent to:

```cron
@reboot <env> <absolute-snip-sync> croncheck
*/5 * * * * <env> <absolute-snip-sync> croncheck
```

Use exact marker comments so install/uninstall can update only snip-sync-owned lines without disturbing unrelated user cron entries.

The command path must be safely quoted. Reject newline/control characters in paths rather than constructing unsafe cron text.

`croncheck` remains the health watchdog; the crontab must not call `serve` directly.

## Windows Task Scheduler

Avoid Windows SCM/service implementation in this phase; it adds a separate service runtime/control model and is not required for the primary SBC use case.

Use Task Scheduler as the Windows equivalent of croncheck:

- one startup/logon trigger as allowed by privilege;
- one periodic five-minute trigger;
- action invokes the absolute `snip-sync.exe croncheck` command;
- entries have stable names so install/uninstall is idempotent.

Administrator installs may use an at-startup task. A non-admin path may use an at-logon task if that is the highest reliable privilege-free option; document the difference clearly.

## Critical Windows croncheck lock correction

Current `snip-sync::process::try_lock()` uses a create-new/delete-on-drop file lock on non-Unix hosts. A crash can leave the file behind forever and cause every later Windows `croncheck` to skip.

Before enabling Task Scheduler:

- replace the Windows maintenance-lock implementation with kernel-backed `LockFileEx` semantics equivalent to the server singleton lock, or
- factor a small reusable kernel lock helper used by both.

The lock file may remain on disk after exit; kernel ownership, not file existence, must be authoritative.

Add a Windows test proving a stale on-disk lock file without a held kernel lock does not block a subsequent maintenance lock.

## Windows stop/restart

Current stop logic intentionally returns unsupported on non-Unix. To make Task Scheduler/update practical without SCM, add the smallest safe Windows stop implementation.

Requirements:

1. use the current server-lock owner metadata;
2. verify PID is live;
3. verify the recorded process start token still matches;
4. open only that process with the minimal termination/synchronization rights;
5. terminate and wait with a bounded timeout using `windows-sys` primitives already present;
6. never scan process names or kill by executable name;
7. after termination, verify the server kernel lock becomes acquirable.

A forced process termination is acceptable for this lightweight local server because SQLite provides transactional crash recovery, but document that Windows does not currently provide the same graceful SIGTERM path as Unix.

Do not add a general Windows service framework solely to get graceful stop.

Once this lands, existing `cmd_restart` can become cross-platform with minimal branching.

## Transport/environment policy for generated startup entries

Do not make startup registration accidentally expose plaintext service traffic.

When rendering an unattended startup entry:

- if the current install invocation explicitly has `TLS_ENABLED=true`, preserve that acknowledgement in the generated environment;
- otherwise load the server config and require both gRPC and HTTP bind addresses to be loopback before persisting `SNIP_SYNC_ALLOW_HTTP=true` for local backend operation;
- if a non-loopback bind is configured without explicit TLS-termination acknowledgement, refuse automatic startup installation and print the correction/instructions.

Prefer also moving/centralizing the `SNIP_SYNC_ALLOW_HTTP` safety check so it cannot be used with a non-loopback bind accidentally. This is a narrow safety correction, not a TLS redesign.

A same-host reverse proxy may continue to connect to a loopback plaintext backend.

## Manager-aware restart

After startup installation exists, `snip-sync restart` should detect a known installed/active supervisor before using direct stop+serve.

Expected behavior:

```text
active systemd service -> systemctl restart + bounded health check
loaded launchd job     -> launchctl kickstart/restart + bounded health check
Task Scheduler watchdog/direct running server -> safe stop + detached start/croncheck
cron watchdog          -> safe stop + croncheck/direct detached start
unmanaged foreground   -> existing direct stop + serve semantics
not running            -> start according to installed manager if known, otherwise direct serve
```

Keep detection deterministic. Do not inspect every process on the host.

## `startup uninstall`

Remove only assets owned by snip-sync:

- named systemd unit;
- named launchd plist/job;
- marked cron lines;
- named Task Scheduler jobs.

Do not delete binaries, config, database, premade libraries, or certificates.

## Tests

Required unit/pure coverage:

- method auto-detection;
- systemd environment detection;
- path/shell quoting;
- exact systemd/plist/cron/task rendering;
- idempotent install marker handling;
- uninstall removes only owned records;
- loopback/non-loopback transport-policy decision;
- Windows stale lock file does not block kernel lock acquisition.

Required integration smoke where the host allows it:

- Linux systemd rendering and command sequence in an isolated/mock execution layer;
- cron install/uninstall against a fixture crontab;
- macOS plist parse (`plutil -lint`) on macOS CI;
- Windows Task Scheduler command rendering plus Windows direct stop/restart against a temporary `snip-sync` server process.

Do not make ordinary CI mutate the runner's real boot configuration.

## Acceptance criteria

1. `snip-sync startup instructions` is read-only and useful on every supported OS.
2. Linux auto mode chooses systemd only when systemd is actually running.
3. Failure to install systemd/launchd due to privilege prints the exact elevated command and does not silently create cron.
4. Cron install is idempotent and uses `croncheck`, not direct `serve`.
5. Windows Task Scheduler uses `croncheck` and has stable install/uninstall identities.
6. A stale Windows maintenance-lock file cannot permanently disable croncheck.
7. Windows `snip-sync stop`/`restart` works using lock-owner identity and bounded process termination.
8. Generated unattended startup refuses unsafe non-loopback plaintext configuration without explicit acknowledgement.
9. `restart` uses an installed manager when one is active and otherwise preserves the existing direct behavior.
10. Startup uninstall never removes user data/config.
