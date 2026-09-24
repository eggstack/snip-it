# AGENTS.override.md

## Implementation Notes

### Common Pitfalls

1. **Argon2 parameter change is a breaking change.** If memory cost is changed, all existing encrypted snippets become undecryptable. Consider parameter versioning first.
2. **`commands/mod.rs` changes affect all TUI commands.** Be careful modifying `load_snippets`, `save_snippets`, or `run_snippet_selection`.
3. **`snip-sync/src/main.rs` is large.** Keep CLI and process-lifecycle edits localized; HTTP endpoint policy belongs in `snip-sync/src/http.rs`.
4. **`src/sync.rs` methods take `&mut self`.** All gRPC RPCs retry through the single `retry_grpc_unified!` macro (with `RetryBackoff`), which expands inline so `self.client.<rpc>()` reborrows work. Do not reintroduce a closure-based generic retry helper — it fails to compile with "captured variable cannot escape `FnMut` closure body".
5. **`src/ui/` split requires updating imports.** Any function moved to `ui/theme.rs` etc. needs re-exports in `ui/mod.rs` for callers in `commands/`.
6. **Keychain testing.** The `keyring` crate behaves differently on macOS, Linux, and Windows. Test on all platforms or add a fallback path.
7. **Sync encryption failure flow.** Changes to sync flow logic affect the `last_sync` timestamp update. Test with: (a) normal sync, (b) sync with intentionally corrupted snippets, (c) partial failure.
8. **Removing CLI flags is a breaking change.** If users have scripts using removed flags, they will break. Consider deprecation warning first.
9. **Transaction journals live in `<config>/.transaction/`, not a separate journals dir.** Pending-marker APIs take the state dir; transaction APIs take `.transaction`. See `.skills/transactions-and-auto-sync.md`.
10. **`write_schema_version` must use `toml::Table` (not `toml::Value`)** to preserve array-of-tables structures in TOML files.
11. **Isolated-test env overrides need `unsafe` in edition 2024.** `std::env::set_var` in tests (e.g., `XDG_CONFIG_HOME` overrides) requires an unsafe block.
12. **snip-sync HTTP is a concrete EggServe leaf service.** `/health`, `/metrics`, CORS, and security-header policy live in `snip-sync/src/http.rs`; keep Tonic on its separate listener and preserve explicit request-body rejection, typed shutdown completion, and external TLS termination. Do not reintroduce Axum/Tower-HTTP or generic routing infrastructure.
