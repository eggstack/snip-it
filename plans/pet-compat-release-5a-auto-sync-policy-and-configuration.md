# Release 5A Plan: Auto-Sync Policy, Configuration, and Compatibility Contract

## Purpose

Define and implement the configuration and policy layer for optional post-mutation synchronization without changing existing manual or scheduled sync behavior.

Release 5 adds convenience for users accustomed to Pet's `auto_sync`, but snip-it's encrypted `snip-sync` protocol, conflict model, local-first persistence, and explicit security posture remain canonical. This phase establishes the contract before any mutation command begins triggering synchronization.

This document is intended for implementation-agent handoff. Inspect the current configuration model, sync settings, CLI commands, library mutation paths, process lifecycle, logging, error types, and tests before editing.

## Product Invariants

1. Auto-sync is disabled by default.
2. Local mutation commits before any remote work begins.
3. Remote failure never rolls back or corrupts a successful local mutation.
4. Existing `snp sync`, `snp cron`, daemon/service workflows, and scheduled sync behavior remain unchanged.
5. The existing encrypted protocol and merge/conflict behavior are reused; no plaintext or third-party sync backend is introduced.
6. Auto-sync never changes sync direction, credentials, server selection, library mapping, or conflict policy implicitly.
7. Machine-facing stdout remains free of background sync diagnostics.
8. Command bodies, output metadata, credentials, API keys, and encryption material are never included in auto-sync logs or errors.
9. Missing or invalid auto-sync configuration must fail safely and must not block ordinary local mutation unless the user explicitly chooses a strict failure policy.
10. Configuration changes are additive and backward compatible with existing files.

## Proposed Configuration

Evaluate the current config hierarchy and prefer one canonical location. Suggested shape:

```toml
[settings.sync]
auto_sync = false
auto_sync_debounce_seconds = 2
auto_sync_failure = "warn"
```

If sync settings already live in a separate file, avoid duplicating authoritative values across files. Choose one owner and document migration/precedence explicitly.

### Fields

#### `auto_sync`

Boolean. Default `false`.

When false, no mutation command should schedule post-mutation sync.

#### `auto_sync_debounce_seconds`

Bounded integer or duration. Suggested default: `2` seconds.

Define:

- minimum accepted value;
- maximum accepted value;
- whether zero means immediate execution or is rejected;
- serialization format;
- behavior for malformed or out-of-range values.

Avoid unbounded delays or integer overflow.

#### `auto_sync_failure`

Use a closed enum rather than free-form strings. Suggested values:

```text
ignore
warn
error
```

Recommended semantics:

- `ignore`: retain local success and suppress user-facing failure, while allowing bounded debug logging;
- `warn`: retain local success and emit a concise warning to stderr;
- `error`: local mutation remains committed, but the command returns a distinct post-commit sync failure outcome or nonzero exit code.

The `error` policy must never imply rollback. Documentation and error text must say that the local change succeeded and only synchronization failed.

Consider whether `error` is desirable for interactive commands. If retained, define exact exit-code behavior and machine-output implications before implementation.

## Workstream A: Configuration Ownership

### A1. Audit current configuration

Inspect:

- general application config;
- sync settings file;
- environment overrides;
- CLI flags;
- config cache/invalidation;
- serialization helpers;
- integrity/checksum handling;
- doctor/compatibility diagnostics.

Document which file owns:

- server URL;
- sync direction;
- credentials/key references;
- timeouts;
- automatic-sync policy.

### A2. Add typed settings

Introduce typed configuration structures with serde defaults so old files load unchanged.

Requirements:

- missing section yields disabled defaults;
- unknown future fields remain compatible under current serde policy;
- invalid enum values produce actionable diagnostics;
- save/load is idempotent;
- comments and unrelated settings are not unnecessarily rewritten unless the existing config system already normalizes them.

### A3. Precedence

If environment variables or CLI overrides are supported, define precedence explicitly:

```text
CLI one-shot override > environment > persisted config > defaults
```

Do not invent overrides unless they provide clear value. A persisted configuration-only implementation is acceptable if simpler and consistent with the repo.

## Workstream B: User-Facing Configuration Surface

Choose an additive command surface consistent with existing configuration commands. Possible approaches:

```text
snp config set sync.auto_sync true
snp config set sync.auto_sync_debounce_seconds 2
snp config set sync.auto_sync_failure warn
```

or a sync-specific surface:

```text
snp sync config --auto-sync on
snp sync config --debounce 2s
snp sync config --failure warn
```

Do not add both unless there is already a generic config command requiring integration.

Requirements:

- inspect current state;
- update one field without resetting unrelated sync settings;
- support non-interactive use;
- print machine-readable output only when explicitly requested;
- never print credentials or encryption secrets;
- provide clear disabled/not-configured states.

## Workstream C: Effective Policy Model

Create a small internal policy type resolved once per command invocation:

```rust
pub struct AutoSyncPolicy {
    pub enabled: bool,
    pub debounce: Duration,
    pub failure_mode: AutoSyncFailureMode,
}
```

Resolution should validate and clamp/reject configuration before mutation dispatch where possible, but invalid optional auto-sync config should not prevent local operations under the normal `warn`/`ignore` model.

Avoid reading configuration repeatedly after every mutation.

## Workstream D: Mutation Classification Contract

Before wiring triggers, define which operations are considered mutations.

Candidate mutation classes:

- create snippet;
- edit snippet command/description/tags/output;
- delete snippet/tombstone;
- import create/merge/replace;
- library create/delete/rename/set-primary if remote mapping semantics justify it;
- premade library installation;
- sync conflict resolution that writes local state;
- account/config changes — generally excluded.

Create an explicit enum or event model:

```rust
pub enum MutationKind {
    SnippetCreate,
    SnippetUpdate,
    SnippetDelete,
    Import,
    LibraryChange,
}
```

This phase should define the matrix but not necessarily wire every event yet.

For each command, specify:

- whether it mutates syncable library content;
- whether it should trigger auto-sync;
- which library identity is affected;
- whether multiple writes form one logical mutation;
- whether dry-run/cancel/failure emits no event.

## Workstream E: Failure and Exit-Code Contract

Document exact behavior for:

- no sync configuration;
- server unavailable;
- authentication failure;
- encryption/key error;
- conflict;
- timeout;
- local mutation succeeded but scheduling failed;
- background job could not be started;
- process exits before debounce fires.

Define stream ownership:

- stdout: original command payload only;
- stderr: concise warnings/errors;
- logs: bounded operational metadata without command bodies/secrets.

If a background/deferred process cannot communicate failure to the originating command after it exits, do not promise synchronous `error` semantics. Either constrain `error` to inline execution or choose a persisted status/reporting mechanism in Release 5B.

## Workstream F: Security and Privacy

Requirements:

- no command, description, output, tag, token, API key, encryption key, or plaintext payload in logs;
- no credentials in process arguments if a helper process is spawned;
- config files retain existing restrictive permissions;
- auto-sync cannot bypass encryption configuration;
- server URL diagnostics redact embedded userinfo/query secrets;
- malformed config cannot redirect sync to an unintended default server;
- no shell invocation for scheduling or sync dispatch.

Add sentinel tests with known secret strings across human output, JSON output, logs, and process arguments where observable.

## Workstream G: Diagnostics and Documentation

Extend `snp doctor --compatibility` or the appropriate diagnostic surface to report:

- auto-sync enabled/disabled;
- effective debounce;
- failure mode;
- sync configured/not configured;
- warning when auto-sync is enabled but no usable sync target exists;
- warning for unsupported or invalid values.

Do not expose credentials.

Update:

- README concise feature mention;
- USER_GUIDE setup and safety semantics;
- architecture sync/config docs;
- CLI exit-code/stream policy;
- CHANGELOG;
- AGENTS.md implementation notes.

## Workstream H: Tests

### Unit tests

Cover:

1. old config without section loads disabled defaults;
2. full config round-trip;
3. invalid failure mode;
4. debounce minimum/maximum/overflow;
5. precedence rules;
6. mutation classification matrix;
7. redaction helpers;
8. effective policy resolution;
9. unrelated settings preserved;
10. disabled policy produces no scheduling request.

### Integration tests

Cover:

1. enable/disable through chosen CLI;
2. inspect effective config;
3. no credentials in output;
4. existing manual sync behavior unchanged;
5. existing cron output unchanged;
6. malformed optional policy does not corrupt config or library;
7. doctor reports effective policy;
8. stdout remains clean for machine-facing mutation commands.

### Regression tests

Pin current behavior of mutation commands before triggers are added. In this phase, enabling the config may be accepted but should not yet cause sync unless Release 5B lands in the same commit series.

## Acceptance Criteria

Release 5A is complete when:

- a single authoritative typed auto-sync policy exists;
- defaults are disabled and backward compatible;
- failure semantics are explicit and local-first;
- the mutation-trigger matrix is documented;
- credentials and command content cannot leak through configuration/diagnostics;
- manual and scheduled sync behavior is unchanged;
- tests and documentation establish the contract for Release 5B/5C.

## Non-Goals

- Triggering synchronization from mutation commands in this phase.
- New sync providers or plaintext backends.
- Protocol or conflict-model redesign.
- Remote synchronization of local usage metadata or output metadata.
- Automatic shell startup modification.
- Long-running daemon introduction unless separately justified in Release 5B.
