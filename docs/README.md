# snp-it Documentation Index

Every document in `docs/`, classified by whether it is a **current contract** you
should trust, or a **historical snapshot** retained only for traceability.

**If you are an agent:** read the status column before trusting anything here.
A document in `archive/` describes a codebase state that no longer exists.

## Current contracts (trust these)

| Document | Subject | Enforced by |
|----------|---------|-------------|
| [EXIT_CODES.md](EXIT_CODES.md) | Stable exit codes 0–10 and `CliOutcome` mapping | `src/outcome.rs`, `tests/execution_outcomes.rs` |
| [COMMAND_CONTRACTS.md](COMMAND_CONTRACTS.md) | Per-command behavioral contract, startup-recovery policy, stream discipline | `src/main.rs::command_behavior` |
| [CLI_EXITCODE_STREAM_POLICY.md](CLI_EXITCODE_STREAM_POLICY.md) | stdout/stderr discipline and easy-to-miss exit cases | `docs/EXIT_CODES.md` |
| [JSON_SCHEMAS.md](JSON_SCHEMAS.md) | Machine-readable JSON envelopes | `tests/schema.rs` |
| [LIBRARY_SCOPE.md](LIBRARY_SCOPE.md) | `--library` resolution, single vs selector path | `src/selector.rs` |
| [IDENTITY_CONTRACT.md](IDENTITY_CONTRACT.md) | Deterministic snippet IDs, library identity | `tests/identity_contract.rs` |
| [LOGICAL_LAYERS.md](LOGICAL_LAYERS.md) | Three-layer dependency direction and known exceptions | `tests/architecture.rs` |
| [MCP.md](MCP.md) | Read-only stdio MCP tools and protocol | `src/mcp/`, `tests/mcp_integration.rs` |
| [PERSISTENCE_INVENTORY.md](PERSISTENCE_INVENTORY.md) | Every persisted artifact, path, durability class | `src/library/`, `src/config/` |
| [PUBLIC_API.md](PUBLIC_API.md) | Stable public surface of `src/lib.rs` | `tests/public_api_smoke.rs` |
| [THREAT_MODEL.md](THREAT_MODEL.md) | Assets, adversaries, trust boundaries | `SECURITY.md` |
| [COMPATIBILITY.md](COMPATIBILITY.md) | Backward-compatibility and deprecation rules | `tests/cli_surface_compat.rs` |
| [SUPPLY_CHAIN_POLICY.md](SUPPLY_CHAIN_POLICY.md) | Dependency, license, `cargo-deny` policy | `deny.toml`, CI |
| [PET_COMPATIBILITY.md](PET_COMPATIBILITY.md) | Behavioral matrix vs. `pet` import | `src/commands/pet_analysis.rs` |
| [ARCHITECTURE_INVENTORY.md](ARCHITECTURE_INVENTORY.md) | Module inventory, scoped to pet-compatibility work | — |

## Historical snapshots (do not trust; retained for traceability)

Moved out of `docs/` so they cannot be mistaken for contracts. Each carries an
`ARCHIVED` header explaining exactly what is stale about it.

| Document | Why archived |
|----------|--------------|
| [archive/SECURITY_AUDIT.md](archive/SECURITY_AUDIT.md) | Frozen 2026-07-22, pre-EggServe / pre-eggfetch. Current posture is in `SECURITY.md` + `THREAT_MODEL.md` |
| [archive/FEATURE_BOUNDARIES.md](archive/FEATURE_BOUNDARIES.md) | Self-contradictory; recommends feature gates that were removed. Version numbers drifted |
| [archive/CANONICAL_OPERATIONS.md](archive/CANONICAL_OPERATIONS.md) | Every `path:line` citation points at the pre-split `src/library.rs` / `src/config.rs` |
| [archive/FUZZING_AND_PROPERTY_TESTS.md](archive/FUZZING_AND_PROPERTY_TESTS.md) | Planning artifact for fuzz work that was never started; no fuzz harness exists |

## Related, outside `docs/`

- [`../architecture/overview.md`](../architecture/overview.md) — **start here.** Module map plus the Deep-Dive Index for all 61 architecture documents.
- [`../AGENTS.md`](../AGENTS.md) — verify commands, gotchas, invariants.
- [`../AGENTS.override.md`](../AGENTS.override.md) — session pitfall notes.
- [`../.skills/`](../.skills) — task-shaped skills (sync, transactions, server, selector parity, …).
- [`../plans/registry.md`](../plans/registry.md) — authoritative planning status.
- [`../USER_GUIDE.md`](../USER_GUIDE.md) — end-user guide.
- [`../SECURITY.md`](../SECURITY.md) — vulnerability reporting and current security posture.
