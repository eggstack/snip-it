# JSON Output Schemas

> Phase 08A — Workstream H
> Machine-readable JSON schemas for commands that support `--json` or `--report json`.

---

## Rules

1. All field names use `snake_case`.
2. Optional fields use explicit `null` (never omitted).
3. Timestamps use ISO 8601 / RFC 3339 format (e.g., `"2026-01-15T10:30:00Z"`).
4. UUIDs use standard hyphenated format (e.g., `"550e8400-e29b-41d4-a716-446655440000"`).
5. Ordering is deterministic (sorted by ID or insertion order — never hash-map random).
6. New fields are additive (non-breaking).
7. Breaking changes increment the `schema` version number.
8. No ANSI escape sequences in any JSON output.
9. Secret values (API keys, passwords) are never included in JSON output.

---

## `list --json`

```json
{
  "schema": 1,
  "items": [
    {
      "id": "string",
      "description": "string",
      "command": "string",
      "output": "string | null",
      "tags": ["string"],
      "folders": ["string"],
      "favorite": false,
      "deleted": false,
      "created_at": "2026-01-15T10:30:00Z",
      "updated_at": "2026-01-15T10:30:00Z"
    }
  ]
}
```

- Items are sorted by `updated_at` descending (matching `save_library` sort order).
- `deleted` snippets are excluded (consistent with TUI display).
- `output` is always present but may be `null` or empty string.

---

## `get --json`

```json
{
  "schema": 1,
  "id": "string",
  "description": "string",
  "command": "string",
  "expanded": "string",
  "tags": ["string"],
  "library": "string | null",
  "library_id": "string | null"
}
```

- `command` is the raw template (with `<var>` placeholders).
- `expanded` is the fully expanded command (variables replaced with defaults or prompt values).
- `library` is the library name; `library_id` is the library filename stem.

---

## `status --json`

```json
{
  "schema": 1,
  "top_level": "string",
  "sync_configured": true,
  "sync_direction": "string | null",
  "pending_generation": "number | null",
  "pending_age_secs": "number | null",
  "last_attempt_result": "string | null",
  "last_attempt_time": "string | null",
  "backoff_until": "string | null",
  "worker_active": false,
  "executor_active": false,
  "diagnostics": [
    {
      "severity": "string",
      "message": "string"
    }
  ]
}
```

- `top_level` is one of: `"healthy"`, `"sync_disabled"`, `"pending"`, `"backing_off"`, `"failed"`.
- `sync_direction` is one of: `"push"`, `"pull"`, `"bidirectional"`, or `null`.
- Timestamps are ISO 8601; durations are in seconds.

---

## `doctor --report json`

```json
{
  "schema": 1,
  "mode": "string",
  "file": "string",
  "entries": [
    {
      "severity": "string",
      "message": "string",
      "details": "string | null"
    }
  ],
  "summary": {
    "total": 0,
    "errors": 0,
    "warnings": 0,
    "info": 0
  }
}
```

- `severity` is one of: `"error"`, `"warning"`, `"info"`.
- `mode` reflects the doctor sub-mode: `"compatibility"`, `"sync"`, `"check-shell"`, `"library"`, `"pet-file"`.
- `summary` provides aggregate counts for quick scripting.

---

## `validate --json`

```json
{
  "schema": 1,
  "items": [
    {
      "severity": "string",
      "message": "string",
      "file": "string | null",
      "snippet_id": "string | null"
    }
  ]
}
```

- `severity` is one of: `"error"`, `"warning"`, `"info"`.
- `file` is the path to the file containing the issue (may be relative).
- `snippet_id` identifies the specific snippet when applicable.

---

## `backup --json`

```json
{
  "schema": 1,
  "backup_id": "string",
  "timestamp": "2026-01-15T10:30:00Z",
  "files": [
    {
      "path": "string",
      "sha256": "string",
      "size_bytes": 0
    }
  ],
  "total_files": 0,
  "total_bytes": 0
}
```

- `backup_id` is a UUID.
- `sha256` is the hex-encoded SHA-256 checksum of the file content.
- Backup files exclude secrets (API keys, passwords are redacted).

---

## `restore --json`

```json
{
  "schema": 1,
  "restore_id": "string",
  "timestamp": "2026-01-15T10:30:00Z",
  "files_restored": 0,
  "files_skipped": 0,
  "dry_run": false,
  "mode": "string",
  "details": [
    {
      "path": "string",
      "action": "string",
      "reason": "string | null"
    }
  ]
}
```

- `action` is one of: `"restored"`, `"skipped"`, `"conflict"`.
- `mode` is one of: `"dry-run"`, `"merge"`, `"replace"`.

---

## `repair --json`

```json
{
  "schema": 1,
  "items": [
    {
      "severity": "string",
      "message": "string",
      "file": "string | null",
      "action": "string | null"
    }
  ],
  "backups_created": 0,
  "repairs_applied": 0,
  "dry_run": true
}
```

- `action` describes the repair action taken or proposed: `"quarantine"`, `"recreate"`, `"fix"`.
- `dry_run` indicates whether repairs were actually applied.

---

## `import --report json`

```json
{
  "schema": 1,
  "source": "string",
  "destination": "string",
  "total_entries": 0,
  "imported": 0,
  "skipped": 0,
  "merged": 0,
  "errors": [
    {
      "index": 0,
      "description": "string",
      "reason": "string"
    }
  ],
  "dry_run": false
}
```

- `source` and `destination` are file paths.
- `skipped` counts entries that already exist (exact match).
- `merged` counts entries that were combined with existing snippets.
- `errors` lists entries that could not be imported.

---

## Schema Versioning

When a breaking change is made to any JSON schema (field removed, type changed, semantics altered):

1. Increment the `schema` version number for that command's output.
2. Document the change in `CHANGELOG.md`.
3. Old schema versions continue to work for one major version cycle.

Non-breaking changes (new optional fields, new enum values) do not increment the schema version.
