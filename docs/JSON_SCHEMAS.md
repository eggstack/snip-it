# JSON Output Schemas

> **Evergreen contract.** Machine-readable JSON schemas for commands that
> support `--json` or `--report json`.
> `tests/schema.rs` enforces only the `list --json` and `list --csv` shapes;
> per-command emitters are listed in `architecture/outcome.md`.
>
> **Note:** Line numbers are approximate and may drift as the codebase evolves.

---

## Rules

1. All field names use `snake_case`.
2. Timestamps are Unix epoch milliseconds as integers (`*_unix_ms`). The snippet
   model's `created_at` / `updated_at` are epoch **seconds** integers, but no
   JSON envelope currently exposes them. **No command currently emits
   ISO 8601 / RFC 3339 strings.**
3. Snippet IDs are `legacy-<64 hex chars>` (deterministic SHA-256), not UUIDs.
   See [`IDENTITY_CONTRACT.md`](IDENTITY_CONTRACT.md).
4. Enum values serialize with Rust variant names (`PascalCase`) unless a
   `serde(rename)` overrides it. **Struct-variant enums serialize as
   objects** (`{"LiveExecution":{…}}`), not bare strings — see `sync.top_level`,
   `pending.state`, `execution_lock`, and `worker_lock` below.
5. Two envelopes hand-format their enum values instead of deriving `Serialize`,
   so they do **not** follow rule 4: `repair --json`'s `exit_status` and
   `action`, and `restore --json`'s `mode`. Their casing is listed with each
   envelope below.
6. Ordering is deterministic (sorted by ID or insertion order — never hash-map random).
7. New fields are additive (non-breaking).
8. Breaking changes increment the `schema` version number.
9. No ANSI escape sequences in any JSON output.
10. Secret values (API keys, passwords) are never included in JSON output.
11. **Optional-field caveat:** most optional fields serialize as explicit
    `null`, but some carry `#[serde(skip_serializing_if = "Option::is_none")]`
    and are **omitted** entirely — e.g. `CompatibilityDiagnostic.span`
    (`src/diagnostics.rs`). `StatusDiagnostic.remediation`
    (`src/status_snapshot.rs`) carries **no** such attribute, so it is
    serialized as an explicit `null`. Do not assume a key is always present;
    check the serde attribute on the struct.

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

- Items are ordered by the `--sort` mode (default `relevance`). Relevance ranks
  by the fuzzy match score when a `--filter` is given; with no filter there are
  no scores, so the order is the library's insertion order.
- `deleted` snippets are excluded (consistent with TUI display).
- `output` is always present but may be an empty string.
- No wrapping envelope — the output is a bare JSON array.

## Local MCP tool results

The local MCP adapter returns a JSON object inside both `structuredContent`
and a text content item. Successful snippet entries use these stable fields:
`id`, `library`, `description`, `command`, `tags`, `folders`, and `favorite`.
`snippets_list` and `snippets_search` wrap entries in `snippets`; a successful
`snippet_get` returns one entry directly. A failed `snippet_get` returns
`isError: true` with an `error` code (`"not_found"` or `"ambiguous"`) and a
`matches` array — the ambiguous entries carry only `id`, `library`, and
`description`, while `not_found` carries an empty `matches`. Credentials,
sync state, local output/notes, and execution are out of scope.

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
- `expanded` is the command with variables expanded (defaults or `--var`
  assignments) when the stored command has placeholders, and `null` when it has
  none. `--expanded` cannot be combined with `--json`; the JSON path always
  expands without prompting.
- `library` is the library filename stem; `library_id` is the **server-side**
  library ID and is an empty string for a library that is not sync-linked (and
  in legacy single-file mode).

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
  (`DiagnosticSeverity`, `src/diagnostics.rs:11-18`) — **capitalized**,
  not lowercase.
- `analysis_mode` is always `"diagnostic"`. The doctor's sub-modes
  (`--pet-file`, `--compatibility`, `--sync`, `--check-shell`, `--library`)
  select *what* is analysed; they do not change the value.
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
- Secrets are not stored in the config in the first place: the API key lives
  in the keychain. `sync.toml` is only included when `--include-sync-state` is
  passed, and its `api_key` values are redacted to `"<redacted>"`.

---

## `restore --json`

`snp restore --mode dry-run` emits a **different, manifest-focused** envelope
first (`src/commands/restore_cmd.rs:1094-1104`) and performs no writes, no lock,
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

- `action` is the Debug representation of the repair action — e.g.
  `"PruneOrphanedUsage"`, `"RemoveOrphanedArtifact"`, `"RepairLibraryIndex"`,
  `"RepairSnippetIds"`, `"RepairTimestamps"`, or a struct variant such as
  `"RollbackTransaction { transaction_id: \"…\" }"`.
- `category` groups the repair by the artifact it targets: `"config"`,
  `"primary"`, `"ids"`, `"timestamps"`, `"usage"`, `"unsafe"`, or
  `"transaction"`.
- `safe` indicates whether the repair is considered safe to apply automatically.
- `exit_status` is one of: `"clean"`, `"repaired"`, `"partial_failure"`,
  `"unsafe_only"`, `"findings_safe"`, `"dry_run"`.

---

## `import pet --report json`

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
      "entry_index": "number | null",
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

Machine modes (`--json`, `--csv`) are versioned contracts: field additions are
fine, renames and removals are not. See
[`.skills/cli-and-outcome-contract.md`](../.skills/cli-and-outcome-contract.md).

Two versioning styles are in use:

- `list --json`, `get --json`, and `status --json` carry a numeric `schema`
  field.
- `doctor --report json`, `validate --json`, and `import pet --report json`
  carry a semver string `schema_version` field (currently `"1.0.0"`).

When a breaking change is made to any JSON schema (field removed, type
changed, semantics altered):

1. Increment the version for that command's output.
2. Document the change in `CHANGELOG.md`.

Non-breaking changes (new optional fields) do not increment the version. New
enum *variants* are additive for the schema but break a consumer that matches
exhaustively, so treat them as a soft break.

---

## Related

- [architecture/outcome.md](../architecture/outcome.md) — the per-command emitter index.
- [`IDENTITY_CONTRACT.md`](IDENTITY_CONTRACT.md) — deterministic snippet IDs.
- [`COMPATIBILITY.md`](COMPATIBILITY.md) — the deprecation and versioning policy.
- [MCP.md](MCP.md) — the MCP tool contract over the same read-only surface.
