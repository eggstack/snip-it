# Documentation Authoring Skill

## Purpose
Guide agents through changing documentation in this workspace without
creating drift. The repo carries four documentation layers plus a planning
layer; each layer has a distinct job, and the failure mode is the same in
every one of them — **two documents claiming the same fact, with one of them
eventually wrong**.

---

## The layers, and who owns what

| Layer | Location | Owns | Audience |
|-------|----------|------|----------|
| Agent entry point | `AGENTS.md` | The traps that will burn you. Short, dense, linked. | Agents, first 60 seconds |
| Task skills | `.skills/*.md` | "How do I do X" for one subsystem, with the rules that task needs. | Agents, mid-task |
| Deep dives | `architecture/*.md` | How a component actually works, with `file:line` citations. 61 deep dives (62 documents including the overview). | Reviewers and agents doing real work |
| Contracts | `docs/*.md` | The operator/user-facing contract: commands, exit codes, schemas, threat model. | Users and operators |
| Planning | `plans/**` | Intent, roadmap, ADRs, handoff and closure records. | Humans deciding what to build |

**The single-authoritative-source rule.** A fact has exactly one home. Every
other layer links to it. In particular:

- A behavior is described in depth **once**, in `architecture/`. `AGENTS.md`,
  `.skills/`, and `docs/` link to it.
- An exit code has one table, `docs/EXIT_CODES.md`. Nothing else re-lists the
  mapping.
- A command has one contract row, `docs/COMMAND_CONTRACTS.md`.
- The layer lists in `tests/architecture.rs` and the tables in
  `docs/LOGICAL_LAYERS.md` are a **coupled pair** — change both together.

If you find yourself restating a deep dive inside a skill, replace the
restatement with a link. That is a fix, not a loss.

---

## Citation discipline

`architecture/` and `.skills/` cite source as `path/to/file.rs:LINE` or
`file.rs:START-END`. These are **auditable claims**, not decoration. A reader
must be able to open the file and find the cited code.

Rules:

1. **Every citation must be verified** against the current tree before you
   commit. Line numbers drift silently when unrelated code is added above.
2. **A range must contain the thing you claim it contains.** `:120-140` is not
   a citation for "the function at line 130" if the function moved.
3. **A blank line is a broken citation.** It usually means the cited code was
   deleted or moved.
4. **Cite the real path.** A vendored crate path (e.g. a `ratatui` source path)
   is not a citation into this repo — either drop it or say plainly that it is
   an upstream path.

### The mechanical check

Line drift is the dominant defect class and is exactly what a human misses
when re-reading their own prose. Validate mechanically instead of by eye:

```bash
# scans every markdown doc in .skills/, docs/, plans/, architecture/ plus the
# root documents; reports out-of-range, bad-range, and blank-target citations
python3 /tmp/citation_check.py
```

Treat every reported line as a real finding until proven otherwise. Fix the
document (the code is the truth), not the citation.

The same discipline applies to **links**: a relative link to a file or a
`#anchor` that no longer resolves is a broken contract. Renaming a doc means
updating every inbound link, including `docs/README.md` and
`architecture/overview.md`.

---

## `AGENTS.md` is an index, not a manual

`AGENTS.md` is the file every agent reads first. Its job is to get the
reader to the right place, not to be the place.

- **Keep the traps.** A surprising invariant that will corrupt data or break
  a build belongs in `AGENTS.md` even if it is explained at length elsewhere.
- **Cut the explanation.** If a bullet needs four sentences to state a rule
  that `architecture/overview.md` explains in a section, replace the four
  sentences with one line plus a link to that section's anchor.
- **Never let it become a second copy of a deep dive.** Duplication is how
  two documents diverge.
- **The Skills table is the index into `.skills/`.** When you add or remove a
  skill, update that table in the same commit.

---

## Adding a new command

A new subcommand touches more than the dispatch arm. The full set:

1. **`src/main.rs`** — the `Commands` variant and its Clap schema. The group
   commands (`sync_cmd`, `import_cmd`, `library_cmd`, `premade_cmd`,
   `shell_cmd`) keep their schema in `src/main.rs` rather than in
   `src/commands/`; the other modules define a canonical `*Args` struct.
2. **`src/main.rs:887`** — a `command_behavior()` arm. Every new subcommand
   needs one; it declares the startup-recovery policy (whether the command
   triggers interrupted-transaction recovery before it runs).
3. **`src/outcome.rs`** — a `CliOutcome` variant and its exit-code mapping,
   if the command needs a new outcome. Reusing an existing variant is
   strongly preferred: the exit-code table is a published contract.
4. **`docs/COMMAND_CONTRACTS.md`** — a row for the new command (stdout/stderr
   split, exit codes, startup-recovery policy).
5. **`docs/COMMANDS.md`** — the user-facing reference entry.
6. **`docs/EXIT_CODES.md`** — only if a new exit code was introduced.
7. **`architecture/commands/<name>.md`** — the deep dive, if the command has
   non-trivial behavior.
8. **`architecture/overview.md`** — the index row linking to that deep dive.

A `snp data <x>` spelling additionally needs a `DataCommands` variant, a
dispatch arm, and a `command_behavior()` arm. `snp data` is a **mirror
group**, not a Clap `alias` — the five data spellings (`validate`, `backup`,
`restore`, `repair`, `status`) are the only commands with single-path
`handle_*` functions in `src/main.rs`.

---

## Archive, do not silently delete

`docs/README.md` splits the reference docs into **current contracts** and
**archived snapshots** under `docs/archive/`. Archived docs describe code that
no longer exists.

- An archived doc is **never cited as current**. If a fact lives only in
  `docs/archive/`, it is not a contract — either restore it to `docs/` with a
  correction, or treat it as historical.
- **An archived doc's `file:line` citations are intentionally dead.** They
  record the pre-split layout; the archive banner says so explicitly. Do not
  "repair" them — repairing them falsifies the historical record. A mechanical
  citation sweep will report them; treat `docs/archive/**` findings as
  expected.
- When superseding a doc, move it to `docs/archive/` and add a status line
  saying what superseded it and when. Preserve the original content — the
  archive is traceability, not a rubbish bin.
- **Pruning** a doc means it had no reader and no traceability value (e.g. a
  plan for work that never started, duplicated in the archive). Record the
  removal in `CHANGELOG.md`.
- Before archiving, check whether the doc's *subject* still exists. A doc
  about a removed subsystem should be archived; a doc about a live subsystem
  should be corrected.

---

## `CHANGELOG.md`

The repo keeps an active `## [Unreleased]` section with `### Added`,
`### Changed`, `### Fixed`, and `### Documentation` subsections. Add an entry
in the same commit as the change, under the subsection that matches. A
documentation-only change still gets a `### Documentation` entry.

**Version bumps and the changelog go together in one PR** — but only for a
release. A docs-only change does not bump the version.

---

## Before you commit

```bash
bash scripts/check.sh                              # the authoritative gate
python3 /tmp/citation_check.py                     # citations
```

Then, for a documentation change:

1. Every `file:line` you touched is verified against the current source.
2. Every link and anchor you touched resolves.
3. Any doc you added or removed is reflected in its index (`docs/README.md`,
   `architecture/overview.md`, the `AGENTS.md` skills table).
4. `CHANGELOG.md` has an entry under `## [Unreleased]`.
5. You did **not** change source code to make a doc true. If the doc was
   wrong, fix the doc. If the code is wrong, that is a separate change.

---

## Common mistakes

- Restating a deep dive inside a skill or inside `AGENTS.md` instead of
  linking to it.
- Adding a command and forgetting the `command_behavior()` arm or the
  `docs/COMMAND_CONTRACTS.md` row.
- Citing a line number from memory instead of re-reading the file.
- Moving or renaming a doc and not updating the inbound links.
- Treating an archived doc as a current contract.
- Archiving a doc instead of correcting it when the subject still exists.
- Changing the code to match a doc instead of changing the doc.
