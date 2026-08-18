# Compatibility and Deprecation Policy

> Phase 08A — Workstream L
> Rules for maintaining backward compatibility across releases.

---

## Principles

1. **Existing aliases are preserved** where harmless — removing an alias is a breaking change.
2. **Warn before removal** — deprecated flags/subcommands emit a warning for at least one minor version before removal.
3. **No silent repurposing** — existing flags are never reused for different semantics. New flags get new names.
4. **`select` behavior is preserved** for shell integration — `snp select -f "query"` continues to output the command to stdout.
5. **Semantic versioning** governs breaking changes — major version bump for incompatible changes.

---

## Deprecation Process

### Phase 1: Deprecation Warning

When a flag, subcommand, or behavior is deprecated:

1. The old behavior continues to work.
2. A warning is emitted to stderr: `warning: --old-flag is deprecated, use --new-flag instead`.
3. The deprecation is documented in `CHANGELOG.md`.

### Phase 2: Removal

After at least one minor version:

1. The deprecated item is removed.
2. Using it produces a clear error message pointing to the replacement.
3. The removal is documented as a breaking change in `CHANGELOG.md`.

---

## Exit Code Changes

- Exit codes 0-9 are stable public contract (see `EXIT_CODES.md`).
- New exit codes are additive (non-breaking).
- Changing the meaning of an existing exit code is a breaking change.
- Internal auto-sync worker codes are not part of the public contract.

---

## Stream Changes (stdout/stderr)

- Moving human-readable output from stdout to stderr is a **breaking change** for scripts parsing stdout.
- When making stream changes:
  1. Introduce a `--stdout` transitional flag to restore old behavior.
  2. Deprecate `--stdout` after one minor version.
  3. Document the migration in release notes.

---

## JSON Schema Changes

- New optional fields are non-breaking (additive).
- Removing a field or changing its type is a breaking change — increment `schema` version.
- See `JSON_SCHEMAS.md` for schema versioning rules.

---

## `select` Shell Integration

The `snp select` command is specifically designed for shell integration. Its contract must be preserved:

```bash
# This pattern must always work:
command=$(snp select -f "deploy") && eval "$command"
```

- `snp select` outputs the command string to stdout on success.
- Exit code 0 on success, 4 on cancellation.
- `--raw` and `--expanded` flags control output format.
- `--output-file` writes to a file instead of stdout.
- No TUI rendering on the output stream (TUI is on the terminal, output is on stdout).

---

## Migration Examples

### From `snp get` with implicit search to deterministic `get`

Before (ambiguous):
```bash
snp get "git"          # may match multiple snippets
```

After (deterministic):
```bash
snp get --id <uuid>    # exact match by ID
snp get "git" --exact  # exact description match
```

### From `snp list` stdout parsing to `--json`

Before (fragile — ANSI escapes in stdout):
```bash
snp list | grep "deploy"
```

After (robust):
```bash
snp list --json | jq '.items[] | select(.description | contains("deploy"))'
```

---

## Alias Preservation

| Command | Alias | Status |
|---------|-------|--------|
| `version` | `v` | Stable |
| `new` | `n` | Stable |
| `list` | `l` | Stable |
| `run` | `r` | Stable |
| `clip` | `c` | Stable |
| `search` | `s` | Stable |
| `select` | `sel` | Stable |
| `edit` | `e` | Stable |
| `keybindings` | `k` | Stable |
| `sync` | `y` | Stable |
| `cron` | `cr` | Stable |
| `register` | `reg` | Stable |
| `library` | `lib` | Stable |
| `premade` | `p` | Stable |
| `import` | `i` | Stable |
| `repair` | `rp` | Stable |
| `validate` | `val` | Stable |
| `data` | `d` | Stable |
| `completions` | `g` | Stable |
| `get` | — | No alias |
| `doctor` | — | No alias |
| `backup` | — | No alias |
| `restore` | — | No alias |
| `shell` | — | No alias |
| `status` | — | No alias |

Aliases are removed only with a major version bump and explicit deprecation notice.

---

## Breaking Change Checklist

Before merging a breaking change:

- [ ] Major version bump planned
- [ ] `CHANGELOG.md` updated with migration guide
- [ ] Deprecated items had warning period (at least one minor version)
- [ ] No silent repurposing of existing flags/aliases
- [ ] Exit code contract preserved (or new codes are additive)
- [ ] JSON schema version incremented (if applicable)
- [ ] Shell integration patterns tested (`snp select` pipeline)
- [ ] `--help` text updated to reflect changes
