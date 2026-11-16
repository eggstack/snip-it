# snp-it Documentation Index

Every document in `docs/`, classified by whether it is a **current contract** you
should trust, or a **historical snapshot** retained only for traceability.

**If you are an agent:** read the status column before trusting anything here.
A document in `archive/` describes a codebase state that no longer exists.

## Current contracts (trust these)

| Document | Subject | Enforced by |
|----------|---------|-------------|
| [USER_GUIDE.md](USER_GUIDE.md) | Task-oriented guide: libraries, variables, pet import, shell integration, themes, sync, auto-sync, recovery | Narrative guide; individual claims cross-reference the contract docs below |
| [COMMANDS.md](COMMANDS.md) | Command overview and common selection behavior | The CLI schema and `snp --help` provide the current flag list |
| [EXIT_CODES.md](EXIT_CODES.md) | Stable exit codes 0–10 and `CliOutcome` mapping | `src/outcome.rs`, `tests/execution_outcomes.rs` |
| [COMMAND_CONTRACTS.md](COMMAND_CONTRACTS.md) | Per-command behavioral contract, startup-recovery policy, stream discipline | `src/main.rs::command_behavior` |
| [CLI_EXITCODE_STREAM_POLICY.md](CLI_EXITCODE_STREAM_POLICY.md) | stdout/stderr discipline and easy-to-miss exit cases | Points to [`EXIT_CODES.md`](EXIT_CODES.md) for the live exit-code contract; not independently test-enforced |
| [JSON_SCHEMAS.md](JSON_SCHEMAS.md) | Machine-readable JSON envelopes | `tests/schema.rs` covers `list --json` and `list --csv`; it does not cover `get --json`, `status --json`, `doctor --report json`, or `restore --dry-run --json` |
| [LIBRARY_SCOPE.md](LIBRARY_SCOPE.md) | `--library` resolution, single vs selector path | `src/selector.rs` |
| [IDENTITY_CONTRACT.md](IDENTITY_CONTRACT.md) | Deterministic snippet IDs, library identity | `tests/identity_contract.rs` |
| [LOGICAL_LAYERS.md](LOGICAL_LAYERS.md) | Three-layer dependency direction and known exceptions | `tests/architecture.rs` checks crate-root `crate::` path prefixes and raw-pointer casts; external-crate and `std` rules are not test-enforced |
| [MCP.md](MCP.md) | Read-only stdio MCP tools and protocol | `src/mcp/`, `tests/mcp_integration.rs` |
| [PERSISTENCE_INVENTORY.md](PERSISTENCE_INVENTORY.md) | Every persisted artifact, path, durability class | `src/library/`, `src/config/` |
| [PUBLIC_API.md](PUBLIC_API.md) | Stable public surface of `src/lib.rs` | `tests/public_api_smoke.rs` |
| [THREAT_MODEL.md](THREAT_MODEL.md) | Assets, adversaries, trust boundaries | Narrative contract, cross-referenced from `SECURITY.md`; not test-enforced |
| [COMPATIBILITY.md](COMPATIBILITY.md) | Backward-compatibility and deprecation rules | `tests/cli_surface_compat.rs` checks `snp data` equivalence and the `val` and `v` aliases; the alias table and deprecation policy are not test-enforced |
| [SUPPLY_CHAIN_POLICY.md](SUPPLY_CHAIN_POLICY.md) | Dependency, license, `cargo-deny` policy | `deny.toml`; `cargo deny check` is a local gate, not a CI step |
| [PET_COMPATIBILITY.md](PET_COMPATIBILITY.md) | Pet file compatibility and migration | `src/commands/pet_analysis.rs` |

## Historical snapshots (do not trust; retained for traceability)

Moved out of `docs/` so they cannot be mistaken for contracts. Each carries an
`ARCHIVED` header explaining exactly what is stale about it.

| Document | Why archived |
|----------|--------------|
| [archive/SECURITY_AUDIT.md](archive/SECURITY_AUDIT.md) | Frozen 2026-07-22, pre-EggServe / pre-eggfetch. Current posture is in `SECURITY.md` + `THREAT_MODEL.md` |
| [archive/CANONICAL_OPERATIONS.md](archive/CANONICAL_OPERATIONS.md) | Every `path:line` citation points at the pre-split `src/library.rs` / `src/config.rs` |
| [archive/CLI_EXITCODE_STREAM_POLICY_SUPERSEDED.md](archive/CLI_EXITCODE_STREAM_POLICY_SUPERSEDED.md) | Retired pre-`CliOutcome` 2–6 exit-code proposal plus an unimplemented stdout/stderr stream contract. Live contract: [`EXIT_CODES.md`](EXIT_CODES.md) |

Three archived snapshots were **pruned** rather than retained, because they
documented nothing that ever existed: a plan for fuzz work that was never
started, a self-contradictory feature-flag analysis recommending gates that
were removed, and a pre-`architecture/` module inventory that named a
non-existent function and wrong counts. The surviving archives are the ones
either still referenced from a live document or recording a decision that
actually shipped.

## Related, outside `docs/`

- [`../architecture/overview.md`](../architecture/overview.md): **start here** for the module map, cross-cutting invariants, and index of 62 deep dives (63 documents including the overview).
- [`../AGENTS.md`](../AGENTS.md): verification commands, gotchas, and invariants, with links into the architecture sections.
- [`../AGENTS.override.md`](../AGENTS.override.md): session pitfall notes.
- [`../.skills/`](../.skills): task-shaped skills for sync, transactions, server, selector parity, testing, and documentation.
- [`../plans/registry.md`](../plans/registry.md): authoritative planning status.
- [`../snip-sync/README.md`](../snip-sync/README.md): running the sync server with systemd, Docker, or a reverse proxy.
- [`../SECURITY.md`](../SECURITY.md): vulnerability reporting and current security posture.
- [`../packaging/README.md`](../packaging/README.md): installer verification, pinned installs, and fallbacks.
