# Security Assurance M001 — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/security-assurance/001-advisory-posture-and-license-policy.md`

Source subsystem roadmap:

- `plans/subsystems/security-assurance-roadmap.md#milestone-001--advisory-posture-and-license-policy`

Repository baseline reviewed: `0f3bc98` (`chore/prune-comments-and-plans`), plus
the uncommitted dependency-refresh and doc-pruning changes already present in
that worktree. Implementation is uncommitted; the SHA is recorded at commit time.

Implementation commits or pull requests:

- Uncommitted working tree on `chore/prune-comments-and-plans` — `deny.toml`,
  `.cargo/audit.toml`, `SECURITY.md`, `scripts/check-advisories.sh`,
  `docs/SUPPLY_CHAIN_POLICY.md`, ADR-0003 status.

Primary class: infrastructure. Policy and local gate only; no runtime change.

## 1. Executive finding

M001 is complete. `cargo audit` and `cargo deny check` both exit 0 on the
working tree with exactly four advisory IDs suppressed, each carrying the
six-field rationale block from ADR-0003, and `CDLA-Permissive-2.0` is the only
license added. `scripts/check-advisories.sh` proves the three-way suppression
mirror and fails on an unsuppressed advisory, on a missing rationale field, and
on a non-ID suppression.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| `cargo audit` exits 0 | `cargo audit` | pass | 468 crate dependencies scanned; four suppressed IDs reported as allowed warnings |
| `cargo deny check` exits 0 | `cargo deny check` | pass | `advisories ok, bans ok, licenses ok, sources ok` |
| Exactly four advisory IDs, each with ID/scope/why/added/review-by/remove-when | `deny.toml` | pass | Fields enforced by the script, see next row |
| Missing rationale field fails the gate | removed `Scope:` from the `RUSTSEC-2026-0370` block in a temp edit | pass | `FAIL: RUSTSEC-2026-0370 has an incomplete rationale block`; edit reverted |
| No crate-name or wildcard suppression | script parses the `ignore` array | pass | Every element must match `^"RUSTSEC-NNNN-NNNN"$` |
| `CDLA-Permissive-2.0` is the only license addition | `deny.toml` diff | pass | `node_modules`-free; one line plus rationale comment |
| Every ignored ID appears in both machine lists and in `SECURITY.md` | script set comparison | pass | `deny.toml` = `.cargo/audit.toml` = `advisory-ignore-ids` marker (4 IDs) |
| A drifted mirror fails | removed one ID from the `SECURITY.md` marker in a temp edit | pass | `FAIL: deny.toml and SECURITY.md suppress different advisory sets`; edit reverted |
| Unsuppressed advisory still fails | `--self-test` | pass | Removed `RUSTSEC-2026-0370` from a temp config; gate failed and the output named the ID |
| No dependency-source change | `Cargo.lock`, `deny.toml` `[sources]` | pass | crates.io only; `unknown-registry`/`unknown-git` still `deny` |
| SQLite scope unchanged | `cargo tree -e features -i libsqlite3-sys` | pass | `bundled` present, reached through `snip-sync`; `snip-it` edge remains a dev-dependency |
| Workspace still builds | `cargo check --workspace --all-targets` | pass | No lockfile churn beyond the working tree's existing state |

## 3. Production implementation evidence

- `deny.toml`: four `ignore` entries with rationale blocks; policy comment
  pointing at ADR-0003; `CDLA-Permissive-2.0` allowed; the unused
  `BSD-2-Clause` allowance retained with a written justification (it is a
  deliberate policy ceiling, and the resulting `license-not-encountered`
  warning is not a gate). The stale RUSTSEC-2024-0437 comment was removed:
  `protobuf` is now 3.7.2, above the patched threshold the comment named.
- `.cargo/audit.toml`: mirrors the same four IDs so a bare `cargo audit` agrees
  with `cargo deny`.
- `scripts/check-advisories.sh`: tool-presence check, both tools, three-way
  mirror comparison, rationale-field validation, ID-only validation, and
  `--self-test`.
- `SECURITY.md` § Advisory Exceptions: operator-facing mirror, the
  machine-readable marker line, a per-advisory table with removal conditions,
  and the per-finding exploitability summary.

## 4. Verification executed

```bash
cargo audit
cargo deny check
bash scripts/check-advisories.sh
bash scripts/check-advisories.sh --self-test
cargo check --workspace --all-targets
cargo tree -e features -i libsqlite3-sys
```

Results: all pass. `cargo audit` exit 0; `cargo deny check` exit 0; both script
modes exit 0; the negative proofs above exited 1 for the intended reason and
were reverted. `cargo check --workspace --all-targets` passes (one pre-existing
macOS-only `dead_code` warning; see § 10).

## 5. Invariant review

- `Cargo.lock` remains the single resolution source; the only lockfile change is
  the `snip-sync` version bump owned by M004.
- Dependency sources remain crates.io only.
- Both published crates stay publishable; no git or path dependency entered the
  release graph.
- Suppressions are ID-only and scoped; no global lint relaxation exists. The
  prohibited `unmaintained = "workspace"` setting was not introduced.
- The allowlist gained exactly one entry.
- No blanket dependency refresh was performed.

## 6. Failure and recovery review

- A missing tool is an environment failure with an install hint, never a silent
  pass.
- Network failure fetching the advisory database fails the command; it is not
  converted into a clean verdict.
- `--self-test` writes only to `mktemp -d` and removes it on exit; it does not
  mutate the repository.
- Re-running the script is idempotent.

## 7. Migration and compatibility review

No source, wire, storage, or CLI change. `deny.toml`, `.cargo/audit.toml`, and
the script are excluded from the published crate (`Cargo.toml` `exclude` list
covers `deny.toml`; `scripts/` is excluded too). No version bump is required by
this milestone.

## 8. Security review

- Suppressions do not change runtime authorization or behavior; they change
  what the gate reports.
- The SQLite exception's named mitigation is M004's owner-only server data
  directory, not "the server is usually local".
- The three unmaintained advisories are suppressed individually, so a **new**
  unmaintained dependency still fails the gate.
- No secret was introduced; the rationale blocks contain no credential.

## 9. Documentation and operations

- `SECURITY.md` § Advisory Exceptions (new mirror and discipline).
- `deny.toml` inline rationale.
- `docs/SUPPLY_CHAIN_POLICY.md` rewritten where it contradicted reality: the
  "ignore list is empty / gate currently fails / not a CI step" statements, the
  license list, the (wrong) structured-ignore-table format, and the stale
  `rustls`/`webpki-roots` finding snapshot.
- ADR-0003 flipped to `accepted` with a status-history line.

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| low | `.cargo/audit.toml` is a third machine mirror beyond the two the plan named | A divergence is possible only if the check script is bypassed | None — the script compares all three and fails on drift |
| low | Pre-existing macOS-only clippy failure in `snip-sync/src/startup.rs` (`needless_return` in the `cfg(target_os = "macos")` arm) | `cargo clippy -D warnings` is red on macOS only; Linux CI compiles a different arm | Fix separately; not in this workstream's scope |
| low | Pre-existing macOS-only `dead_code` warning for `McpProcess::pid` in `tests/selector_search_parity.rs` | Same as above | Fix separately |

## 11. Roadmap disposition

Milestone closed; M002 may proceed. The SQLite suppression is removed in the
same change that adopts a `sqlx` release accepting `libsqlite3-sys >=0.39.0`,
without waiting for its review date.

## 12. Registry updates

`plans/registry.md` and the roadmap § 12 table record M001 as `closed`. No other
registry row changes.
