# JSON Output Schemas

> **Evergreen contract.** Machine-readable JSON schemas for commands that
> support `--json` or `--report json`.
> Enforced by `tests/schema.rs`; per-command emitters are listed in
> `architecture/outcome.md`.
>
> **Note:** Line numbers are approximate and may drift as the codebase evolves.

---

## Rules

1. All field names use `snake_case`.
2. Timestamps are Unix epoch milliseconds as integers (`*_unix_ms`), or epoch
   seconds as integers (`created_at` / `updated_at` on snippets). **No
   command currently emits ISO 8601 / RFC 3339 strings.**
3. Snippet IDs are `legacy-<64 hex chars>` (deterministic SHA-256), not UUIDs.
   See [`IDENTITY_CONTRACT.md`](IDENTITY_CONTRACT.md).
4. Enum values serialize with Rust variant names (`PascalCase`) unless a
   `serde(rename)` overrides it. **Struct-variant enums serialize as
   objects** (`{"LiveExecution":{…}}`), not bare strings — see `sync.top_level`,
   `pending.state`, `execution_lock`, and `worker_lock` below.
5. Ordering is deterministic (sorted by ID or insertion order — never hash-map random).
6. New fields are additive (non-breaking).
7. Breaking changes increment the `schema` version number.
8. No ANSI escape sequences in any JSON output.
9. Secret values (API keys, passwords) are never included in JSON output.
10. **Optional-field caveat:** most optional fields serialize as explicit
    `null`, but some carry `#[serde(skip_serializing_if = "Option::is_none")]`
    and are **omitted** entirely — e.g. `CompatibilityDiagnostic.span`
    (`src/diagnostics.rs`) and `StatusDiagnostic.remediation`. Do not assume a
    key is always present; check the serde attribute on the struct.

---

## `list --json`

```json
[
  {
    "description": "string",
    "command": "string",
    "output": "string",
    "tags": ["string"],
    "folders": ["string"],
    "favorite": false
  }
]
```

- Items are sorted by fuzzy relevance ranking (default), or by the explicit `--sort` mode.
- `deleted` snippets are excluded (consistent with TUI display).
- `output` is always present but may be an empty string.
- No wrapping envelope — the output is a bare JSON array.

## Local MCP tool results

The local MCP adapter returns a JSON object inside both `structuredContent`
and a text content item. Successful snippet entries use these stable fields:
`id`, `library`, `description`, `command`, `tags`, `folders`, and `favorite`.
`snippets_list` and `snippets_search` wrap entries in `snippets`; a successful
`snippet_get` returns one entry directly. Not-found and ambiguous
`snippet_get` calls return `isError: true` with an `error` code and structured
`matches` data. Credentials, sync state, local output/notes, and execution
are out of scope.

---

## `get --json`

```json
{
  "schema": 1,
  "id": "string",
  "description": "string",
  "command": "string",
  "expanded": "string | null",
  "tags": ["string"],
  "library": "string",
  "library_id": "string"
}
```

- `command` is the raw template (with `<var>` placeholders).
- `expanded` is the fully expanded command (variables replaced with defaults or prompt values); `null` when `--expanded` is not used.
- `library` is the display name; `library_id` is the filename stem (empty string when not in library mode).

---

## `status --json`

```json
{
  "schema": 1,
  "generated_at_unix_ms": 0,
  "config_root": "string",
  "log_dir": "string",
  "local": {
    "libraries": 0,
    "snippets": 0,
    "primary_library": "string | null"
  },
  "sync": {
    "configuration": "string",
    "top_level": "string"
  },
  "pending": {
    "state": "string"
  },
  "attempt": {
    "state": "string",
    "last_attempt_generation": 0,
    "last_attempt_at_unix_ms": 0,
    "last_success_at_unix_ms": 0,
    "last_failure_class": "string",
    "consecutive_failures": 0,
    "next_attempt_at_unix_ms": 0,
    "attention_required": false,
    "message": "string"
  },
  "execution": {
    "execution_lock": "string",
    "worker_lock": "string"
  },
  "diagnostics": [
    {
      "severity": "string",
      "code": "string",
      "message": "string",
      "remediation": "string | null"
    }
  ]
}
```

- `sync.configuration` is one of: `"NotConfigured"`, `"Configured"`, `"ConfiguredAutoSyncDisabled"`, `"LoadFailed"`.
- `sync.top_level` has two **struct** variants and six bare variants
  (`src/status_snapshot.rs:53-62`):
  - bare strings: `"CorruptOrInaccessible"`, `"PendingAttentionRequired"`, `"PendingAwaitingScheduling"`, `"ConfiguredAndCurrent"`, `"ConfiguredAutoSyncDisabled"`, `"NotConfigured"`
  - objects: `{"LiveExecution":{"pid":0,"started_at_unix_ms":0}}`, `{"PendingRetryBackoff":{"next_attempt_at_unix_ms":0}}`
- `pending.state` has one bare variant and three **struct** variants
  (`src/status_snapshot.rs:65-77`):
  - bare string: `"None"`
  - objects: `{"Pending":{"generation":0,"created_at_unix_ms":0}}`, `{"Corrupt":{"reason_code":"…"}}`, `{"Inaccessible":{"reason_code":"…"}}`
- `attempt.state` is one of: `"NeverAttempted"`, `"Succeeded"`, `"RetryScheduled"`, `"AttentionRequired"`, `"Deferred"`, `"Corrupt"`.
- `execution_lock` and `worker_lock` have three bare variants and two **struct**
  variants (`src/status_snapshot.rs:108-114`):
  - bare strings: `"Idle"`, `"Malformed"`, `"Inaccessible"`
  - objects: `{"Live":{"pid":0,"started_at_unix_ms":0}}`, `{"DeadStale":{"pid":0}}`

---

## `doctor --report json`

Emitted by serializing `DoctorReport` (`src/diagnostics.rs:108-124`).
**There is no `schema`, `mode`, `file`, `entries`, or `summary` field.**

```json
{
  "schema_version": "1.0.0",
  "tool_version": "string",
  "source": "string | null",
  "analysis_mode": "string",
  "total_entries": 0,
  "diagnostics": [ { "...": "CompatibilityDiagnostic" } ],
  "duplicates": [],
  "normalizations": [],
  "has_toml_error": false,
  "toml_error_detail": "string | null",
  "recommended_import_command": "string | null",
  "detected_capabilities": [],
  "strict_mode": false,
  "dry_run": true
}
```

- `severity` on each diagnostic is one of: `"Info"`, `"Warning"`, `"Error"`
  (`DiagnosticSeverity`, `src/status_snapshot.rs:131-135`) — **capitalized**,
  not lowercase.
- `analysis_mode` reflects the doctor sub-mode: `"compatibility"`, `"sync"`,
  `"check-shell"`, `"library"`, `"pet-file"`. There are **five** modes.
- `total_entries` is the count of `diagnostics`; there is no per-severity
  breakdown, so aggregate error/warning counts must be computed by the caller.
- `dry_run` is always `true` for doctor (kept for report-schema uniformity).
- `CompatibilityDiagnostic.span` is **omitted** when `None`
  (`skip_serializing_if`).

---

## `validate --json`

```json
{
  "schema_version": "1.0.0",
  "tool_version": "string",
  "strict_mode": false,
  "dry_run": true,
  "total_libraries": 0,
  "total_snippets": 0,
  "diagnostics": [
    {
      "code": "string",
      "severity": "string",
      "path": "string | null",
      "library": "string | null",
      "snippet_id": "string | null",
      "message": "string",
      "repairability": "string"
    }
  ]
}
```

- `severity` is one of: `"Info"`, `"Warning"`, `"Error"`.
- `repairability` is one of: `"Auto"`, `"Manual"`, `"Unrepairable"`.
- `code` is a machine-readable diagnostic code (e.g., `"E-DUP-ID"`, `"W-DESC-EMPTY"`).
- `dry_run` is always `true` (validate is read-only).

---

## `backup --json`

```json
{
  "backup_dir": "string",
  "schema": 0,
  "version": "string",
  "file_count": 0,
  "total_bytes": 0
}
```

- `backup_dir` is the path to the created backup directory.
- `schema` is the backup manifest schema version number.
- `version` is the snip-it version that created the backup.
- Backup files exclude secrets (API keys, passwords are redacted).

---

## `restore --json`

`snp restore --mode dry-run` emits a **different, manifest-focused** envelope
first (`src/commands/restore_cmd.rs:1092-1095`) and performs no writes, no lock,
and no transaction:

```json
{
  "mode": "DryRun",
  "manifest_schema": 0,
  "manifest_version": "string",
  "files_in_backup": 0,
  "files": [ { "...": "per-file plan entry" } ]
}
```

`merge` and `replace` emit the `RestoreReport` shape:

```json
{
  "mode": "Merge",
  "files_restored": 0,
  "conflicts": [
    {
      "library": "string",
      "kind": "string",
      "detail": "string"
    }
  ],
  "skipped": ["string"],
  "pre_restore_backup": "string | null"
}
```

- `mode` is built with `format!("{:?}")` over `RestoreMode`
  (`src/commands/restore_cmd.rs:1084`), so values are **PascalCase**:
  `"DryRun"`, `"Merge"`, `"Replace"`. It is not `"dry-run"`.
- `conflicts` lists libraries where restore encountered conflicts.
- `skipped` lists library names that were skipped.
- `pre_restore_backup` is the path to the pre-restore backup, if created.

---

## `repair --json`

```json
{
  "items": [
    {
      "action": "string",
      "category": "string",
      "transaction_id": "string | null",
      "problem": "string",
      "fix": "string",
      "safe": false
    }
  ],
  "backups": ["string"],
  "applied": 0,
  "skipped": 0,
  "failed": 0,
  "exit_status": "string"
}
```

- `action` is the Debug representation of the repair action (e.g., `"Quarantine"`, `"Recreate"`, `"Fix"`).
- `category` describes the type of repair.
- `safe` indicates whether the repair is considered safe to apply automatically.
- `exit_status` is one of: `"clean"`, `"repaired"`, `"partial_failure"`, `"unsafe_only"`, `"dry_run"`.

---

## `import --report json`

```json
{
  "schema_version": "1.0.0",
  "tool_version": "string",
  "source": "string",
  "destination": "string | null",
  "analysis_mode": "string",
  "mutation_flag": false,
  "total_entries": 0,
  "imported": 0,
  "skipped": 0,
  "duplicates": [
    {
      "source_index": 0,
      "destination_index": 0,
      "description": "string",
      "reason": "string"
    }
  ],
  "diagnostics": [
    {
      "code": "string",
      "severity": "string",
      "message": "string",
      "entry_index": 0,
      "field": "string | null",
      "suggestion": "string | null",
      "span": { "start": 0, "end": 0 }
    }
  ],
  "normalizations": [
    {
      "entry_index": 0,
      "field": "string",
      "original": "string",
      "normalized": "string"
    }
  ],
  "detected_capabilities": ["string"],
  "dry_run": false,
  "strict_mode": false,
  "had_fatal_error": false
}
```

- `analysis_mode` is `"diagnostic"` (dry-run) or `"mutating"`.
- `duplicates` lists entries skipped during merge due to exact match.
- `diagnostics` lists compatibility issues found in the source file.
- `normalizations` records field name case adjustments (e.g., `Description` → `description`).
- `detected_capabilities` lists features found in the source (e.g., `"toml_format"`, `"variables"`).

---

## Schema Versioning

When a breaking change is made to any JSON schema (field removed, type changed, semantics altered):

1. Increment the `schema` version number for that command's output.
2. Document the change in `CHANGELOG.md`.
3. Old schema versions continue to work for one major version cycle.

Non-breaking changes (new optional fields, new enum values) do not increment the schema version.
