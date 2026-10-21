# Compatibility and Deprecation Policy

> **Evergreen policy.** Rules for maintaining backward compatibility across
> releases. Enforced in part by `tests/cli_surface_compat.rs`.
>
> The CLI surface rules below restate — and must not diverge from —
> [`.skills/cli-and-outcome-contract.md`](../.skills/cli-and-outcome-contract.md).

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
2. A warning is emitted to stderr, in the form
   `warning: --old-flag is deprecated, use --new-flag instead`.
3. The deprecation is documented in `CHANGELOG.md`.

### Phase 2: Removal

After at least one minor version:

1. The deprecated item is removed.
2. Using it produces a clear error message pointing to the replacement.
3. The removal is documented as a breaking change in `CHANGELOG.md`.

---

## Exit Code Changes

- Exit codes 0-10 are stable public contract (see [`EXIT_CODES.md`](EXIT_CODES.md)).
- New exit codes are additive (non-breaking).
- Changing the meaning of an existing exit code is a breaking change — reusing
  a number for a new meaning is breaking even when the number is unchanged.
- Internal auto-sync worker codes are not part of the public contract.
- `CliOutcome::ExecutionFailed { child_code }` propagates the child's exit
  code verbatim, so a process exit code is **not** confined to 0-10.

---

## Stream Changes (stdout/stderr)

- Moving human-readable output from stdout to stderr is a **breaking change** for scripts parsing stdout.
- Changing the stdout format of a default (non-machine) mode is a **soft
  break**: it is acceptable, but `docs/COMMAND_CONTRACTS.md` and
  `docs/USER_GUIDE.md` must be updated in the same change.
- Machine modes (`--json`, `--csv`) are versioned contracts — see
  [`JSON_SCHEMAS.md`](JSON_SCHEMAS.md).
- When making a stream change, document the migration in the release notes.

---

## JSON Schema Changes

- New optional fields are non-breaking (additive).
- Removing a field or changing its type is a breaking change — increment the
  schema version.
- See [`JSON_SCHEMAS.md`](JSON_SCHEMAS.md) for schema versioning rules.

---

## `select` Shell Integration

The `snp select` command is specifically designed for shell integration. Its contract must be preserved:

```bash
# This pattern must always work:
command=$(snp select -f "deploy") && eval "$command"
```

- `snp select` outputs the command string to stdout on success.
- Exit code 0 on success, 4 on cancellation.
- `--expanded` expands `<var>` placeholders before printing; `--raw` (the
  default) prints the stored command. They are mutually exclusive.
- `--output-file` writes to a file instead of stdout.
- No TUI rendering on the output stream (TUI is on the terminal, output is on stdout).

---

## Migration Examples

### From `snp get` with implicit search to deterministic `get`

Before (ambiguous — a fuzzy match may hit several snippets):
```bash
snp get --query "git"          # fuzzy; exits 5 (AMBIGUOUS) on multiple matches
```

After (deterministic):
```bash
snp get --description-exact "git"   # exact description match
snp get --id legacy-<64-hex>        # exact match by ID (see IDENTITY_CONTRACT.md)
```

`snp get` takes no positional argument and has no `--exact` flag; the
selectors are `--id`, `--description-exact`, `--command-exact`, and `--query`.
Use `--resolution first` or `--resolution all` to opt out of the unique-match
requirement. `--resolution` takes `unique` (default), `first`, or `all`.

### From `snp list` stdout parsing to `--json`

Before (fragile — ANSI escapes in stdout):
```bash
snp list | grep "deploy"
```

After (robust):
```bash
snp list --json | jq '.[] | select(.description | contains("deploy"))'
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
| `update` | — | No alias |
| `mcp` | — | No alias |

Aliases are removed only with a major version bump and explicit deprecation notice.

---

## The `snp data` Mirror

`snp data` is a **mirror group**, not an alias for a single command. It
re-spells five top-level commands and dispatches to the same handlers, so the
two spellings must stay behaviorally identical:

| `snp data` subcommand | Top-level equivalent | `snp data` alias |
|------------------------|----------------------|-------------------|
| `validate` | `snp validate` | `v` |
| `backup` | `snp backup` | `b` |
| `restore` | `snp restore` | — |
| `repair` | `snp repair` | — |
| `status` | `snp status` | `s` |

Note that `v` is the alias for `snp data validate` and is **unrelated** to the
top-level `v` alias for `snp version`. The two live in different namespaces.

Both spellings share one Clap schema per command (`ValidateArgs`,
`BackupArgs`, `RestoreArgs`, `RepairArgs`, `StatusArgs`), so a flag added to
one is automatically added to the other. Equivalence is checked by
`tests/cli_surface_compat.rs`.

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

---

## Related

- [`EXIT_CODES.md`](EXIT_CODES.md) — the exit-code table.
- [`JSON_SCHEMAS.md`](JSON_SCHEMAS.md) — the JSON envelopes and their versioning rules.
- [`COMMAND_CONTRACTS.md`](COMMAND_CONTRACTS.md) — per-command stdout/stderr and startup-recovery contracts.
- [`.skills/cli-and-outcome-contract.md`](../.skills/cli-and-outcome-contract.md) — the authoritative CLI-surface rules.
- `tests/cli_surface_compat.rs` — the `snp data` mirror and alias equivalence checks.
