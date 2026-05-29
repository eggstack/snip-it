# CLI Module Review Plan

## Source Files Reviewed

- `src/main.rs` (331 lines) — CLI definition, signal handling, command dispatch
- `src/commands/mod.rs` (271 lines) — Shared helpers, TOML load/save, selection loop
- `src/commands/run_cmd.rs` (161 lines)
- `src/commands/clip_cmd.rs` (41 lines)
- `src/commands/search_cmd.rs` (21 lines)
- `src/commands/new_cmd.rs` (94 lines)
- `src/commands/list_cmd.rs` (61 lines)
- `src/commands/edit_cmd.rs` (130 lines)
- `src/commands/keybindings_cmd.rs` (39 lines)
- `src/commands/sync_cmd.rs` (253 lines)
- `src/commands/cron_cmd.rs` (48 lines)
- `src/commands/register_cmd.rs` (74 lines)
- `src/commands/library_cmd.rs` (103 lines)
- `src/commands/premade_cmd.rs` (154 lines)
- `architecture/cli.md` (86 lines) — Architecture doc being verified

---

## Document Accuracy

### Verified Correct

- `main.rs` is 331 lines — matches doc claim
- `commands/mod.rs` is 271 lines — matches doc claim
- All 13 subcommand modules exist as claimed
- Command dispatch via `dispatch_command()` confirmed at `main.rs:224`
- Global state: `CONFIG_PATH: LazyLock<PathBuf>` (`main.rs:41`), `RUNTIME: LazyLock<Runtime>` (`main.rs:43`)
- Panic handler + signal handlers installed before CLI parsing (`main.rs:321-323`)
- `run_snippet_selection` pattern described in doc matches implementation in `commands/mod.rs:216-271`
- Async commands (run, clip, search, sync, register, premade) all receive `&RUNTIME` — verified
- Non-async commands (version, new, list, edit, keybindings, cron, library) do not touch `RUNTIME` — verified
- All subcommand aliases match doc table (v, n, l, r, c, s, e, k, y, reg, lib, p)
- Library subcommands: list (l), create (c), delete (d), set-primary (p), show (s) — verified
- Premade subcommands: list (l), get, sync (s) — verified

### Discrepancies

1. **`sync` command: `--servers` is not documented in architecture doc table.** The doc shows `sync` as a single entry but doesn't mention the `--servers`, `--non-interactive`, `--push-only`, `--pull-only` flags. These are significant user-facing options. (`main.rs:155-162`)

2. **`new` command: `--tags` flag behavior not documented.** The doc says `new` takes `(interactive stdin)` but doesn't mention the `-t/--tags` flag that prompts for tags. (`main.rs:82-83`)

3. **`list` command: `--filter` flag not mentioned in doc table.** The doc shows no flags for `list`. (`main.rs:94-95`)

4. **Architecture doc says `search_cmd.rs` description is "TUI select → display snippet info".** This is correct but the command also accepts `--sync` and `--filter` flags not mentioned in the doc. (`main.rs:125-136`)

5. **`register` command default server URL not documented.** Default is `http://localhost:50051` (`main.rs:173`).

6. **`cron` command interval default not documented.** Default interval is 15 minutes (`main.rs:167`).

---

## Bugs & Issues

### Critical

**B1. `sync_cmd.rs:224-251` — Duplicate sync execution on `list_libraries` failure.**

When `client.list_libraries()` fails at line 210, execution falls through (no `return`), hitting lines 237-251 which calls `list_and_link_server_libraries()` and then `run_sync()` again. This means sync runs even after a library listing failure, and potentially runs twice in the success path (once at line 224-232, again at line 244-251).

```
// Line 210: if list_libraries fails...
match runtime.block_on(client.list_libraries()) {
    Ok(libs) => { ... return Ok(()); }  // <-- returns
    Err(e) => eprintln!("..."),         // <-- does NOT return!
}
// Falls through to line 237-251 which also calls sync
```

**Fix:** Add `return Ok(());` after the `Err` branch at line 234, or restructure to avoid fall-through.

### High

**B2. `run_cmd.rs:36` — Dead code in `validate_output_path`.**

The `Component::Normal(c)` branch checks `c.to_string_lossy().contains("..")`, but `..` is always parsed as `Component::ParentDir`, never as `Component::Normal`. This branch can never match — the `..` string literal cannot appear inside a `Normal` component.

```rust
Component::Normal(c) => {
    if c.to_string_lossy().contains("..") {  // Dead code
        return Err(...)
    }
}
```

**B3. `cron_cmd.rs:167` / `main.rs:167` — No validation for `interval == 0`.**

`interval` is `u32` with `default_value = "15"`. If a user passes `--interval 0`, the crontab entry becomes `*/0 * * * *` which is invalid cron syntax (minute field must be 1-59 for step values). No validation exists.

**B4. `run_cmd.rs:102-103`, `run_cmd.rs:110-111` — Shell execution output not captured on error.**

When a snippet command fails, only the exit code is logged. Stderr from the shell is not captured or displayed, making debugging difficult for users. `Command::new(&shell).arg("-c").arg(&final_command).status()` discards stderr.

### Medium

**B5. `commands/mod.rs:34-53` — `get_config_path` creates empty file as side effect.**

When a non-existent config path is provided via `--config`, the function creates the parent directory and an empty file before returning. This is an unexpected side effect for a path-resolution function. The caller may not expect filesystem mutations.

**B6. `new_cmd.rs:19-24` — `read_multiline_command` returns empty string for empty input.**

Two consecutive blank lines terminate multiline input. If the user presses Enter twice immediately (without typing anything), the function returns an empty string, creating a snippet with an empty command. No validation prevents this.

**B7. `edit_cmd.rs:7-18` — `_config` parameter silently ignored.**

The `edit` command accepts `--config` but never uses it (parameter is `_config: Option<PathBuf>`). The path is always resolved via `get_library_path(None)`, making the `--config` flag misleading. (`edit_cmd.rs:7`, `main.rs:270`)

**B8. `commands/mod.rs:102-141` — `load_snippets` returns `Ok(default)` on parse error.**

When TOML parsing fails, the function creates a backup but returns empty snippets with no error. This means `save_snippets` called later would overwrite the (backup'd) file with an empty collection, silently deleting user data. The backup saves the corrupted original, but the in-memory state is empty.

**B9. `register_cmd.rs:29-39` — `server_url` resolution is fragile.**

The check `if server != "http://localhost:50051"` is a string comparison against the default value. If the user passes the default URL explicitly, the code falls through to check existing settings, which may override the user's intent. This is brittle coupling to the default value.

**B10. `library_cmd.rs:91-103` — `StringExt::if_empty` trait leaks into module scope.**

The `StringExt` trait is defined for this module but should be scoped more narrowly or use a standard-library method. It could cause name collisions if other modules define similar traits.

### Low

**B11. `search_cmd.rs:1-3` — `_config` parameter unused.**

Like `run_cmd` and `clip_cmd`, the `--config` flag is accepted but the parameter is `_config: Option<PathBuf>` and never used. (`search_cmd.rs:9`, `main.rs:261-267`)

**B12. `new_cmd.rs:47` — Trailing newline not stripped from stdin input.**

`read_line` includes the trailing `\n`. The code does `trim()` on line 47, which correctly removes it, but `read_multiline_command` (line 24) joins lines without stripping newlines, preserving trailing whitespace in the final command.

**B13. `commands/mod.rs:236` — `load_library` called without using the `config` parameter.**

`run_snippet_selection` loads snippets via `load_library(&lib_path)` using `get_library_path`, completely ignoring any `config` argument. This is consistent with how the TUI commands work (they use library mode), but means `--config` has no effect for TUI commands.

---

## Design Issues

**D1. `_config` parameter accepted but unused in 4 commands.**

`run`, `clip`, `search`, and `edit` all accept `--config` but the parameter is prefixed with `_` and never used. The architecture doc documents `config` as a parameter for these commands. Either the parameter should be removed, or it should be wired through to the underlying operations. Currently it creates user confusion: `snp run --config /path/to/file` does nothing.

**D2. `sync_cmd.rs` has complex control flow with redundant sync paths.**

The `run` function in `sync_cmd.rs` creates the sync client up to 3 times:
1. Line 186-202 (for `--servers` flag)
2. Line 204-208 (main sync path)
3. Line 238 (`list_and_link_server_libraries` creates another client)

This results in redundant network connections and makes the control flow hard to follow.

**D3. `run_snippet_selection` creates `LibraryManager` on every invocation.**

Every TUI command (run, clip, search) creates a fresh `LibraryManager` via `get_library_path`. If the user runs multiple TUI commands in sequence, this results in repeated config file reads. A shared or cached manager would be more efficient.

**D4. `commands/mod.rs` mixes concerns.**

The module contains: path resolution (`get_config_path`, `get_library_path`), TOML serialization (`load_snippets`, `save_snippets`), TUI data extraction (`get_snippet_data`), variable expansion (`expand_snippet_command`), and the shared selection loop (`run_snippet_selection`). These are distinct responsibilities that could be separated.

**D5. `ProcessResult` and `ExpandedCommand` enums in different modules.**

`ProcessResult` is defined in `main.rs` (public, used across modules) while `ExpandedCommand` is defined in `commands/mod.rs`. Both represent command execution outcomes but live in different locations with inconsistent naming patterns.

**D6. Inconsistent error handling: `eprintln!` vs `SnipError::runtime_error`.**

Some error paths use `eprintln!` and continue (e.g., `init_library_manager` failures in `get_library_path:62`), while others propagate `SnipError`. The boundary is unclear: `get_library_path` prints a warning but continues, while `get_config_path` returns errors. This inconsistency makes error behavior unpredictable.

---

## Security Concerns

**S1. `run_cmd.rs:9-10` — Output path validation doesn't prevent symlinks.**

`validate_output_path` checks for `..` traversal and absolute paths, but doesn't validate that the resolved path doesn't follow symlinks to sensitive locations. A snippet with `output = "link_to_etc_passwd"` (where `link_to_etc_passwd` is a symlink to `/etc/passwd`) would bypass validation.

**S2. `edit_cmd.rs:26` — Editor resolution uses PATH search.**

`resolve_editor` searches PATH directories for the editor binary. If an attacker can place a malicious binary earlier in PATH, it would be executed. This is a standard PATH trust issue but worth noting for a tool that processes user-provided snippet data.

**S3. `register_cmd.rs:57-61` — API key partially displayed.**

The registration command masks the API key as `xxxx...xxxx`. If the key is short (<8 chars), it shows `****`. The masking is reasonable but the full key is stored in plaintext in `sync.toml`. Any process with filesystem access can read it.

**S4. `run_cmd.rs:107-111` — Arbitrary command execution via shell.**

The `run` command executes snippet commands via `sh -c`. While this is the intended behavior, there's no sandboxing or confirmation prompt beyond the TUI selection. Combined with the `--sync` flag that auto-syncs after execution, a malicious snippet could exfiltrate data via sync.

**S5. `sync_cmd.rs:172-178` — Sync settings loaded with `unwrap_or_default` fallback.**

When `load_sync_settings()` fails, a default `SyncSettings` (disabled, empty key) is used. This silently degrades rather than alerting the user. For `sync_cmd.rs:224` this means sync operations silently become no-ops.

---

## Performance Issues

**P1. `sync_cmd.rs` — Multiple `SyncClient::create` calls per sync.**

Each `list_libraries`, `sync_encrypted`, and `list_premade_libraries` call creates a new client. The `run_sync` function in `sync_cmd.rs` can create 2-3 clients for a single sync operation. Reusing a single client would reduce connection overhead.

**P2. `commands/mod.rs:236` — Library loaded from disk on every TUI loop iteration.**

`run_snippet_selection` loads snippets once, which is correct. But `get_library_path` creates a new `LibraryManager` and reads `libraries.toml` on each invocation. For repeated operations this adds unnecessary I/O.

**P3. `new_cmd.rs:76-81` — Library manager created twice on first snippet creation.**

`init_library_manager()` is called at line 76 (inside `get_library_path`), then `load_library` is called at line 78. The first creates a `LibraryManager` and reads config; the second loads the library file. These could be combined.

---

## Test Coverage

| Module | Has Tests | Coverage Notes |
|--------|-----------|----------------|
| `run_cmd.rs` | Yes | 5 tests: output path validation (relative, absolute, traversal, empty, dotfile) |
| `commands/mod.rs` | No | No tests for `load_snippets`, `save_snippets`, `get_config_path`, `get_library_path`, `expand_snippet_command`, `run_snippet_selection` |
| `clip_cmd.rs` | No | No tests |
| `search_cmd.rs` | No | No tests |
| `new_cmd.rs` | No | No tests for `read_multiline_command` or `run` |
| `list_cmd.rs` | No | No tests for fuzzy filtering logic |
| `edit_cmd.rs` | No | No tests for `resolve_editor` or `has_directory_component` |
| `keybindings_cmd.rs` | No | No tests |
| `sync_cmd.rs` | No | No tests for `link_server_library` or `prompt_conflict` |
| `cron_cmd.rs` | No | No tests |
| `register_cmd.rs` | No | No tests |
| `library_cmd.rs` | No | No tests |
| `premade_cmd.rs` | No | No tests |

**Critical gaps:**
- `commands/mod.rs` has zero tests for 7 public functions including `load_snippets` and `save_snippets` (which handle TOML error recovery)
- `sync_cmd.rs:prompt_conflict` has complex branching with no test coverage
- `edit_cmd.rs:resolve_editor` has 3 code paths (absolute, relative, PATH search) with no tests
- No integration tests exercise the full `dispatch_command` flow

---

## Priority Ranking

| ID | Severity | Summary | File:Line |
|----|----------|---------|-----------|
| B1 | **Critical** | Sync runs twice / falls through on list_libraries failure | `sync_cmd.rs:224-251` |
| D1 | **High** | `_config` flag accepted but silently ignored in 4 commands | `main.rs:229-271` |
| B2 | **High** | Dead code: `..` check in `Component::Normal` branch | `run_cmd.rs:36-40` |
| B3 | **High** | No validation for `--interval 0` in cron | `main.rs:167` |
| B4 | **High** | Shell stderr not captured on snippet execution failure | `run_cmd.rs:102-103` |
| B8 | **Medium** | `load_snippets` silently returns empty on parse error; subsequent save overwrites file | `commands/mod.rs:120-137` |
| B5 | **Medium** | `get_config_path` creates empty file as side effect | `commands/mod.rs:42-48` |
| B6 | **Medium** | Empty command allowed via double-Enter in multiline mode | `new_cmd.rs:16-24` |
| B9 | **Medium** | Server URL comparison is brittle string match against default | `register_cmd.rs:29` |
| D2 | **Medium** | Sync client created 2-3 times per operation | `sync_cmd.rs:186-238` |
| S1 | **Medium** | Output path validation doesn't check symlinks | `run_cmd.rs:10-48` |
| D6 | **Medium** | Inconsistent error handling (eprintln vs SnipError) | Multiple files |
| B7 | **Low** | `--config` on edit command ignored | `edit_cmd.rs:7` |
| B10 | **Low** | `StringExt` trait leaks into module scope | `library_cmd.rs:91-103` |
| B12 | **Low** | Multiline input preserves trailing newlines | `new_cmd.rs:24` |
| D3 | **Low** | `LibraryManager` created on every TUI command invocation | `commands/mod.rs:229` |
| D4 | **Low** | `commands/mod.rs` mixes path resolution, serialization, TUI, variables | `commands/mod.rs` |
| P1 | **Low** | Redundant SyncClient creation | `sync_cmd.rs` |

---

## Recommendations

### Immediate (Critical/High)

1. **Fix sync fall-through bug (B1):** Add `return Ok(());` after the `Err` branch at `sync_cmd.rs:234`, or restructure to use `if let Ok(libs)`.

2. **Remove or wire through `_config` parameter (D1):** Either remove `--config` from `run`, `clip`, `search`, `edit` (and the architecture doc), or pass it through to the underlying load operations. The current behavior is misleading.

3. **Add interval validation (B3):** Validate `interval >= 1` in `cron_cmd.rs` before generating the crontab entry. Return an error with a helpful message.

4. **Fix dead code (B2):** Remove the `Component::Normal(c)` branch containing `contains("..")` check in `run_cmd.rs:36-40`. It can never execute.

5. **Capture stderr on execution failure (B4):** Use `Command::new(&shell).arg("-c").arg(&final_command).output()` instead of `.status()`, then display stderr when the command fails.

### Short-term (Medium)

6. **Fix `load_snippets` error recovery (B8):** When parsing fails, either return an error (letting the caller decide), or store the parsed snippets in a way that doesn't result in data loss on next save. Consider adding a `--force` flag to allow overwriting corrupted files.

7. **Add symlink check to output path validation (S1):** After resolving the path, use `std::fs::canonicalize()` and verify the canonical path stays within expected bounds.

8. **Extract `resolve_editor` tests (D3 gap):** Add unit tests for the 3 code paths in `edit_cmd.rs:resolve_editor`.

9. **Refactor sync client creation (D2):** Create the `SyncClient` once in `run_sync` and pass it to helper functions, rather than creating it in each function.

10. **Document missing flags in architecture doc:** Update `architecture/cli.md` to include `--filter` for list, `--tags`/`--multiline` for new, `--servers`/`--non-interactive`/`--push-only`/`--pull-only` for sync, and defaults for register server and cron interval.

### Long-term (Low)

11. **Add tests for `commands/mod.rs`:** Cover `load_snippets` (empty file, valid file, corrupted file, file with escape sequences), `save_snippets` (roundtrip, backslash handling), and `get_config_path` (existing path, non-existing path, default path).

12. **Consider removing `--config` from TUI commands:** Since TUI commands (run, clip, search) always use library mode and ignore the config parameter, remove the flag entirely to avoid confusion.

13. **Scope `StringExt` trait (B10):** Either make it `pub(crate)` or replace with a standalone function to avoid namespace pollution.
