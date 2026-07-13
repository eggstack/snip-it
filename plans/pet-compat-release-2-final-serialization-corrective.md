# Release 2 Final Corrective Plan: Exact TOML Round-Trips and Closure Evidence

## Purpose

Close the final known Release 2 correctness gap before beginning Release 3.

Release 2 acquisition features are substantially implemented and have already received a broad corrective pass covering secure editor tempfiles, `$VISUAL`/`$EDITOR` argument parsing, shared exact-source validation, file-source policy, shell-history preservation, and downstream backup/select/run/sync tests.

One material issue remains: current documentation and tests exclude commands containing tabs, trailing spaces, and CRLF line endings on the premise that TOML cannot preserve them. That premise must be verified against the actual serialization pipeline. TOML can represent these characters semantically, so any loss in snip-it should be treated as an implementation defect or a documented repository-specific normalization decision, not as an inherent TOML limitation.

This plan is narrowly corrective. Do not begin Release 3 work, redesign the storage format, or change existing acquisition interfaces unless required to preserve exact command strings.

## Required Outcome

After this pass:

1. Tabs, trailing spaces, and CRLF command text are tested through the real library save/load pipeline.
2. Any corruption is localized to the serializer, deserializer, custom TOML helpers, import compatibility layer, or another normalization boundary.
3. The implementation preserves the original Rust command string across create, save, load, resave, select, backup, export, and sync wherever the existing data model promises exact text.
4. The golden command corpus includes tabs, trailing spaces, and CRLF where technically supportable.
5. Documentation no longer attributes repository-specific behavior to a nonexistent TOML format limitation.
6. Every ignored test is inventoried and none silently masks a Release 2 acquisition or preservation defect.
7. Release 2 receives explicit closure evidence from the full workspace, PTY suite, and normal CI matrix.

## Current Risk

The current closure commit states that `toml::to_string_pretty` cannot round-trip tabs, trailing spaces, and CRLF. This must not be accepted without direct tests.

A serializer may choose escaped textual representations such as `\t`, `\r`, and `\n`; that is acceptable if deserialization reconstructs the exact original string. File-level textual identity is not required. Semantic command-string identity is required.

Custom helpers around quoting, backslashes, multiline strings, or compatibility repair are more likely sources of corruption than TOML itself.

## Workstream A: Reproduce the Full Serialization Matrix

### A1. Add direct serializer/deserializer tests

Test `Snippet` or the closest persisted library structure with commands containing:

- an internal tab;
- a leading tab;
- a trailing tab;
- one trailing space;
- multiple trailing spaces;
- spaces before a newline;
- CRLF line endings;
- mixed LF and CRLF;
- a final carriage return;
- combinations with quotes and backslashes.

For every case:

```text
original Rust String
→ toml::to_string / to_string_pretty
→ toml::from_str
→ recovered Rust String
```

Assert semantic equality.

### A2. Test the real save/load helpers

Exercise the exact paths used by:

- named libraries;
- legacy single-file storage;
- backups;
- normal mutation followed by full rewrite;
- load followed by save without mutation.

Do not rely only on serde unit tests.

### A3. Test compatibility preprocessing

Inspect and test any helpers that rewrite TOML before parsing or after serialization, including backslash quoting and legacy compatibility normalization.

Assert that these helpers do not:

- trim trailing spaces;
- normalize CRLF to LF unless explicitly documented;
- replace tabs with spaces;
- modify escaped control characters;
- treat multiline commands as single-line repair targets.

## Workstream B: Fix the Actual Corruption Boundary

### B1. Preserve semantic strings, not textual TOML identity

The stored TOML may use escapes or different quote styles. The acceptance criterion is that reloading produces the exact original command string.

### B2. Repair custom TOML helpers if responsible

Prefer a localized fix over replacing the serializer.

Possible approaches include:

- operating on parsed values rather than raw TOML text;
- limiting repair helpers to fields and representations they can safely recognize;
- avoiding regex or line-based rewriting of multiline strings;
- adding explicit escaping only at serialization boundaries;
- bypassing compatibility repair for newly generated canonical TOML.

### B3. Avoid silent normalization

If a normalization is unavoidable for a particular path, it must be:

- explicit;
- narrowly scoped;
- documented as a snip-it behavior;
- tested;
- excluded from claims of exact preservation.

Do not call it a TOML limitation unless demonstrated by the TOML specification and serializer behavior.

## Workstream C: Restore the Golden Corpus

Add cases for:

- tab-delimited command text;
- Makefile-style leading tabs;
- trailing spaces;
- CRLF scripts;
- mixed newline styles;
- quotes/backslashes combined with the above.

Run each supported exact source through the corpus:

- `--command-stdin`;
- `--from-file`;
- `--editor` through a deterministic test editor;
- positional input where shell/argv semantics permit exact representation.

Then validate:

- JSON list output;
- `snp select --raw` output file;
- backup reload;
- repeated full-library rewrites;
- structured export/import if currently available;
- sync round-trip through the existing test server.

## Workstream D: Account for Ignored Tests

Inventory all ignored tests and record for each:

- test name;
- reason ignored;
- platform or external dependency requirement;
- whether CI runs it elsewhere;
- whether it covers Release 2 behavior.

No Release 2 preservation, shell-capture, editor, file, stdin, backup, export, or sync test may remain ignored without a documented platform-specific execution path.

## Workstream E: Documentation Correction

Update:

- `AGENTS.md`;
- `README.md`;
- `USER_GUIDE.md`;
- `CHANGELOG.md`;
- `architecture/commands/new_cmd.md`;
- `docs/ARCHITECTURE_INVENTORY.md`;
- `docs/PET_COMPATIBILITY.md`;
- the Release 2 closure plan annotations.

Remove statements that TOML inherently cannot preserve tabs, trailing spaces, or CRLF.

Document the actual tested contract:

- exact sources preserve the command string semantically;
- TOML textual escaping may differ;
- interactive `--multiline` remains delimiter-based and is not byte-exact for all input;
- any remaining normalization is explicitly named.

## Workstream F: Validation and Release Gate

Run at minimum:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --test integration -- golden_corpus
cargo test --test integration -- cross_source
cargo test --test integration -- backup
cargo test --test integration -- sync
cargo test --test pty_integration -- --test-threads=1
```

Confirm the normal CI matrix on Linux, macOS, and Windows where supported.

## Release 2 Closure Criteria

Release 2 is closed only when:

- secure editor creation remains intact;
- stdin, file, and editor sources share validation;
- tabs, trailing spaces, and CRLF have direct real-pipeline tests;
- exact strings survive storage and downstream workflows;
- misleading TOML-limitation language is removed;
- ignored tests are accounted for;
- full validation is green;
- no Release 3 functionality is included in the corrective patch.
