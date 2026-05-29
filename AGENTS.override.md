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
