# AGENTS.override.md

## Implementation Strategy Notes

### Wave Parallelization

The plan is organized into 5 waves. Within each wave, items touch independent files and can be parallelized using sub-agents. Recommended sub-agent assignments per wave:

**Wave 1 (Security):** 3 sub-agents
- Sub-agent A: Items 1.1, 1.2 (encryption.rs, config.rs, Cargo.toml)
- Sub-agent B: Items 1.3, 1.4, 1.5 (snip-sync/src/main.rs — CORS, rate limiting, registration)
- Sub-agent C: Item 1.6 (snip-sync/src/main.rs TLS docs, config.rs default URL)

**Wave 2 (Core Bugs):** 4 sub-agents
- Sub-agent A: Items 2.1, 2.2 (sync_cmd.rs, sync.rs, sync_commands.rs)
- Sub-agent B: Items 2.3, 2.4 (library.rs)
- Sub-agent C: Item 2.5 (commands/mod.rs)
- Sub-agent D: Items 2.6, 2.7, 2.8 (clipboard.rs, logging.rs)

**Wave 3 (Server):** 2 sub-agents
- Sub-agent A: Items 3.1, 3.4 (snip-sync/src/db.rs)
- Sub-agent B: Items 3.2, 3.3, 3.5 (snip-sync/src/main.rs, db.rs)

**Wave 4 (Code Quality):** 3 sub-agents
- Sub-agent A: Items 4.1, 4.3, 4.4 (commands/ files — remove --config, validate cron, capture stderr)
- Sub-agent B: Items 4.5, 4.6 (ui.rs, variables.rs — move Variable, fix HashSet lookup)
- Sub-agent C: Item 4.7 (snip-proto/build.rs — add rerun-if-changed)

**Wave 5 (UI & Docs):** 2 sub-agents
- Sub-agent A: Item 5.1 (ui.rs split — largest task)
- Sub-agent B: Items 5.2, 5.3, 5.4 (architecture docs)

### Testing After Each Wave

After each wave completes, run:
```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test
cargo test -p snip-sync
```

### Common Pitfalls

1. **Argon2 parameter change is a breaking change.** If memory cost is increased, all existing encrypted snippets become undecryptable. Consider parameter versioning first.
2. **`commands/mod.rs` changes affect all TUI commands.** Be careful modifying `load_snippets`, `save_snippets`, or `run_snippet_selection`.
3. **`snip-sync/src/main.rs` is ~1080 lines.** When adding rate limiting to `get_snippets`/`list_libraries`, follow the exact pattern from existing endpoints.
4. **`src/sync.rs` methods take `&mut self`.** The `retry_grpc!` macro cannot be used with `self.client.sync()` due to borrow conflicts. See doc comment on `sync_with_retry`.
5. **`src/ui.rs` split requires updating all imports.** Any function moved to `ui/theme.rs` etc. needs re-exports in `ui/mod.rs` for callers in `commands/`.
6. **Wave 1.2 (API key keychain) requires cross-platform testing.** The `keyring` crate behaves differently on macOS, Linux, and Windows. Test on all platforms or add a fallback path.
7. **Wave 2.2 (encryption failure loss) is high-risk.** The fix changes sync flow logic. Test with: (a) normal sync, (b) sync with intentionally corrupted snippets, (c) partial failure where some snippets encrypt and others don't.
8. **Wave 4.1 (remove --config) is a breaking change.** If users have scripts using `snp run --config`, they will break. Consider deprecation warning first.
