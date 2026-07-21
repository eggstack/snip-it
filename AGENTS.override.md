# AGENTS.override.md

## Implementation Notes

### Common Pitfalls

1. **Argon2 parameter change is a breaking change.** If memory cost is changed, all existing encrypted snippets become undecryptable. Consider parameter versioning first.
2. **`commands/mod.rs` changes affect all TUI commands.** Be careful modifying `load_snippets`, `save_snippets`, or `run_snippet_selection`.
3. **`snip-sync/src/main.rs` is ~1080 lines.** When adding endpoints, follow the exact pattern from existing endpoints.
4. **`src/sync.rs` methods take `&mut self`.** The `retry_grpc!` macro cannot be used with `self.client.sync()` due to borrow conflicts. See doc comment on `sync_with_retry`.
5. **`src/ui/` split requires updating imports.** Any function moved to `ui/theme.rs` etc. needs re-exports in `ui/mod.rs` for callers in `commands/`.
6. **Keychain testing.** The `keyring` crate behaves differently on macOS, Linux, and Windows. Test on all platforms or add a fallback path.
7. **Sync encryption failure flow.** Changes to sync flow logic affect the `last_sync` timestamp update. Test with: (a) normal sync, (b) sync with intentionally corrupted snippets, (c) partial failure.
8. **Removing CLI flags is a breaking change.** If users have scripts using removed flags, they will break. Consider deprecation warning first.

### Session Notes (2026-05-29)

#### Plan Consolidation Process
- When reviewing multiple plan files, use subagents to batch read (4-5 files per agent) to preserve context window
- Always verify plan item status against actual code - discrepancies can exist (e.g., CLIP-4 was marked TODO in plan but was actually FIXED per AGENTS.md)
- Consolidate batch summaries into intermediate files before merging into final plan

#### Plan Accuracy Tips
- "Completed in Prior Work" table in plan.md is authoritative for already-fixed items
- Items can be internally inconsistent across sections - always cross-check
- Code verification subagents should read actual files, not rely on summaries
- Line numbers in plan items may be slightly off; always search for the relevant code patterns
- Some items have location descriptions that are technically accurate but describe effects rather than causes (e.g., SERVER-4 lock issue is about contention, not classic "lock across await")

#### Wave-based Parallelization
- WAVE 1 (Security): All 6 items are independent, can be split across multiple agents
- WAVE 2 (Core Bugs): 17 items organized by module (library, commands, clipboard, config)
- WAVE 3 (Improvements): 24 items with sub-waves by module (Security, Commands, Library, Logging, Server, UI)
- WAVE 4 (Low Priority): 40+ items - fully parallel, any agent can pick any item

#### Dependency Tracking
- Most items have NO dependencies (can start immediately)
- Key dependencies: TUI-3 → TUI-7,8,9,10,20,21; LIB-1 → LIB-11; LIB-6 → LIB-8; LIB-2 → LIB-13
- When assigning work, group by module to minimize context switching

### Session Notes (2026-05-29) - Plan Review Findings

During the plan review session, the following discrepancies were corrected:

1. **CLI-1 line numbers**: Claimed 97-100, actual is 92-93 (TOCTOU between validate_output_path and File::create)
2. **SERVER-3 line numbers**: Claimed 389, actual is 390-392 (api_key field completely ignored)
3. **SERVER-6 line numbers**: Claimed 208, actual is 199 (fs::read_to_string without sanitization)
4. **SERVER-4 characterization**: Not a classic "lock across await" - lock is acquired AFTER await; issue is lock contention duration
5. **CMD-3**: --clip copies command as intended; this is a documentation/expectation issue, not a code bug
6. **TUI-1**: Visual line mode bug confirmed at lines 633-638; `selected` stays at cursor while `visual_end` extends to end of list
7. **sync_commands.rs merge**: Uses `>=` at line 427 for timestamp comparison (server wins on tie) - already fixed per AGENTS.md

### Session Notes (2026-07-21) - Phase 07A Implementation

#### Phase 07A New Modules
- `src/transaction.rs` — Transaction boundary with `TransactionJournal`, `TransactionLock`, begin/commit/rollback. Uses `toml::Table` for journal serialization.
- `src/migration.rs` — Schema versioning with `SchemaVersion` ordinal type. `write_schema_version` must use `toml::Table` (not `toml::Value`) to preserve array-of-tables in TOML files.
- `src/commands/validate_cmd.rs` — 12 validation check categories. Uses `LibraryManager::new()` with `XDG_CONFIG_HOME` override for isolated tests (requires `unsafe` in edition 2024).
- `src/commands/backup_cmd.rs` — SHA-256 checksummed backup with manifest. Excludes secrets by default.
- `src/commands/restore_cmd.rs` — DryRun/Merge/Replace modes. Replace creates automatic pre-restore backup.
- `src/commands/repair_cmd.rs` — Validation-first repair with pre-repair backup. Safe repairs auto-apply; unsafe ones require manual review.

#### Key Implementation Patterns
- `atomic_replace` with `AtomicWriteOptions::for_durability()` replaces raw `fs::write` for all user-data
- Transaction journals in `~/.config/snp/transaction-journals/` for crash recovery
- `commit_transaction(state_dir, journal)` requires explicit state_dir parameter (not derived from staged files)
- Integration tests use re-exported types from crate root (`snip_it::Snippet`, `snip_it::atomic_replace` etc.) to maintain `pub(crate)` visibility for `library` and `utils` modules

#### Test Counts
- Unit tests (lib): ~950
- Integration tests: ~870
- Phase 07A-specific: ~55 (persistence_unit, identity_contract, validate_cmd, backup_cmd, restore_cmd, repair_cmd, transaction, migration)
