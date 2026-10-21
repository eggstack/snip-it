# Interim Planning Archive

This directory retains completed, superseded, rejected, or abandoned
interim planning for traceability.

The archive is not an active work queue. Agents MUST use
`plans/registry.md` and current subsystem roadmaps to identify executable
work.

## What belongs here

- completed milestone implementation plans after closure;
- superseded subsystem roadmaps;
- corrective plans whose closure is complete;
- rejected interim proposals worth retaining for historical context;
- status documents no longer needed in active directories.

## What does not belong here

- canonical long-term specification, terminology, roadmap, or planning
  governance;
- accepted ADRs;
- active subsystem roadmaps;
- ready or active implementation plans;
- unresolved closure records.

## Archive layout

The archive holds **pointers and retired direction documents**, not the bodies
of closed milestones:

```text
archive/
    flat-000-...-020-...md (pre-convention sequential plans)
```

The `flat-000-*.md` file is the pre-convention direction plan retained in full
(archival note prepended). The `flat-001`-`flat-020` files are predecessor
pointers: their full bodies were promoted via `git mv` into
`plans/implementation/<subsystem>/` (history preserved), so the stubs record
the replacement subsystem milestone without duplicating content.

Closed milestone bodies stay in place as the record of record:
`plans/implementation/<subsystem>/NNN-*.md` remains the implementation plan
and `plans/closure/<subsystem>/NNN-status.md` remains the closure evidence.
Only superseded or retired direction documents move here.

When moving a document into the archive:

1. update inbound links from `plans/registry.md` and the current subsystem
   roadmap;
2. add a short archival note to the document stating its final status and
   replacement, if any;
3. preserve Git history through a move rather than recreating unrelated
   content where possible;
4. do not rewrite historical conclusions to match later implementation;
5. ensure active documents link to the replacement or later milestone.

Archived plans may be useful evidence, but they are not authoritative over
current canonical documents, accepted ADRs, active subsystem roadmaps, or
current repository behavior.
