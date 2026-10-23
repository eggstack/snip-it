# Maintenance and Duplicate-Policy Consolidation Milestone 003 — Diagnostic vocabulary unification

Status: ready

Repository baseline: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Source roadmap: [`plans/subsystems/maintenance-consolidation-roadmap.md` § M003 — Diagnostic vocabulary unification](../../subsystems/maintenance-consolidation-roadmap.md#m003--diagnostic-vocabulary-unification)

Long-term requirements:

- [`plans/000-long-term-specification.md` § 2. Primary product goals](../../000-long-term-specification.md#2-primary-product-goals)
- [`plans/000-long-term-specification.md` § 3. Non-goals](../../000-long-term-specification.md#3-non-goals)
- [`plans/000-long-term-specification.md` § 4. Crate and module ownership](../../000-long-term-specification.md#4-crate-and-module-ownership)
- [`plans/000-long-term-specification.md` § 10. Verification and evidence rules](../../000-long-term-specification.md#10-verification-and-evidence-rules)
- [`plans/001-terminology-and-domain-model.md` § 4. Library and persistence terms](../../001-terminology-and-domain-model.md#4-library-and-persistence-terms)
- [`plans/002-long-term-roadmap.md` § Cross-phase execution rules](../../002-long-term-roadmap.md#cross-phase-execution-rules)
- [`plans/003-planning-process.md`](../../003-planning-process.md)

Applicable ADRs: none

Primary class: infrastructure

---

## 1. Objective

Give the `validate`, `doctor`, and `repair` surfaces **one shared diagnostic vocabulary** and **one
shared library scan**, so that the same repository state is described the same way wherever it is
inspected.

The deliverable is three shared types in `src/diagnostics.rs` — a severity, a code, and a
repairability — plus one library-layer scan that the three surfaces consume instead of
re-deriving it. **Each surface keeps its own report type and its own output format.** No
diagnostic code, message, exit code, or JSON output shape changes.

## 2. Why this milestone is ready

- M001 is closed (`plans/closure/maintenance-consolidation/001-status.md`), and M002 is planned
  (`plans/implementation/maintenance-consolidation/002-backup-and-snapshot-policy-convergence.md`).
  M003's dependencies are "soft M001 (documentation context), hard none"
  (roadmap § 7), and it does not touch the backup or copy paths M002 owns — the two milestones
  share no file: M002 owns `src/utils/atomic.rs` and the backup call sites; M003 owns
  `src/diagnostics.rs`, `src/library/manager.rs`, and the three command modules' *diagnostic*
  code.
- The duplication is fully reproducible from the current tree: three severity vocabularies, one
  free-form code namespace enforced only by a literal list in a unit test, two library-file walks
  inside `repair_cmd`, and a code namespace that the documentation already describes as one table
  while the code enforces nothing.
- The shared classification input already exists in the library layer
  (`LibraryManager::inspect_library_index`, `src/library/manager.rs:344`), so this milestone
  extends an existing seam rather than inventing one.

## 3. Current implementation evidence

### 3.1 The three surfaces and their vocabularies

| Surface | Severity | Code | Repairability | Report type | Exit |
|---------|----------|------|---------------|-------------|------|
| `snp validate` | **local** `Severity` — `src/commands/validate_cmd.rs:32` | `String` — `src/commands/validate_cmd.rs:49`; `E-`/`W-`/`I-` screaming-case | **local** `Repairability` — `src/commands/validate_cmd.rs:40` (`Auto`/`Manual`/`Unrepairable`) | `ValidationReport` — `src/commands/validate_cmd.rs:60` | `CliOutcome::ValidationFailed` → exit 6 — `src/commands/validate_cmd.rs:732-733` |
| `snp doctor` | shared `crate::diagnostics::DiagnosticSeverity` — `src/diagnostics.rs:11` | `String` — `src/diagnostics.rs:30`; `compat.*` / `sync.*` dotted plus `E-`/`W-`/`I-` from `src/commands/pet_analysis.rs` | **none** | `DoctorReport` — `src/diagnostics.rs:108` | `CliOutcome::ValidationFailed` → exit 6 — `src/commands/doctor_cmd.rs:1314` |
| `snp repair` | **none** | **none** — `RepairItem.category: String` (`src/commands/repair_cmd.rs:132`) plus `RepairAction` (`:36`) | `RepairItem.safe: bool` (`:138`) plus `RepairAction::is_safe()` (`:98`) | `RepairReport` and a **hand-built** `JsonReport` — `src/commands/repair_cmd.rs:1002-1009` | `RepairExitStatus` (`:146`) → 0 / 1 / 10 |

So `validate` owns two local enums that duplicate the shared `DiagnosticSeverity`; `doctor`
already uses the shared severity but keeps its code a free-form `String`; `repair` has neither a
severity nor a code, only a per-finding `category` string and a `safe` boolean.

### 3.2 The code namespace is documented as one table and enforced as nothing

`architecture/diagnostics.md:307-318` publishes a single **Code-Prefix Table** with five forms
(`E-`, `W-`, `I-`, `sync.*`, `compat.*`) and states their meanings. Enforcement, however, is a
unit test over twelve hard-coded literals (`src/diagnostics.rs:342-365`), and
`architecture/diagnostics.md:418-419` says so plainly: "Code-prefix enforcement is only a unit
test (`diagnostics.rs:342`), not a typed constraint; a new consumer can introduce any code
string."

A closed enum of codes is **not feasible**: doctor builds some codes at runtime —
`format!("compat.shell.{shell_name}.ok")` (`src/commands/doctor_cmd.rs:676`) and the catch-all
`format!("sync.{}", other.to_lowercase())` (`src/commands/doctor_cmd.rs:133`). The namespace must
therefore be a **validating newtype**, not an enum.

### 3.3 The shared library scan — what exists, and what repair duplicates

The shared views live in the **library layer**, not in any command:

- `LibraryManager::resolve_readonly_sources` (`src/library/manager.rs:295`) is "the single
  canonical implementation for side-effect-free library discovery" (`:286-288`): it never calls
  `ensure_library_mode`, never creates files, never rewrites metadata. `Some("all")` enumerates
  **index entries only** (`:325`).
- `LibraryManager::inspect_library_index` (`src/library/manager.rs:344`) is "side-effect-free"
  (`:338-343`; `architecture/diagnostics.md:391-392`) and returns
  `LibraryIndexInspection` (`src/library/model.rs:212`) with `missing_files` (`:214`),
  `orphan_files` (`:216`), and `primary` (`:218`). It performs exactly **one** directory listing
  (`:355`).

Current consumers:

| Surface | Index-driven sources | Index inspection | Own directory walk |
|---------|----------------------|------------------|--------------------|
| validate | `readonly_library_sources(Some("all"))` — `src/commands/validate_cmd.rs:687` | `inspect_library_index()` — `:375` | none |
| doctor | none | `inspect_library_index().primary` — `src/commands/doctor_cmd.rs:563` | none |
| repair | none | `inspect_library_index().primary` — `src/commands/repair_cmd.rs:392` | **two**: `fs::read_dir(&libraries_dir)` at `src/commands/repair_cmd.rs:304` and again at `:857` inside `collect_active_snippet_ids` (called from `:671`) |

**Premise correction.** The roadmap's M003 entry says the repair surface re-scans "instead of
reusing the validate surface's library scan". The duplication is real, but the **owner is wrong**:
the shared scan is the library layer's, not validate's. `src/commands/validate_cmd.rs:370-373`
says the view is shared by "`validate`, `repair`, `doctor`, and `status`", and
`architecture/commands/doctor_cmd.md:69` says the same: "What they *do* share is the
classification input — `LibraryManager::inspect_library_index()` and
`crate::library::find_orphaned_ids`". So the plan must not build a "validate scan"; it must
extend the library layer's scan and make repair consume it.

**Second premise correction — the migration is not a drop-in.** Repair's walk is
**disk-truth**: every `*.toml` in `libraries/`, orphans included
(`src/commands/repair_cmd.rs:304-309`). `resolve_readonly_sources(Some("all"))` is
**index-driven** (`src/library/manager.rs:325`). Pointing repair at the index-driven resolver
alone would change what repair enumerates. The shared scan must be a **superset** — indexed
sources **plus** orphan files — so the migration is behavior-preserving. (Repair does not detect
orphan *library* files today; its orphan detection is about orphan *usage* entries, via
`crate::library::find_orphaned_ids` at `src/commands/repair_cmd.rs:443`. The shared scan must not
silently add or remove findings.)

### 3.4 The repair surface has two competing category identities, already drifting

`RepairItem.category: String` (`src/commands/repair_cmd.rs:132`) and
`RepairAction::category() -> &'static str` (`:81`) are two different representations of a
category, and they **disagree** at three call sites:

| Site | `action` | emitted `category` | `RepairAction::category()` |
|------|----------|--------------------|--------------------------|
| `src/commands/repair_cmd.rs:275-276` | `RepairLibraryIndex` | `"config"` | `"index"` (`:91`) |
| `src/commands/repair_cmd.rs:396` | `RepairLibraryIndex` | `"primary"` | `"index"` (`:91`) |
| `src/commands/repair_cmd.rs:471` | `RollbackTransaction { .. }` | `"unsafe"` | `"transaction"` (`:85`) |

The **emitted** value is the drifted literal; the typed one is unused at these sites. Note that
the two are genuinely different axes — `category` is a *per-finding* label while
`RepairAction::category()` is a *per-action* label — so `category` **cannot** simply be derived
from `action`. The shared vocabulary must name the two axes differently instead of letting one
name mean two things.

### 3.5 Two pre-existing code collapses that M003 must preserve

The doctor sync mapping (`src/commands/doctor_cmd.rs:70-133`) collapses two distinct snapshot
conditions onto one code each:

- `EXECUTION_LOCK_INACCESSIBLE` → `sync.execution.malformed` (`:93-96`), the same code as
  `EXECUTION_LOCK_MALFORMED` (`:89-92`);
- `WORKER_LOCK_INACCESSIBLE` → `sync.worker_lock.malformed` (`:105-108`), the same code as
  `WORKER_LOCK_MALFORMED` (`:101-104`).

The roadmap forbids collapsing distinct diagnostics *to simplify the vocabulary*, and M003's
deliverable boundary forbids changing any diagnostic code. Both collapses are therefore
**preserved verbatim** and recorded, not fixed.

### 3.6 The exit condition names a spelling that does not exist

The roadmap's M003 exit condition requires that "`snp data validate`, `snp data repair`, and
`snp data doctor` still mirror their top-level spellings". The `DataCommands` group in
`src/main.rs` has exactly six variants — Validate, Backup, Restore, Repair, Status, Delete — and
**there is no `doctor`**: grepping the whole tree, the only occurrence of "data doctor" is the
roadmap text itself. `docs/COMMAND_CONTRACTS.md:44` confirms the mirror group is those six, and
`:103` even notes "there is no `snp data import`".

**Premise correction.** The exit condition's third clause is **vacuous**. M003 must not add
`snp data doctor`: a new `snp data` spelling needs three edits (a `DataCommands` variant, a
dispatch arm, and a `command_behavior()` arm) plus a `docs/COMMAND_CONTRACTS.md` row, and M003
forbids CLI-surface changes. The correct reading is "the mirror spellings that exist must keep
mirroring" — `snp data validate` and `snp data repair` in particular.

### 3.7 Adjacent severity vocabularies outside the three surfaces

Two more severity enums exist: `src/status_snapshot.rs:131` and `src/utils/variables.rs:16`.
`src/utils/variables.rs:11-13` says explicitly that "the diagnostic vocabulary is unified in a
later milestone, so the type stays until then", and the type is `#[allow(dead_code)]`.
`status_snapshot`'s severity is consumed by doctor through `map_snapshot_diagnostic`
(`src/commands/doctor_cmd.rs:70`). These are outside M003's three-surface scope (§ 5) but the
shared vocabulary must be *usable* by them without forcing a migration.

### 3.8 Two stale documentation references in the files this milestone touches

`architecture/diagnostics.md:70`, `:378` and `architecture/library.md:408` cite
`src/library/manager.rs:310` for `inspect_library_index`; the actual definition is
`src/library/manager.rs:344`. `architecture/diagnostics.md:378` cites
`src/library/model.rs:123` for the view types; `PrimaryState` is at `src/library/model.rs:194`
and `LibraryIndexInspection` at `:212`.

### 3.9 A roadmap / closure-record discrepancy

`plans/closure/maintenance-consolidation/001-status.md:226-227` names "M003 (the `Durability`
taxonomy and the remaining suppression candidates)", while the roadmap's § 7 defines M003 as
diagnostic vocabulary unification. The roadmap is authoritative; the closure record's parenthetical
is a slip and is not a source for this plan's scope.

## 4. Invariants that must not regress

1. **No diagnostic code, message, or exit code changes.** Every code string emitted today
   (`E-`/`W-`/`I-` codes, `compat.*` and `sync.*` codes, and repair's `category` strings) is
   emitted byte-identically after this milestone.
2. **JSON output shapes are unchanged.** `ValidationReport` (`src/commands/validate_cmd.rs:60`),
   `DoctorReport` (`src/diagnostics.rs:108`), and repair's hand-built `JsonReport`
   (`src/commands/repair_cmd.rs:1002-1009`) keep exactly the same fields. A new shared type must
   be `#[serde(transparent)]` (or otherwise shape-preserving) wherever it replaces a `String`.
3. **Each surface keeps its own report type and output format.** The shared vocabulary supplies
   *types*, not envelopes. `src/diagnostics.rs` stays serde-only with no I/O and no imports from
   `commands`/`ui`/`logging` (`architecture/diagnostics.md:382`).
4. **Read-only paths stay read-only.** `snp validate` and `snp doctor` must not mutate. The
   shared library scan performs existence checks and a directory listing only — it never creates,
   migrates, or rewrites library state (`architecture/diagnostics.md:391-392`).
5. **The canonical read-only resolver stays canonical.** The shared scan builds on
   `resolve_readonly_sources` (`src/library/manager.rs:295`) and `inspect_library_index` (`:344`);
   it must not introduce a second resolver, and it must not call `ensure_library_mode`
   (`src/library/manager.rs:287`).
6. **The scan count must not increase.** The shared scan *replaces* repair's two directory walks
   (`src/commands/repair_cmd.rs:304`, `:857`); it must not add a third (roadmap § 8,
   "Performance and resource use").
7. **Exact-count assertion discipline is preserved.** A unified vocabulary must not merge two
   distinct diagnostics into one row (roadmap § 8, "Observability and audit").
8. **Exit codes are unchanged.** `snp validate` exits 0/1/6, `snp doctor` exits 0/1/6, `snp repair`
   exits 0/1/10 (`docs/COMMAND_CONTRACTS.md:24`, `:25`, `:28`; `docs/EXIT_CODES.md:102`).
9. **The `snp data` mirror group is unchanged.** `snp data validate` and `snp data repair` keep
   mirroring their top-level spellings; no `DataCommands` variant, dispatch arm, or
   `command_behavior()` arm is added or removed.
10. **No new command, flag, or `command_behavior()` entry.** `docs/COMMAND_CONTRACTS.md` gains no
    row.
11. **Search parity and the selector are untouched.** This milestone adds no searchable field and
    changes no resolver (`.skills/selector-and-search-parity.md`).
12. **The mutation gate and the atomic write layer are untouched.** `snp repair --apply` still
    mutates only through the existing gated paths; M003 changes no write, no lock, and no
    durability class.

## 5. Scope

### In scope

- Three shared types in `src/diagnostics.rs`: the **severity** (reusing the existing
  `DiagnosticSeverity`, `src/diagnostics.rs:11`), a new **code** type, and a new **repairability**
  type.
- `snp validate` adopts the shared severity, code, and repairability; its local `Severity`
  (`src/commands/validate_cmd.rs:32`) and `Repairability` (`:40`) are deleted.
- `snp doctor` adopts the shared code type; `CompatibilityDiagnostic.code`
  (`src/diagnostics.rs:30`) stops being a free-form `String`.
- `snp repair` adopts the shared severity, code, and repairability **internally**, without
  changing its report type, its JSON, or its emitted `category` strings.
- One shared library scan in the library layer, consumed by all three surfaces, replacing repair's
  two directory walks.
- A **diagnostic-vocabulary agreement test** (roadmap § 9): the validate, doctor, and repair
  surfaces must agree on severity and code for a shared fixture.
- Documentation (§ 12) and a `CHANGELOG.md` entry recording the refactor as behavior-preserving.

### Explicitly out of scope

- **`snp status` / `src/status_snapshot.rs`.** The status surface has its own severity enum
  (`:131`) and its own `StatusDiagnostic` (`:123`). It is not one of M003's three surfaces, and
  migrating it would change the status snapshot's JSON shape. The shared types must be usable by
  it later without a breaking change.
- **`src/utils/variables.rs`'s `DiagnosticSeverity` (`:16`).** It is `#[allow(dead_code)]` and
  its own comment (`:11-13`) defers it to "a later milestone". It is not one of M003's three
  surfaces.
- **The pet-file / import surface** (`src/commands/pet_analysis.rs`,
  `src/diagnostics.rs`'s `PetImportReport`). Doctor *reuses* `pet_analysis`'s codes, but the
  import consumer keeps its own report envelope (`architecture/diagnostics.md:334-335`).
- **Fixing the two code collapses in § 3.5.** They are preserved (§ 4.1, § 4.7).
- **Adding `snp data doctor`** (§ 3.6).
- **Merging two diagnostics into one code**, even where the strings differ only in case. The
  roadmap's risk section forbids it without preserving the string identity.
- **Changing any message text, exit code, report envelope, or output format.**
- **The MCP surface** (`src/mcp/`). It is read-only and non-executing on the `serve` path and has
  its own JSON contract; it emits no library diagnostics.

## 6. Required production changes

| Area | Change |
|------|--------|
| Core/domain | `src/diagnostics.rs` — keep `DiagnosticSeverity` (`:11`) as the one severity vocabulary and generalize its doc comments (they currently say "compatibility diagnostics" / "cannot be imported safely", `:9`, `:17`); add the shared **code** type and the shared **repairability** type. The module stays serde-only with no I/O (`architecture/diagnostics.md:382`) and must not import from `crate::commands`, `crate::ui`, or `crate::logging` (`docs/LOGICAL_LAYERS.md`, core enforced set). |
| Core/library | `src/library/manager.rs` — add the shared library scan next to `inspect_library_index` (`:344`) and `resolve_readonly_sources` (`:295`), performing **one** directory listing and returning the union of index-driven sources, orphan files, missing files, and primary state. `src/library/model.rs` — the scan's result type, beside `LibraryIndexInspection` (`:212`) and `PrimaryState` (`:150`). |
| validate | `src/commands/validate_cmd.rs` — delete the local `Severity` (`:32`) and `Repairability` (`:40`); `ValidationDiagnostic` (`:48`) uses the shared severity (`:50`), the shared code (`:49`), and the shared repairability (`:55`); `diags()` (`:111`) takes the shared types; `STRICT_CODES` (`:708-713`) becomes a typed list. Every code string is unchanged. |
| doctor | `src/diagnostics.rs` — `CompatibilityDiagnostic.code` (`:30`) becomes the shared code type, `#[serde(transparent)]` so the JSON is unchanged. `src/commands/doctor_cmd.rs` — every `code:` literal becomes a shared code value; the runtime-built codes (`:676`, `:133`) use the validating constructor; `STRICT_WARNING_CODES` (`:53-63`) becomes a typed list; `map_snapshot_diagnostic` (`:70`) returns the shared code type. The two collapses in § 3.5 are preserved verbatim. |
| repair | `src/commands/repair_cmd.rs` — `RepairItem` (`:128`) gains internal severity / repairability / code fields derived from `RepairAction` (`:36`); `RepairAction` gains `severity()`, `repairability()`, and `code()` accessors and a distinctly named finding-category accessor so the two axes in § 3.4 stop sharing a name. **The emitted `category` strings and the JSON shape are unchanged**, because the JSON is hand-built in `JsonRepairItem` (`:992-999`). `collect_repair_candidates` (`:270`) and `collect_active_snippet_ids` (`:847`) consume the shared library scan instead of their own `fs::read_dir` walks (`:304`, `:857`). |
| Runtime | The shared scan is side-effect-free and takes no locks. It replaces repair's two walks with one, so the total number of library directory listings does not increase (roadmap § 8). `snp repair` still mutates only through its existing gated paths. |
| Operator surface | No new command, flag, or spelling. No `command_behavior()` entry, no `docs/COMMAND_CONTRACTS.md` row, no `snp data` change. |
| Docs and guards | Update § 12. Add a static guard that rejects a *new* free-form `code: String` in the three command modules and the shared diagnostics module, and a test that asserts the emitted code set is unchanged. |

## 7. Ordered work packages

### Work package A — Add the shared diagnostic vocabulary to `src/diagnostics.rs`

**Required changes**

- `src/diagnostics.rs`:
  - **Severity** — keep `DiagnosticSeverity` (`:11`) exactly as it is (variants `Info`, `Warning`,
    `Error`; `Copy`, `PartialEq`, `Eq`, `Serialize`, `Deserialize`). Only its doc comments change,
    so that they describe the shared use rather than the import use. Its serde representation
    (`"Info"`, `"Warning"`, `"Error"`) must not change.
  - **Repairability** — add
    `pub enum DiagnosticRepairability { Auto, Manual, Unrepairable }` with
    `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]`. The variant names and
    the serde representation must match `src/commands/validate_cmd.rs:40-44` exactly, so that
    validate's JSON is unchanged when it adopts the type.
  - **Code** — add
    `pub struct DiagnosticCode(String)` with `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]`
    and `#[serde(transparent)]`, plus:
    - `pub fn new(raw: impl Into<String>) -> SnipResult<Self>` — validates the namespace and
      returns a typed error naming the offending string;
    - `pub fn as_str(&self) -> &str`;
    - `pub fn severity(&self) -> Option<DiagnosticSeverity>` — `Some` for an `E-`/`W-`/`I-` prefix
      (the prefix implies the severity), `None` for the dotted forms (doctor assigns severity
      explicitly);
    - `pub fn is_strict_elevatable(&self) -> bool` — `true` for a `W-` prefix.
  - The namespace grammar is the one `architecture/diagnostics.md:307-318` already publishes:
    - `E-…`, `W-…`, `I-…` followed by a screaming-case body (`[A-Z0-9-]+`);
    - `compat.<area>.<detail>` — dotted, at least two segments;
    - `sync.<area>.<detail>` — dotted, at least two segments.
    The grammar **must** accept the runtime-built forms
    (`format!("compat.shell.{shell_name}.ok")`, `src/commands/doctor_cmd.rs:676`, and the
    catch-all `format!("sync.{}", other.to_lowercase())`, `:133`) — the `sync.` fallback means the
    detail segment may be any lowercase token.
  - The module stays dependency-free apart from serde and `crate::error`
    (`architecture/diagnostics.md:382`); `DiagnosticCode::new` must not require a filesystem, a
    config directory, or a runtime.

**Acceptance evidence**

- Unit tests in `src/diagnostics.rs`: the grammar accepts every code string the three surfaces
  emit today (enumerated from the source), rejects a malformed one (lowercase `e-cmd-empty`, an
  unknown prefix, an empty body), `severity()` maps `E-`/`W-`/`I-` correctly, `DiagnosticCode`
  serializes to the same JSON string as the `String` it replaces, and `DiagnosticRepairability`
  serializes to `"Auto"` / `"Manual"` / `"Unrepairable"`.
- `cargo clippy --workspace --all-targets -- -D warnings` is clean.

### Work package B — `snp validate` adopts the shared vocabulary

**Required changes**

- `src/commands/validate_cmd.rs`:
  - delete the local `Severity` (`:32-36`) and `Repairability` (`:40-44`);
  - `ValidationDiagnostic` (`:48`) takes the shared types: `code: DiagnosticCode` (`:49`),
    `severity: DiagnosticSeverity` (`:50`), `repairability: DiagnosticRepairability` (`:55`);
  - `diags()` (`:111`) takes the shared types and constructs the code with
    `DiagnosticCode::new(...)`;
  - the strict elevation list `STRICT_CODES` (`:708-713`) becomes `&[DiagnosticCode]`, and the
    comparison at `:715` uses `DiagnosticCode` equality;
  - every code string literal is preserved verbatim, including the test-only `E-TEST`, `I-TEST`,
    `W-TEST`, and `W-OTHER`.
- `ValidationReport` (`:60-68`) keeps exactly its fields.

**Acceptance evidence**

- A golden JSON comparison: `snp validate --json` on a fixture with at least one error, one
  warning, and one info produces the **same JSON** as before the change, field for field and
  value for value.
- The validate code set is unchanged: `E-CMD-EMPTY`, `E-DUP-ID`, `E-FILE-READ`,
  `E-INDEX-MISSING-FILE`, `E-PRIMARY-MISSING`, `E-TOML-PARSE`, `I-FILE-EMPTY`, `W-CORRUPT-BAK`,
  `W-DESC-EMPTY`, `W-DUP-CHECK-SKIPPED`, `W-EXACT-DUP`, `W-ID-EMPTY`, `W-INSECURE-PERMS`,
  `W-LIB-INDEX-UNREADABLE`, `W-LIB-UNREADABLE`, `W-NO-PRIMARY`, `W-ORPHAN-FILE`,
  `W-SAME-ID-DIVERGENT`, `W-USAGE-ORPHAN`; `snp validate --strict` still exits 6 and elevates
  the same four (`W-ID-EMPTY`, `W-DESC-EMPTY`, `W-SAME-ID-DIVERGENT`, `W-EXACT-DUP`).

### Work package C — `snp doctor` adopts the shared code type

**Required changes**

- `src/diagnostics.rs`: `CompatibilityDiagnostic.code` (`:30`) becomes `DiagnosticCode`. The
  struct's field order and the other fields are unchanged, so the JSON shape is unchanged.
- `src/commands/doctor_cmd.rs`:
  - every `code: "…".to_string()` literal becomes a shared code value; the runtime-built codes
    (`:676`, `:686`, `:850`, `:890`, `:901`, `:916`, `:1091`, `:1126`, `:1144`, `:1157`, `:1187`,
    `:1204`, `:1215`) use the validating constructor;
  - `map_snapshot_diagnostic` (`:70-133`) returns the shared code type and preserves the two
    collapses in § 3.5 **verbatim**;
  - `STRICT_WARNING_CODES` (`:53-63`) becomes a typed list, keeping the same nine codes in the
    same order;
  - `apply_strict_elevation` (`src/commands/doctor_cmd.rs:187`) compares shared codes.
- `src/commands/doctor_report.rs` renders the shared code unchanged.

**Acceptance evidence**

- A golden JSON comparison: `snp doctor --library <name> --report json` produces the **same
  JSON** as before the change.
- The doctor code set is unchanged: the `compat.*` and `sync.*` literals, plus the
  `pet_analysis` codes (`E-CMD-EMPTY`, `I-CHOICE-VARS`, `I-FIELD-UNKNOWN`, `I-OUTPUT-PRESENT`,
  `I-TAGS-EMPTY`, `W-CMD-MISSING`, `W-DESC-EMPTY`, `W-DESC-MISSING`, `W-DUP-CMD`,
  `W-DUP-DESC`, `W-MALFORMED-VAR`, `W-TAG-EMPTY`, `W-TYPE-MISMATCH`); the strict-elevation set
  is unchanged and `snp doctor --strict` still exits 6 on the same fixtures.

### Work package D — `snp repair` adopts the shared vocabulary

**Required changes**

- `src/commands/repair_cmd.rs`:
  - `RepairItem` (`:128`) gains three internal fields — `severity: DiagnosticSeverity`,
    `repairability: DiagnosticRepairability`, `code: DiagnosticCode` — all derived from
    `action`. They are **not** serialized, because the JSON is hand-built in `JsonRepairItem`
    (`:992-999`); the plan must require that struct to keep exactly its six fields
    (`action`, `category`, `transaction_id`, `problem`, `fix`, `safe`).
  - `RepairAction` (`:36`) gains `severity()`, `repairability()`, and `code()` accessors, plus a
    **distinctly named** finding-category accessor so that the per-finding axis (`:132`) and the
    per-action axis (`:81`) stop sharing the word "category".
  - `RepairItem.category` (`:132`) is populated from the finding-category accessor and keeps
    **exactly** its current strings — `"config"`, `"ids"`, `"timestamps"`, `"primary"`,
    `"transaction"`, `"unsafe"`, `"usage"` — so no emitted value changes. The three drift sites
    in § 3.4 are preserved and documented, not silently "fixed", because fixing them would change
    the output.
  - `safe` (`:138`) is derived from the shared repairability: `Auto` ⇒ `safe: true`,
    `Manual`/`Unrepairable` ⇒ `safe: false`, matching `RepairAction::is_safe()` (`:98-110`)
    today. The plan must require a test that the two agree for every variant, so the shared
    repairability cannot drift from the existing safety policy.
- The human renderer (`src/commands/repair_cmd.rs:933-987`) and the JSON renderer (`:990-1049`)
  print the same fields in the same order.

**Acceptance evidence**

- Golden comparisons: `snp repair --json` produces the **same JSON** and `snp repair --dry-run`
  the same human output as before the change; `snp repair` still exits 0 / 1 / 10 on the same
  fixtures.
- A test that `RepairAction::is_safe()` and the shared repairability agree for **every** variant.

### Work package E — One shared library scan

**Required changes**

- `src/library/manager.rs`: add the shared scan beside `inspect_library_index` (`:344`):
  ```rust
  pub fn scan_libraries(&self) -> LibraryScan
  ```
  `LibraryScan` (in `src/library/model.rs`) carries:
  - `sources: Vec<ResolvedLibrarySource>` — index-driven, exactly what
    `resolve_readonly_sources(Some("all"))` returns (`src/library/manager.rs:325`);
  - `orphan_files: Vec<PathBuf>` — the same list `inspect_library_index` computes
    (`:353-372`);
  - `missing_files: Vec<(String, PathBuf)>` — the same list (`:345-351`);
  - `primary: PrimaryState` — the same value (`:374`).
  The scan performs **one** directory listing (the one `inspect_library_index` already does at
  `:355`) and reuses it for both `sources` and `orphan_files`.
- `src/library/model.rs`: `pub struct LibraryScan { … }`, beside `LibraryIndexInspection` (`:212`).
- `src/commands/repair_cmd.rs`:
  - `collect_repair_candidates` (`:270`) obtains its `library_files` from the scan instead of
    `fs::read_dir` (`:304-309`);
  - `collect_active_snippet_ids` (`:847`) obtains the same list from the scan instead of its own
    `fs::read_dir` (`:857`);
  - the "all libraries" branch (`:299-310`) keeps its current semantics — every `*.toml` in
    `libraries/`, orphans included — which the scan's `sources ∪ orphan_files` reproduces
    exactly;
  - the "named library" branch (`:289-298`) keeps its current error message
    (`"Library not found"`, `:294-297`).
- `src/commands/validate_cmd.rs` (`:687`) and `src/commands/doctor_cmd.rs` (`:563`) consume the
  same scan, so all three surfaces enumerate the same library set.

**Acceptance evidence**

- `snp repair --dry-run` and `snp validate` enumerate the **same** library set on a fixture
  containing an orphan library file (present in `libraries/`, absent from the index).
- The total number of library-directory listings does not increase: the scan performs exactly
  one listing.
- `snp validate --json` and `snp doctor --library <name> --report json` produce unchanged JSON on
  a fixture with an orphan library file.

### Work package F — The diagnostic-vocabulary agreement test

**Required changes**

- Add a test that runs the three surfaces against one shared fixture and asserts they agree on
  severity and code for the conditions they actually share:
  - `E-CMD-EMPTY` (severity `Error`) and `W-DESC-EMPTY` (severity `Warning`) are emitted by
    **both** `snp validate` (`src/commands/validate_cmd.rs`) and `snp doctor --library <name>`
    (via `src/commands/pet_analysis.rs`) today — the strings already agree, and the test pins
    that agreement through the shared types;
  - the primary-library state is classified identically by all three (they already share
    `inspect_library_index().primary`: `src/commands/validate_cmd.rs:375`,
    `src/commands/repair_cmd.rs:392`, `src/commands/doctor_cmd.rs:563`);
  - the orphan-usage classification is shared by validate and repair through
    `crate::library::find_orphaned_ids` (`src/commands/repair_cmd.rs:443`).
- The test must assert the **exact code strings and severities**, not a set relationship, so that
  a future divergence fails loudly.
- The test must **not** require the three surfaces to emit the same *set* of findings — they have
  different scopes by design (validate checks the whole repository, doctor analyzes one file or
  the environment, repair proposes actions).

**Acceptance evidence**

- The agreement test passes on the current code and would fail if any shared code's severity
  diverged between the surfaces; it is included in the focused run in § 11.

### Work package G — Static guard, documentation, and changelog

**Required changes**

- Add a static guard (a focused test, or a `tests/architecture.rs` extension) that fails when a
  **new** free-form `code: String` field appears in `src/diagnostics.rs`,
  `src/commands/validate_cmd.rs`, `src/commands/doctor_cmd.rs`, or `src/commands/repair_cmd.rs`.
- Update the documents listed in § 12, including the two stale references in § 3.8.
- Add a `CHANGELOG.md` entry recording this as a **behavior-preserving refactor**: no code,
  message, exit code, or JSON shape changed.

**Acceptance evidence**

- `bash scripts/check.sh` passes; the static guard fails when a `code: String` field is
  reintroduced (proved during implementation, not in the merge commit); the emitted-code-set test
  passes with the pre-milestone code list.

## 8. Failure, cancellation, restart, and contention semantics

**A malformed code string.** `DiagnosticCode::new` rejects it and returns a typed error naming
the offending string. The surfaces must not panic and must not silently accept it: a rejected
code is a programming error in the producing module, so the surface returns an error (exit 1)
rather than emitting an out-of-namespace code. This is the behavior change with the widest blast
radius, and it is why the grammar must accept every string the surfaces build today — including
the runtime-built `compat.shell.*`, `compat.shell_syntax.*`, `compat.shell_init.*`, and the
catch-all `sync.*` forms.

**A library file that disappears between the scan and the load.** The scan enumerates; the
consumer loads. A file removed in between makes the load fail. Today repair's own walk has the
same window (`src/commands/repair_cmd.rs:304` then `:318`). The shared scan does not change that
window, and the surfaces must keep treating a load failure as a finding (validate:
`W-LIB-UNREADABLE` / `E-FILE-READ`) rather than as a crash.

**A library file that appears between the scan and the load.** The scan's `orphan_files` list is
a snapshot; a file created afterwards is simply not in it. No surface may write during a scan —
the scan is side-effect-free (§ 4.4).

**Cancellation.** `snp validate` and `snp doctor` are read-only and short-lived; `snp repair`
mutates only in `--apply` mode, after `collect_repair_candidates` has finished. The scan runs
before any mutation, so a cancellation during the scan leaves the repository untouched. This
ordering must not change.

**Restart.** None of the three surfaces keeps state between runs; the shared scan holds no state
beyond the call. `snp repair`'s transaction behavior is untouched by this milestone.

**Contention.** The scan takes no locks and performs one directory listing. The listing is a
point-in-time view; a concurrent writer can change the set between the scan and the load, which
is the same exposure the current code has. The scan count must not increase (roadmap § 8), so the
plan adds no lock and no second listing.

**A code that no longer matches its prefix.** `DiagnosticCode::severity()` reports the severity
implied by an `E-`/`W-`/`I-` prefix. The surfaces assign severity explicitly, so a mismatch
between the prefix and the assigned severity is possible in principle. The milestone must **not**
turn that into a runtime error — it would change which findings are reported. The mismatch is
surfaced only by a test that asserts the emitted set is unchanged.

## 9. Compatibility and migration

- **No diagnostic code, message, or exit code changes.** Every emitted string is identical.
- **No JSON output shape changes.** `DiagnosticCode` is `#[serde(transparent)]`;
  `DiagnosticSeverity` and `DiagnosticRepairability` keep their variant names; repair's
  `JsonRepairItem` (`:992-999`) keeps exactly its six fields.
- **No command, flag, spelling, `command_behavior()` entry, or `docs/COMMAND_CONTRACTS.md` row is
  added.** The `snp data` mirror group keeps its six spellings; `snp data validate` and
  `snp data repair` keep mirroring.
- **No exit code changes.** `snp validate` 0/1/6, `snp doctor` 0/1/6, `snp repair` 0/1/10.
- **No migration.** Nothing on disk is rewritten, and no persisted artifact changes.
- **Public API impact.** `crate::diagnostics` is a crate-root module; the new types are additive.
  The deleted `src::commands::validate_cmd::Severity` and `::Repairability` are crate-internal
  (`src/commands/validate_cmd.rs:32`, `:40`), not part of the published surface — but
  `tests/public_api_smoke.rs` and `tests/architecture.rs` must be re-checked for references to
  them before the change lands.
- **Rollback.** Reverting the code restores the previous types; the emitted output was identical
  in both directions, so there is nothing to migrate back.

## 10. Required tests

### Focused unit tests

- `src/diagnostics.rs`: the grammar, `severity()`, `is_strict_elevatable()`, serde-transparency,
  and the repairability serde names (work package A's acceptance evidence).
- `src/library/manager.rs` (or `src/library/tests.rs`): the shared scan returns the same
  `sources`, `orphan_files`, `missing_files`, and `primary` that `resolve_readonly_sources` and
  `inspect_library_index` return today, over a fixture with an orphan library file, a missing
  indexed library, and no primary set.
- `src/commands/repair_cmd.rs`: `RepairAction::is_safe()` and the shared repairability agree for
  every variant.

### Integration tests

- **The diagnostic-vocabulary agreement test** (work package F) — the milestone's headline test:
  the validate, doctor, and repair surfaces agree on severity and code for a shared fixture.
- **Golden JSON comparisons** for all three surfaces: `snp validate --json`,
  `snp doctor --library <name> --report json`, and `snp repair --json` produce byte-identical
  output before and after the change, over a fixture with at least one error, one warning, one
  info, and one repairable finding.
- **The scan-agreement test**: `snp validate` and `snp repair --dry-run` enumerate the same
  library set, including an orphan library file.
- **The scan-count test**: the shared scan performs exactly one library-directory listing.
- **The strict-mode tests**: `snp validate --strict` and `snp doctor --strict` elevate the same
  codes and exit 6 on the same fixtures.
- **The emitted-code-set test**: the full list of codes each surface can emit is asserted to
  match the pre-milestone list (work packages B and C).

### Restart, recovery, contention, and cancellation tests

- `tests/repair_transactions.rs` stays green: it is the suite that covers the repair path's
  transaction interaction, and `snp repair` is the surface most changed by this milestone.
- `tests/backup_contracts.rs`, `tests/manifest_contracts.rs`, and
  `tests/restore_manifest_gate.rs` are unaffected by M003 but must stay green — they share the
  `repair_cmd` module.

### Security and negative tests

- A test that a malformed code string is rejected by `DiagnosticCode::new` with a message naming
  the string, and that no surface emits it.
- A test that `snp validate` and `snp doctor` still perform no write: the config directory's
  file set and mtimes are unchanged after a run. (Note the pre-existing exception recorded in
  § 8: `LibraryManager::new` may write a corrupt-file backup at `src/library/manager.rs:62-77`;
  the test fixture must use a *valid* index so no backup is triggered.)

### Migration and compatibility tests

- `tests/public_api_smoke.rs` and `tests/architecture.rs` stay green — they are the suites most
  likely to reference a deleted local type.
- `tests/output_contracts.rs` stays green: it pins the stdout/stderr split of the CLI surfaces.

## 11. Required verification commands

Run these in order. **Never run a cargo command while another test run is in flight** — the
integration tests execute `target/debug/snp`, and a concurrent rebuild replaces that binary
mid-run.

```bash
# Focused: the shared vocabulary and the shared scan
cargo test --features test-support --lib diagnostics
cargo test --features test-support --lib library
cargo test --features test-support --test output_contracts
cargo test --features test-support --test public_api_smoke
cargo test --features test-support --test architecture

# The repair surface: serial by contract
cargo test --features test-support --test repair_transactions -- --test-threads=1

# The golden-JSON and agreement tests added by this milestone
cargo test --features test-support --test diagnostic_vocabulary_agreement -- --test-threads=1
cargo test --features test-support --test diagnostic_output_stability -- --test-threads=1

# Lint gates (NOT --all-features; the six required-features targets are
# never linted by check.sh and must be linted explicitly)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Authoritative Linux CI gate
bash scripts/check.sh
```

Notes:

- The two new test targets named above are the ones this milestone adds; if they are placed in
  existing files instead, substitute those files.
- `repair_transactions` is one of the three serial targets
  (`repair_transactions`, `local_data_lock_barriers`, `process_lock_concurrency`) and must run
  with `-- --test-threads=1`.
- `cleanup_crash_failpoints` and `restore_crash_failpoints` are invoked by **no** script and are
  not affected by M003 (it touches no restore or cleanup path); run them only if M002's changes
  are in the same tree.
- `bash scripts/check.sh` is the authoritative Linux CI gate and is equivalent to CI.

## 12. Documentation updates

| Document | Change |
|----------|--------|
| `architecture/diagnostics.md` | Add the shared severity / code / repairability types to "Key Types" (`:27-49`); rewrite the Code-Prefix Table (`:307-318`) to describe the **enforced** grammar rather than a convention; update the "Gotchas" entry at `:397-401` ("There is no shared *finding* type") to say that the *vocabulary* is shared while the *report envelopes* stay per-surface; update the "Invariants" bullet at `:386-387` and the note at `:418-419`; correct the stale line references at `:70` and `:378` (`src/library/manager.rs:344`, `src/library/model.rs:194`/`:212`). |
| `architecture/commands/validate_cmd.md` | Record that validate uses the shared severity / code / repairability types and the shared library scan; update the shared-view table row at `:137`. |
| `architecture/commands/doctor_cmd.md` | Record the shared code type and the shared scan; update the "no generic finding DSL" note at `:69` to distinguish the shared vocabulary from the per-surface envelopes. |
| `architecture/commands/repair_cmd.md` | Record that repair uses the shared severity / repairability / code internally and the shared library scan; document that the emitted `category` strings are preserved and that the per-finding and per-action category axes are now distinctly named (§ 3.4); update the shared-classifier row at `:85` and the note at `:233`. |
| `architecture/library.md` | Document the shared library scan and correct the stale `src/library/manager.rs:310` reference at `:408`. |
| `architecture/overview.md` | Add the shared scan and the shared vocabulary to the layer map (§ 5.2) and to the deep-dive index (§ "Deep-Dive Index") if a module is added. |
| `docs/COMMANDS.md` | No change to the command reference; update only if a described output shape moves (it does not). |
| `docs/COMMAND_CONTRACTS.md` | No new row — no command is added. Confirm the `validate` / `doctor` / `repair` rows (`:24`, `:25`, `:28`) still describe the same flags and exit codes. |
| `docs/LOGICAL_LAYERS.md` | Update only if a module is added (the scan's result type lives in `src/library/`, which is already in the core layer). |
| `CHANGELOG.md` | One entry recording this as a behavior-preserving refactor: shared severity / code / repairability vocabulary and one shared library scan; no code, message, exit code, or JSON shape changed. |

## 13. Acceptance criteria

1. One shared severity type, one shared code type, and one shared repairability type exist in
   `src/diagnostics.rs`; `src/commands/validate_cmd.rs`'s local `Severity` (`:32`) and
   `Repairability` (`:40`) are deleted.
2. All three surfaces use the shared types: validate, doctor, and repair.
3. One shared library scan exists in the library layer and is consumed by all three surfaces; the
   repair surface no longer enumerates library files independently — `fs::read_dir` at
   `src/commands/repair_cmd.rs:304` and `:857` is gone.
4. The scan count does **not** increase: the shared scan performs exactly one library-directory
   listing.
5. No diagnostic code, message, or exit code changed. Golden JSON comparisons prove the JSON
   output shapes of `snp validate`, `snp doctor`, and `snp repair` are unchanged.
6. The diagnostic-vocabulary agreement test passes: validate, doctor, and repair agree on
   severity and code for a shared fixture.
7. The repair surface's emitted `category` strings are unchanged, including the three drift sites
   in § 3.4, and the per-finding / per-action category axes now have distinct names.
8. `RepairAction::is_safe()` and the shared repairability agree for every variant.
9. The two pre-existing code collapses (§ 3.5) are preserved verbatim and documented.
10. No command, flag, spelling, `command_behavior()` entry, or `docs/COMMAND_CONTRACTS.md` row was
    added; the `snp data` mirror group is unchanged and `snp data doctor` was **not** added.
11. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
    `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and
    `bash scripts/check.sh` all pass.
12. `tests/repair_transactions.rs`, `tests/output_contracts.rs`, `tests/public_api_smoke.rs`, and
    `tests/architecture.rs` are green.
13. A static guard rejects a new free-form `code: String` in the three command modules and the
    shared diagnostics module, and the emitted-code-set test pins the current code lists.
14. The two stale documentation references in § 3.8 are corrected, and `CHANGELOG.md` records the
    change as behavior-preserving.

## 14. Stop conditions

Stop and report to the parent session — do **not** improvise — if any of the following holds:

1. Enforcing the code grammar would reject a code string that a surface emits today. The grammar
   must be widened to accept the existing strings, not the strings changed.
2. The shared severity or repairability type would change a serialized value (for example if
   `DiagnosticRepairability`'s serde names cannot match
   `src/commands/validate_cmd.rs:40-44` exactly). Stop rather than change the JSON.
3. The shared scan would have to perform a **second** library-directory listing, or would need a
   lock the current code does not take. The scan replaces repair's two walks; it must not add
   one.
4. Migrating repair onto the shared scan would change which libraries or files repair enumerates.
   The scan must be a superset of repair's current view (index-driven sources **plus** orphan
   files).
5. Making the three surfaces "agree" would require changing an emitted code, merging two
   distinct diagnostics, or suppressing a finding any surface reports today.
6. A shared type would have to live outside `src/diagnostics.rs` / `src/library/` in a way that
   inverts the layer rules in `docs/LOGICAL_LAYERS.md`, or `src/diagnostics.rs` would have to
   gain an I/O or platform dependency.
7. `snp data doctor` would have to be added for the milestone's exit condition to be satisfiable.
   It must not be added — the clause is vacuous (§ 3.6).
8. Any of `tests/repair_transactions.rs`, `tests/output_contracts.rs`,
   `tests/public_api_smoke.rs`, or `tests/architecture.rs` fails for a reason that cannot be fixed
   without changing an emitted value.

## 15. Closure evidence required

- The closure record must cite, with `file:line` at the closing commit: the shared severity, code,
  and repairability types and their definitions; the deletion sites of validate's local `Severity`
  and `Repairability`; the shared scan's definition and its single listing; the removal of
  `fs::read_dir` at `src/commands/repair_cmd.rs:304` and `:857`; and the two stale documentation
  references, now corrected.
- It must list, by name, every test added and every existing suite re-run, with the exact commands
  and their results, and include the output of `cargo fmt --all -- --check`, both `cargo clippy`
  invocations, and `bash scripts/check.sh`.
- It must include the **before/after JSON comparison** for all three surfaces, proving the output
  shapes are unchanged.
- It must record the emitted-code lists for validate and doctor, and state explicitly that the
  two collapses in § 3.5 and the three category drift sites in § 3.4 were **preserved**, not
  fixed, with the reason.
- It must state explicitly that `snp data doctor` was **not** added and why (§ 3.6).

## 16. Handoff notes

- **Start with work package A.** The shared types are the vocabulary everything else adopts, and
  the grammar in work package A must be checked against every code string the three surfaces
  build **before** any surface is migrated — the runtime-built doctor codes are the trap.
- **The code type must be a validating newtype, not an enum.** Doctor builds codes at runtime
  (`src/commands/doctor_cmd.rs:676`, `:133`), so a closed enum is impossible.
- **Keep `DiagnosticCode` serde-transparent.** That is what makes "the JSON output shapes are
  unchanged" mechanically true rather than a promise.
- **Repair's JSON is hand-built.** `JsonRepairItem` (`src/commands/repair_cmd.rs:992-999`) is the
  real contract, not `RepairItem`. Enriching `RepairItem` internally is free; touching
  `JsonRepairItem` is a breaking change.
- **Do not "fix" the category drift.** The three drift sites (§ 3.4) are output. Preserving them
  is the point; renaming the axes is the fix.
- **Do not "fix" the two code collapses.** They are output too (§ 3.5). M003 changes vocabulary,
  not findings.
- **The scan replaces walks; it never adds one.** If an implementation adds a second listing,
  the roadmap's performance rule is broken — stop and reconsider the design.
- **The shared scan belongs in the library layer, not in validate.** The roadmap attributes it to
  validate; the code says otherwise (§ 3.3). Putting it in `src/library/manager.rs` keeps all
  three surfaces as equal consumers.
- **`snp data doctor` does not exist and must not be created.** The roadmap's exit condition is
  vacuous on that clause (§ 3.6).
- **Two documentation defects are known and in scope:** `architecture/diagnostics.md:70`/`:378`
  and `architecture/library.md:408` cite `src/library/manager.rs:310` for
  `inspect_library_index` (actual: `:344`), and `architecture/diagnostics.md:378` cites
  `src/library/model.rs:123` for the view types (actual: `PrimaryState` `:194`,
  `LibraryIndexInspection` `:212`).
- **Adjacent work deliberately not done here:** `src/status_snapshot.rs`'s severity enum and
  `src/utils/variables.rs`'s `#[allow(dead_code)]` severity enum remain. The shared vocabulary
  must be shaped so a later milestone can adopt them without a breaking change.
