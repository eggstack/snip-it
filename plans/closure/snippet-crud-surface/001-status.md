# Snippet CRUD Surface Milestone 001 — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/snippet-crud-surface/001-non-interactive-snippet-delete.md`

Source subsystem roadmap:

- `plans/subsystems/snippet-crud-surface-roadmap.md`

Repository baseline reviewed: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Implementation commits or pull requests:

- Implementation commit `d67feeab52fa6f3442cf372dee16ed5f5061c036`, relative to baseline
  `dca5d31`. Change set: 1 new command module
  (`src/commands/delete_cmd.rs`), 1 extracted shared primitive, 1 new
  command surface (`snp delete` + `snp data delete`), 1 new deep dive
  (`architecture/commands/delete_cmd.md`), 10 new unit tests.

## 1. Executive finding

The milestone's user-visible capability boundary is complete and is
**demonstrated at runtime**, not only by compilation. A script can now
tombstone a snippet deterministically, with no TUI and no confirmation
prompt, and the persisted state it produces is byte-identical to what the
interactive TUI delete produces — proven by a test that drives both paths
against the same fixture and compares the persisted library files.

The delete sequence was extracted from the TUI selection loop into one shared
primitive, so the interactive path and the new command cannot drift.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| One shared delete primitive | `commands::delete_snippet(library_path, target_id, trigger)` (`src/commands/mod.rs:744`) performs the mutation gate, the local-data lock, the re-read inside the critical section, the lookup by identifier with `!deleted`, the tombstone, the durable save, the audit entry, and the sync trigger | pass | No picker logic: no display index, no in-memory mirror. |
| The TUI path calls the primitive | `apply_snippet_delete` (`src/commands/mod.rs:836`) calls the primitive, then performs the picker-specific in-memory mirror (`mark_snippet_deleted`) itself | pass | `src/ui/**` was not modified; the TUI's `d` key, hint, and layout are unchanged. |
| The persisted state is identical on both paths | `interactive_and_command_deletes_persist_identical_state` — both paths run against byte-identical fixtures (a far-future `updated_at` makes the tombstone deterministic), then the two persisted library files are compared byte for byte | pass | The non-target snippet is untouched in both. |
| The tombstone timestamp advances strictly | `delete_snippet_tombstones_with_a_strictly_advancing_timestamp` | pass | `updated_at = updated_at.max(now).saturating_add(1)`, unchanged from the interactive path, so a tombstone still beats live content in the client merge. |
| The identifier is unchanged by the delete | asserted in the same test | pass | The delete is a tombstone, not a removal. |
| An already-deleted target is reported distinctly | `delete_snippet_reports_an_already_deleted_target_distinctly`, `already_deleted_target_is_reported_distinctly_and_changes_nothing` | pass | The file is unchanged; the command exits 9. |
| A missing target is reported distinctly | `delete_snippet_reports_a_missing_target_distinctly`, `missing_target_is_reported_as_not_found_and_changes_nothing` | pass | The command exits 3. |
| An ambiguous selector deletes nothing | `ambiguous_selector_deletes_nothing_and_changes_no_file` | pass | The command exits 5. |
| The audit entry is written | `delete_snippet_writes_the_delete_audit_entry` | pass | The audit log contains a `delete` entry naming the target. |
| `command_behavior()` entries exist for both spellings | `src/main.rs` — `Commands::Delete` and `DataCommands::Delete` both classify as `(StartupRecoveryPolicy::Allow, StartupServices::Logging)` | pass | A mutating command must not be classified read-only, or file logging and audit would be suppressed. |
| The `snp data` mirror behaves identically | `snp data delete --id <ID>` produced the same JSON and the same tombstone as `snp delete` in the live run | pass | `cli_surface_compat` (18 tests) asserts the mirror table. |
| The MCP surface stays read-only | `src/mcp/**` was not modified | pass | No MCP tool was added; no MCP mutation or execution was introduced. |

## 3. Production implementation evidence

**New.** `src/commands/delete_cmd.rs` — the argument surface (`--id`,
`--description-exact`, `--command-exact`, `--query`/`-q`, `--library`/`-l`,
`--json`), the `run()` implementation, the JSON document, and 10 unit tests.

**Extracted.** `commands::delete_snippet` in `src/commands/mod.rs` — the
shared primitive described above. `src/commands/mod.rs` grew by 359 lines
net (primitive + TUI call site + tests).

**New command surface.** `snp delete` and its `snp data delete` mirror, with
`handle_delete` and the two `command_behavior()` arms in `src/main.rs`.

**Live behavior** (built binary `snp` v1.4.0, isolated `XDG_CONFIG_HOME`):

| Invocation | Exit | Result |
|---|---|---|
| `snp delete --id <ID> --json` | 0 | `{"schema":1,"status":"deleted","id":"legacy-4ca9…","description":"greeting two","library":"snippets"}`; the library file shows `deleted = true` and `updated_at` advanced `1792707879 → 1792708043` |
| `snp delete --id <ID> --json` (repeat) | 9 | `{"schema":1,"status":"already_deleted",…}`; the library file is unchanged |
| `snp delete --query "zzz-no-such-snippet-zzz"` | 3 | stderr: `Snippet not found: no live snippet matches the given selector.`; no stdout document |
| `snp delete --query "greeting"` (ambiguous) | 5 | stderr lists both matches and suggests `--id`; nothing was deleted |
| `snp data delete --id <ID> --json` | 0 | same JSON shape and same tombstone as the top-level spelling |

The audit log for the isolated config contains one `delete` row per
successful delete, naming the target identifier.

**Not implemented (other milestones).** The `folders` / `favorite` write path
is M002; non-interactive description / command / tag updates are M003. No
bulk delete, `--force`, confirmation prompt, or `--recursive` flag was added.

## 4. Verification executed

### Commands run

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --lib
cargo test --test cli_surface_compat
cargo test --test selector_integration
cargo test --test public_api_smoke
```

### Results

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass (exit 0, no output) |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass (0 errors, 0 warnings) |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | pass (0 errors, 0 warnings) |
| `cargo test --workspace --lib` | pass — snip-it 1211 passed / 0 failed / 8 ignored (includes the 10 new delete tests); snip-sync 187 passed / 0 failed |
| `cargo test --test cli_surface_compat` | pass — 18 passed / 0 failed |
| `cargo test --test selector_integration` | pass — 23 passed / 0 failed |
| `cargo test --test public_api_smoke` | pass — 3 passed / 0 failed |

Plus the live runs recorded in §3 (exit codes 0 / 3 / 5 / 9, the JSON
documents, the tombstone transition, the audit rows, and the `snp data`
mirror).

**Scope note.** Verification was scoped to the targets this change can
affect. The full `scripts/check.sh` suite was not run in this pass; it
remains the pre-merge gate.

## 5. Invariant review

| Invariant | Evidence | Result |
|---|---|---|
| The delete is a tombstone, not a removal | The identifier is preserved; only `deleted` and `updated_at` change | pass |
| A tombstone beats live content in a merge | `updated_at = updated_at.max(now).saturating_add(1)` — the pre-existing interactive rule, reused unchanged | pass |
| Every local mutation is gated | The primitive reaches the store through `library::update_library`, which runs the mutation gate and takes the local-data lock | pass |
| The library is re-read inside the critical section | The primitive inherits this from `library::update_library`; it does not carry a picker copy | pass |
| The selector's ambiguity rules protect against deleting the wrong target | `ambiguous_selector_deletes_nothing_and_changes_no_file`; live exit 5 | pass |
| Read-only paths must not mutate | The command uses the **mutating** `resolve_selector`; the read-only resolver is not used | pass |
| The MCP boundary stays read-only | `src/mcp/**` untouched; no MCP tool added | pass |
| The merge order, the synced field set, the fingerprint, and the identifier derivation are unchanged | The change set touches no sync or identity code | pass |
| The persisted schema is unchanged | The tombstone reuses the existing `deleted` and `updated_at` fields | pass |
| Exit codes and the stdout/stderr split follow the contract | 0 / 3 / 5 / 9 (all pre-existing codes); the JSON document goes to stdout, diagnostics to stderr | pass |
| The `snp data` mirror is a mirror, not a second implementation | One `handle_delete`; both spellings share the `command_behavior()` classification | pass |

## 6. Failure and recovery review

- **Duplicate delivery / idempotency** — deleting an already-deleted
  snippet is a no-op reported distinctly (exit 9); the file is not rewritten.
- **Cancellation races** — the mutation is a single critical section; a
  cancellation before the commit leaves the file unchanged.
- **Restart** — no new state; the tombstone is ordinary persisted content.
- **Partial persistence failure** — the store's own atomic write path is
  reused; a failed write leaves the previous file intact.
- **Stale generation or lease** — the library generation counter is bumped
  by the store, as for every other mutation.
- **Contention and resource release** — the local-data lock is taken and
  released by the store's mutation path.
- **Malformed or unauthorized input** — an unparsable selector is a usage
  error (exit 2); a selector that matches nothing is exit 3; a selector that
  matches several is exit 5. None of these paths writes.
- **Bounded artifact behavior** — one tombstone per invocation; no bulk
  operation was introduced.

## 7. Migration and compatibility review

- **Schema migration** — none required; the tombstone uses the existing
  `deleted` flag and `updated_at` field.
- **Backward compatibility** — the new command is additive. No existing
  command spelling, alias, exit code, or output format changed. The TUI's `d`
  key, its hint, and the layout are unchanged.
- **Protocol negotiation** — not applicable (the command is local-only).
- **Configuration behavior** — unchanged.
- **Rollback limitations** — rolling back removes the command and the mirror
  and restores the inlined TUI delete; no persisted state depends on the
  command's existence, so the rollback is lossless.
- **Legacy-path status** — the legacy single-file library migration is
  untouched.

## 8. Security review

- **Authorization enforcement** — unchanged; the CLI is single-user and
  local.
- **Secret handling** — the target snippet's command text is not echoed to
  the audit log (the audit row carries the identifier; the command field is
  redacted in the row format). No credential is introduced.
- **Path validation** — the library path is resolved by the existing store
  path resolution; no new path handling was introduced.
- **Privilege boundaries** — unchanged.
- **Denial-of-service bounds** — the command performs exactly one mutation
  and exits; it cannot be turned into a bulk operation. The selector's
  ambiguity check prevents an accidental mass delete.
- **Redaction** — the JSON document carries only the identifier, the
  description, and the library stem — never the command text or the output
  field, which may contain secrets. This was a deliberate choice for the
  machine-readable surface.
- **Audit behavior** — a `delete` audit row is written per successful
  delete; an audit failure is logged at debug level and does not fail the
  delete (pre-existing behavior, reused unchanged).

## 9. Documentation and operations

New deep dive: `architecture/commands/delete_cmd.md` (235 lines).
Updated: `architecture/cli.md` (the dispatch chain and the `*Args` list),
`architecture/overview.md` (the layer map row and the deep-dive index row),
`docs/COMMANDS.md` (the everyday-command row, a new "Deleting a snippet"
section, and the `snp data delete` mirror row), `docs/COMMAND_CONTRACTS.md`
(the `delete` contract row and the mirror sentence),
`docs/JSON_SCHEMAS.md` (the `delete --json` shape), `docs/USER_GUIDE.md`
("Scripting with `snp delete`"), `docs/LOGICAL_LAYERS.md` (the
`src/commands/` module count, 26 → 27), `AGENTS.md` (the `snp data` mirror
group now lists six spellings), and `CHANGELOG.md` under `### Added`.

`docs/EXIT_CODES.md` needed no change: 0, 3, 5, and 9 all already exist.

Static guard: none was required — `tests/architecture.rs` scans path prefixes
only and no constant list enumerates command modules. The command's presence
in the `snp data` mirror is asserted by the existing `cli_surface_compat`
suite.

Operator diagnostics: the new command's diagnostics (not found, ambiguous,
already deleted) are listed in `docs/COMMANDS.md` and `docs/EXIT_CODES.md`.

Recovery instructions: none. A delete is an ordinary tombstone and is
reverted by restoring a backup snapshot, exactly as any other content change.

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| low | The plan's §6 describes the race case as a distinct **not-found** outcome, while the milestone contract requires a distinct **already-deleted** outcome. The implementation follows the contract: the primitive reports `AlreadyDeleted` and the command maps it to exit 9, keeping it distinct from not-found (exit 3). | Contract wording conflict, resolved in favour of the more precise outcome. Both outcomes are distinct and separately tested. | None. The plan's §6 wording should not be read as authoritative. |
| low | The plan's §5 says the target may be named "by … a filter". `--filter` is the TUI's fuzzy *pre-filter*, not a selector input, and there is no TUI here. The implementation uses the `get` family's convention (`--query`/`-q`) for fuzzy selection. | Deliberate deviation; the selector surface is consistent with the other non-TUI commands. | None. |
| low | The plan's §3 states the `snp data` group has five variants; it now has six. `AGENTS.md`'s note about the mirrored commands was updated accordingly. | Documentation drift, corrected. | None. |
| low | `src/main.rs` needed a `handle_delete` function for consistency with the other five mirrored commands, which the plan did not explicitly require. | Mechanical, consistent with the existing mirror-group shape. | None. |

## 11. Roadmap disposition

Milestone closed; the next dependency may proceed. M002 (the `folders` /
`favorite` write path) and M003 (non-interactive field updates) are now
unblocked and remain to be planned.

## 12. Registry updates

- `plans/registry.md`: the snippet-crud-surface roadmap moves from `ready` to
  `active`; this milestone moves from `not started` to `closed`; the plan row
  moves from `ready` to `closed`.
- `plans/subsystems/snippet-crud-surface-roadmap.md`: §12 milestone status
  table updated — M001 `closed`, with the closure record linked; the
  milestone's completion note records the `--filter` → `--query` deviation.
