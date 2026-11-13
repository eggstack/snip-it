# organize_cmd — Deterministic Folder and Favorite Updates

[← Back to Overview](../overview.md)

## Overview

`src/commands/organize_cmd.rs` updates a snippet's local-only `folders` and
`favorite` fields without a TUI. It resolves its target through the
**mutating** selector (`selector::resolve_selector`) and applies the shared
field-update primitive (`commands::update_snippet_fields`).

## CLI surface

`OrganizeArgs` (`organize_cmd.rs`), `snp organize`:

| Flag | Type | Meaning |
|------|------|---------|
| `--id` | `Option<String>` | Exact snippet ID (case-sensitive) |
| `--description-exact` | `Option<String>` | Exact description (case-insensitive) |
| `--command-exact` | `Option<String>` | Exact command text (case-insensitive) |
| `-q/--query` | `Option<String>` | Fuzzy query |
| `-l/--library` | `Option<String>` | One library name; `all` is refused |
| `--favorite` | bool | Set the favorite flag |
| `--unfavorite` | bool | Clear the favorite flag |
| `--add-folder <NAME>` | `Option<String>` | Add a folder membership |
| `--remove-folder <NAME>` | `Option<String>` | Remove a folder membership |
| `--json` | bool | Pretty `OrganizeJsonOutput` |

Exactly one selector and exactly one action flag are required. Folder names
must be non-empty after trimming and must not contain `;` (CSV ambiguity).

`OrganizeJsonOutput` is `{schema, status, id, description, library, favorite,
folders}` with `schema: 1` and `status` ∈ {`updated`, `unchanged`}.

## The shared primitive

`commands::update_snippet_fields` (`src/commands/mod.rs`) mirrors
`delete_snippet`: mutation gate, local-data lock, re-read under the lock,
lookup by identifier with `!deleted`, durable save, best-effort audit, and
auto-sync notification after commit.

No-ops (re-setting the current favorite, adding an existing folder, removing
an absent folder) return `SnippetFieldUpdateOutcome::Unchanged` **without**
rewriting the library file.

Audit action names: `favorite`, `unfavorite`, `add-folder`, `remove-folder`.

## Outcomes and exit codes

| Situation | stdout | stderr | Exit |
|-----------|--------|--------|------|
| Updated / unchanged | result document | – | 0 |
| Ambiguous selector | – | candidates | 5 |
| Not found | – | diagnostic | 3 |
| Usage (selector, action, folder name) | – | diagnostic | 2 |
| Operational failure | – | diagnostic | 1 |

There is no exit 9 analogue: an unchanged end state succeeds with
`status: "unchanged"`.

## Related modules

- `src/commands/mod.rs` — `update_snippet_fields`, `SnippetFieldMutation`
- `src/selector.rs` — mutating resolution
- `src/library/model.rs` — `folders` / `favorite` serde rules
- `docs/JSON_SCHEMAS.md` — `--json` envelope
