# snip-it Canonical Terminology and Domain Model

Status: normative companion to `plans/000-long-term-specification.md`

This document defines the language snip-it implementation plans, protocol
types, storage schemas, architecture documents, tests, CLI labels, and
operator documentation MUST use. When current code uses a term differently,
the compatibility mapping here describes the migration target.

## 1. Naming rules

1. A durable snippet or device concept MUST have a stable identifier rather
   than a path-derived or display-derived string.
2. A filesystem path is a locator and MUST NOT be treated as durable snippet
   or library identity.
3. A snippet, a library, a device, a sync generation, a transaction journal,
   a pending marker, a selector match, a CLI outcome, and an MCP projection
   are distinct objects.
4. Terms MUST NOT be used as interchangeable shorthand when they cross
   persistence, sync, authorization, or protocol boundaries.
5. Compatibility fields MAY remain during migration but MUST be labeled as
   compatibility projections.

## 2. Top-level relationships

```text
Deployment
|-- snp installs (snip-it crate)
|-- snip-sync servers (snip-sync crate, at most one live per state dir)
|-- Libraries
|   |-- Snippets
|   |   |-- Synced fields (identity, description, command, tags, notes, updated_at, device_id)
|   |   `-- Local-only fields (output, folders, favorite)
|   |-- libraries.toml (linkage + last_sync)
|   |-- Transaction journals (<config>/.transaction/)
|   |-- Pending markers (state dir; monotonic generations)
|   `-- Recovery markers (<library>.sync_recovery)
|-- Sync protocol (snip-proto: ProtoSnippet, PushSnippets idempotent)
|-- CLI surface (Commands + DataCommands alias, CliOutcome -> exit codes)
`-- MCP projection (read-only search/get over selector::searchable_text)
```

The principal runtime relationship is:

```text
Device
  -> Library
  -> Snippet (synced fingerprint + local-only overlay)

Snippet selection
  -> SearchFields / searchable_text
  -> resolve_selector (mutating) | resolve_selector_readonly (side-effect-free)
  -> run_snippet_selection (Option<&Runtime>)
  -> CliOutcome -> exit code
```

## 3. Snippet terms

### Snippet

The unit of stored executable text plus metadata. Identity is stable across
edits; conflict identity is `(updated_at, device_id, SHA-256(synced fields))`.

### Synced fields

The fields covered by the sync fingerprint and `ProtoSnippet`: identity,
description, command, tags, notes, timestamps, device origin, deletion
markers. `output` is explicitly not in `ProtoSnippet`.

### Local-only fields

`output`, `folders`, and `favorite`. They MUST NOT enter the fingerprint,
MUST NOT cross the sync boundary, and MUST NOT be searchable except
`output`/notes through the explicit `list --search-output` / `search_output`
opt-in.

### Device

A sync participant identified by `device_id`. Conflict resolution MUST NOT
depend on device roles; there is no server-wins rule.

## 4. Library and persistence terms

### Library

A named snippet collection with its own file plus linkage metadata in
`libraries.toml` (`last_sync`, remote binding).

### Transaction journal

A crash-recovery record under `<config>/.transaction/`. Transaction APIs
take `.transaction`; pending-marker APIs take the state dir. One journal
means auto-rollback; multiple or incomplete journals mean refuse and direct
to `snp repair`.

### Pending marker

An auto-sync scheduling record with a monotonic generation. Lower
generations are corrupt (preserve the marker, spawn nothing), except a
lower generation with a strictly newer timestamp clears, re-records, and is
adopted as new work.

### Recovery marker

An atomic `<library>.sync_recovery` record used for missing-library
relinking. It preserves corrupt markers, allows exactly one normalized
remote-name match, fails on ambiguity, and is removed only after durable
relink plus retry sync.

### Golden corpus

The save/load compatibility fixture set (tabs, trailing spaces, CRLF) that
MUST survive the save path byte-for-byte. The save path MUST NOT
post-process `toml::to_string_pretty`.

## 5. Sync and execution terms

### Sync generation

A monotonic scheduling epoch for auto-sync work. It MUST NOT go backward
except through the timestamp-qualified re-record rule above.

### SyncExecutionLock

The single async/sync ownership primitive shared by manual sync and the
detached `auto-sync-worker`. Kernel `flock`/`LockFileEx` state is
authoritative over in-memory or file-metadata claims.

### Auto-sync worker

A detached process using `tokio::new_current_thread()`. It MUST NOT
introduce a second daemon, scheduler, or process-control architecture.

### PushSnippets batch

A Prost-`encoded_len()` byte-bounded upload unit (client 3.5 MiB ceiling
below the server 4 MiB gRPC limit), sorted deterministically by snippet ID.
Failures preserve the original `SyncFailureKind` via `add_batch_context()`.

### Retry policy

The single `retry_grpc_unified!` inline-expansion macro plus `RetryBackoff`
in `src/sync.rs`. RPC methods take `&mut self` so reborrows work. A generic
closure-based retry helper is a compile error and MUST NOT return.

## 6. Selection and outcome terms

### SearchFields / searchable_text

The canonical fuzzy-search projection shared by `snp get --query`,
`snp list --filter`, and MCP `snippets_search`: description plus command
always, tags by default, output/notes only on explicit opt-in.

### resolve_selector vs resolve_selector_readonly

`resolve_selector` may mutate (e.g. record selection effects);
`resolve_selector_readonly` is side-effect-free and backs shared inspection
plus read-only paths. New read paths MUST use the readonly resolver plus
`inspect_library_index` where applicable.

### CliOutcome

The single CLI-to-exit-code translation owned by `src/outcome.rs` per
`docs/EXIT_CODES.md`. Consolidation work MUST NOT reintroduce parallel
outcome enums that duplicate success/cancel/execution-failure semantics.

### Commands vs DataCommands

Two Clap schemas for one semantic operation family. `snp data` is an alias;
each semantic command has one canonical `*Args` type and one `handle_*`
path in `src/main.rs`.

## 7. Server terms

### snip-sync lifecycle

The baseline primitives `serve`, `stop`, `restart`, `croncheck`, and
`/health`, plus process identity checks and the kernel-backed singleton
server lock. OS startup wrappers consume them; they MUST NOT become a
second daemon architecture.

### EggServe leaf service

The one concrete two-route (`/health`, `/metrics`) native HTTP/1 service
in `snip-sync/src/http.rs` using only `eggserve-server` and
`eggserve-primitives`. Tonic stays on its separate gRPC listener with both
listeners pre-bound. No Axum/Tower-HTTP, no generic router/middleware.

### Wire parity

The proven HTTP contract locked in `tests/snip_sync_lifetime.rs`: empty
router 404/405 with no content-type (metrics-disabled 404 keeps
`"Not found"`), known-route preflight/unsupported-method `Allow: GET, HEAD`,
`Vary: origin` on non-allow-all responses (omitted for allow-all), and no
security headers on preflight.

## 8. Distribution and update terms

### Release identity

Independent crate versions with independent exact tags: `snip-it X.Y.Z`
maps to GitHub tag `vX.Y.Z`; `snip-sync A.B.C` maps to
`snip-sync-vA.B.C`. Stable asset names omit the version because the tag is
the namespace (`snp-<target>`, `snip-sync-<target>`, plus `.sha256`).

### Binary-first install/update

Detect host, install the verified prebuilt binary when available, fall back
to exact-version Cargo only when needed. Integrity failure is hard failure.
`snip-sync update` intentionally retains external `curl` after the measured
+34.23% lean-eggfetch trial; `snp update` uses in-process lean
`eggfetch-core 0.2.0` with delegated strict redirects and native
`Timeout.total`.

### MCP integration

A local stdio read-only adapter (`snippets_search`, `snippet_get`) launched
on demand by the agent client, plus explicit client registration. No
mutations, no command execution, no networked MCP, no long-running MCP
service.

## 9. Compatibility mapping

- Legacy path-derived project or library references are compatibility
  projections; stable library linkage in `libraries.toml` is authoritative.
- Legacy duplicate CLI schemas map to one canonical `*Args` per semantic
  command.
- Legacy parallel outcome enums map to `CliOutcome`.
- Legacy transport shims (manual redirect state machines, outer Tokio
  timeouts, Axum adapters, generic HTTP abstractions) map to delegated
  eggfetch/EggServe ownership as recorded in the updater and server
  subsystem roadmaps. Do not revive them.
