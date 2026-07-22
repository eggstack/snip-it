# Feature Boundaries Analysis — Phase 06A Workstream I

## Current Feature Definitions

### snip-it (root crate)
Empty feature labels (`tui`, `clipboard`, `sync`, `self-update`, `bundled-themes`) were removed in Phase 10
because they did not gate any dependencies and were misleading. `test-support` is retained for test
infrastructure. All functionality is unconditionally compiled — the binary is monolithic.

### snip-sync
```toml
[features]
default = []
test-helpers = []  # In-process test server, not shipped in production
```

### snip-proto
**No features.** Pure protobuf codegen, always built.

---

## Dependency Duplication (`cargo tree -d`)

The workspace has **463 lines** of duplicate dependency output. Key duplications:

| Crate | Versions | Caused by |
|-------|----------|-----------|
| `sha2` | 0.10.9 + 0.11.0 | snip-it uses 0.11; sqlx uses 0.10 (via digest) |
| `digest` | 0.10.7 + 0.11.3 | Same split as sha2 |
| `block-buffer` | 0.10.4 + 0.12.1 | Tied to sha2/digest versions |
| `cpufeatures` | 0.2.17 + 0.3.0 | Tied to sha2 versions |
| `crypto-common` | 0.1.7 + 0.2.2 | Tied to digest versions |
| `thiserror` | 1.x + 2.x | snip-it/snip-sync use 2.x; some transitive dep uses 1.x |
| `signal-hook` | 0.3.18 + 0.4.4 | snip-it uses 0.4; signal-hook-mio transitively pulls 0.3 |
| `getrandom` | 0.2.17 + 0.4.3 | Different major versions across deps |
| `hashbrown` | 0.15 + 0.16 + 0.17 | Three versions from different transitive paths |
| `regex` | 1.13.0 | Appears duplicated in tree (same version, different features) |
| `prost` | 0.14.4 | Duplicated because tonic uses a separate feature set |

**Root cause**: `sqlx` pulls `sha2 0.10` / `digest 0.10`, while the main crate uses `sha2 0.11` directly. `signal-hook` 0.3 vs 0.4 is a transitive pull from `signal-hook-mio`. `thiserror` 1.x vs 2.x comes from transitive deps still on 1.x.

---

## Feature Gate Candidates

### 1. TUI (`tui` feature)
**Dependencies**: `ratatui`, `crossterm`, `fuzzy-matcher`, `unicode-width`
**Currently used by**: `src/ui/`, `commands/select_cmd.rs`, `commands/search_cmd.rs`
**Recommendation**: Gate behind `tui` feature. Headless/library consumers don't need TUI deps. The `clipboard`, `run`, `clip` commands and `sync` commands work without TUI.

### 2. Clipboard (`clipboard` feature)
**Dependencies**: `arboard` (Unix), `clipboard-win` (Windows)
**Currently used by**: `src/clipboard.rs`
**Recommendation**: Gate behind `clipboard` feature. Useful for library consumers who don't need clipboard access. The binary always enables it.

### 3. Sync (`sync` feature)
**Dependencies**: `tonic`, `prost`, `tonic-prost`, `aes-gcm`, `argon2`, `base64`, `keyring`, `sha2`, `zeroize`, `semver`
**Currently used by**: `src/sync.rs`, `src/sync_commands.rs`, `src/encryption.rs`, `src/config.rs` (sync settings)
**Recommendation**: Gate behind `sync` feature. The entire sync client + encryption stack is unnecessary for local-only use. This is the largest removable dependency surface (~15 crates).

### 4. Self-update (`self-update` feature)
**Dependencies**: `tempfile` (already used elsewhere), HTTP fetching (via existing tonic or manual)
**Currently used by**: `src/update.rs`
**Recommendation**: Gate behind `self-update` feature. Library consumers don't need self-update capability.

### 5. Bundled themes (`bundled-themes` feature)
**Dependencies**: `lzma-rs`, `unicode-width` (already used by TUI)
**Currently used by**: `scripts/build_themes.py` → `src/ui/_generated_bundled_themes.rs`
**Recommendation**: Gate behind `bundled-themes` feature. The `include_str!` bundle adds binary size. Library consumers don't need themes.

### 6. Auto-sync background system (`auto-sync` feature)
**Dependencies**: Uses `tokio` (already required), filesystem operations, process spawning
**Currently used by**: `src/auto_sync/` module tree
**Recommendation**: Gate behind `auto-sync` feature. Library consumers don't need background sync workers.

---

## Platform-Specific Dependencies

```toml
[target.'cfg(windows)'.dependencies]
clipboard-win = "5.4"
windows-sys = { version = "0.59", features = ["Win32_System_Threading", "Win32_Foundation"] }

[target.'cfg(not(windows))'.dependencies]
arboard = "3"
signal-hook = "0.4"
libc = "0.2"
```

**Observations**:
- `signal-hook` 0.4 is only used on Unix; Windows uses `windows-sys` for process liveness
- `libc` is also a dev-dependency (for PTY tests)
- `clipboard-win` and `arboard` provide the same clipboard abstraction — good, properly split
- `windows-sys` is used for `GetExitCodeProcess` (Windows process liveness) — correctly platform-gated

---

## Potential Issues

1. **Empty feature labels removed (Phase 10)**: The previous `[features]` table contained empty labels
   that did not gate any dependencies. These were removed as misleading. Real feature gates could be
   added in the future if library consumers need to exclude subsystems, but the binary crate has no
   need for optional compilation.

2. **`tokio` is unconditional**: Even non-async features (TUI, clipboard) pull tokio. A feature gate for `async-runtime` could help, but `tokio` is so pervasive that gating it is impractical.

3. **Duplicate `sha2` versions**: Cannot be resolved without upgrading `sqlx` or pinning `sha2` to a compatible version. This is a transitive limitation.

4. **`tempfile` is duplicated** in `[dependencies]` and `[dev-dependencies]` — this is harmless but could be cleaned up.

---

## Recommended Feature Gate Priority

| Priority | Feature | Binary impact | Library impact |
|----------|---------|---------------|----------------|
| 1 | `sync` | None (always on) | Major — removes ~15 crates |
| 2 | `tui` | None (always on) | Moderate — removes 3-4 crates |
| 3 | `auto-sync` | None (always on) | Moderate — removes process spawning |
| 4 | `clipboard` | None (always on) | Minor — removes platform clipboard |
| 5 | `self-update` | None (always on) | Minor — removes HTTP for updates |
| 6 | `bundled-themes` | None (always on) | Minor — removes lzma-rs |
