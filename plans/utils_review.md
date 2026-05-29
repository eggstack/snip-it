# Utils Module Review

**Reviewed files**: `src/utils/mod.rs`, `src/utils/config.rs`, `src/utils/variables.rs`, `src/utils/toml_helpers.rs`, `src/utils/shell_keywords.rs`
**Architecture doc**: `architecture/utils.md`

---

## 1. Document Accuracy

### Verified Correct

- Module index matches: `config`, `variables`, `toml_helpers`, `shell_keywords` — all present in `src/utils/`.
- Variables syntax: `<name>`, `<name=default>`, `\<`/`\>` escapes — all implemented in `extract_variable_tokens()` (`src/utils/variables.rs:17-66`).
- Functions listed in doc all exist with correct signatures: `parse_variables`, `extract_variables_for_display`, `expand_command`, `strip_escape_sequences`.
- `strip_escape_sequences` correctly converts `\<` → `<` and `\>` → `>` (`src/utils/variables.rs:13-15`).
- TOML helper functions exist with documented purposes: `fix_invalid_toml_escapes`, `quote_strings_containing_backslashes`.
- TOML strategy description matches code: check for problematic backslashes, convert to single-quoted or escape with `\\` (`src/utils/toml_helpers.rs:19-49`).
- Regex pattern `r#""([^"\\]*(?:\\.[^"\\]*)*)""#` confirmed in code (`src/utils/toml_helpers.rs:15`).
- `SHELL_KEYWORDS_SET` is a `LazyLock<HashSet<&str>>` (`src/utils/shell_keywords.rs:198-199`).
- Config path functions all exist: `get_config_dir`, `get_config_path`, `get_snippets_path`, `get_sync_config_path`, `get_legacy_macos_config_dir`, `migrate_macos_config_dir`.
- `get_config_dir()` uses `$XDG_CONFIG_HOME` with fallback to `~/.config/snp` (`src/utils/config.rs:3-14`).

### Discrepancies

1. **Doc claims "19 tests" for variables** (`architecture/utils.md:52`) but the actual count is **14 tests** in `src/utils/variables.rs:158-276`. The doc overstates coverage.

2. **Doc says `shell_keywords.rs` is "(199 lines)"** (`architecture/utils.md:82`) — this is correct. But the module doc comment in `mod.rs:7` describes it as "Shell keyword expansion" when it is actually a **keyword set for syntax highlighting**, not expansion. The set is used in `ui.rs:225,312` for coloring, not for expanding anything.

3. **Doc doesn't document `copy_recursively()`** (`src/utils/config.rs:37-48`) — a private helper used by `migrate_macos_config_dir`. Minor omission.

---

## 2. Bugs & Issues

### Critical

- **None found.**

### High

1. **Unmatched `<` without `>` creates phantom variables and loses the `<` character.** In `extract_variable_tokens()` (`src/utils/variables.rs:38-63`), if the input contains `<foo` (no closing `>`), the while loop consumes all remaining characters into `var_content`. Since `var_content` is non-empty, a variable `(foo, None)` is pushed to the result. In `expand_command()`, the `<` is consumed from the char iterator but never pushed to the result output — the `<` is silently dropped from the output string. Example: `expand_command("echo <foo", &[])` would produce `"echo foo"` instead of `"echo <foo"`. While snippet commands are typically well-formed, malformed input should degrade gracefully, not silently corrupt output.

2. **Unmatched `>` is silently passed through.** In `extract_variable_tokens()`, a bare `>` that isn't preceded by `<` is just pushed as a normal character via the implicit else path — no special handling. This is correct for parsing, but `expand_command()` has the same behavior: a bare `>` is pushed to result as-is. This is fine, but the asymmetry (unmatched `<` is destructive, unmatched `>` is not) is worth noting.

### Medium

3. **`expand_command` re-parses the command to extract token names, then re-parses during expansion.** `extract_variable_tokens` is called once to build the `tokens` vec (line 89-92), then the expansion loop independently re-reads `<var>` content (line 119-127). The two parses must agree on token order and content, which they do — but any future divergence between them would introduce subtle bugs. A single-pass approach would be more robust.

4. **`expand_command` uses `usage_count` HashMap to handle repeated variables** (`src/utils/variables.rs:96-97`). When a variable appears multiple times (e.g., `<a> and <a>`), the first occurrence uses the first value, the second falls back to the variable name. This is because `token_idx` advances linearly through tokens, but the values lookup uses `usage_count` to pick the nth match. If the user provides only one value for a variable used twice, the second occurrence silently becomes the literal variable name rather than reusing the value. This may be intentional (encouraging unique values) but could surprise users.

5. **`fix_invalid_toml_escapes` and `quote_strings_containing_backslashes` only handle single-line strings.** The regex won't match TOML triple-quoted strings (`"""..."""`). This is documented as acceptable since snippet commands are single-line, but there's no guard — if a user somehow has a multi-line value, backslashes inside it won't be processed. The test at line 165-178 documents this behavior.

6. **`migrate_macos_config_dir` silently skips files that already exist at the destination** (`src/utils/config.rs:72-74`). If migration is interrupted after creating the new directory but before all files are moved, a subsequent run won't re-attempt the skipped files because `get_legacy_macos_config_dir()` returns `None` when the new dir already exists (`src/utils/config.rs:25-27`). This leaves some files in the legacy directory permanently.

7. **`migrate_macos_config_dir` silently ignores removal errors** (`src/utils/config.rs:80-83`). The `let _ = std::fs::remove_dir_all(&src)` and `let _ = std::fs::remove_file(&src)` discard errors. While this is acceptable for best-effort migration, it means the legacy directory may retain files after "successful" migration, with no indication to the user.

### Low

8. **`extract_variables_for_display` test is shallow** (`src/utils/variables.rs:194-207`). Tests only assert that the result "contains" the variable name and "prompt"/"default" — they don't verify the exact format string. For example, the test at line 194 doesn't confirm the output is `"name (prompt)"` vs `"name(prompt)"`.

9. **TOML helper `test_backslash_and_single_quote_escaped` test is hard to read** (`src/utils/toml_helpers.rs:96-99`). The raw string escaping makes it difficult to verify the expected output. Consider a comment or a simpler test case.

---

## 3. Design Issues

### Inconsistent Lazy Initialization

`toml_helpers.rs` uses `once_cell::sync::Lazy` (`src/utils/toml_helpers.rs:11`) while `shell_keywords.rs` uses `std::sync::LazyLock` (`src/utils/shell_keywords.rs:2`). Since the project targets Rust 1.81+ (per `Cargo.toml`), `LazyLock` is stable and preferred. The `once_cell` dependency could be removed from this module, or the project could standardize on one approach.

### Circular-ish Dependency: `utils::variables` → `ui::Variable`

`variables.rs` imports `crate::ui::Variable` (`src/utils/variables.rs:6`). This creates a dependency from a utility module to the UI layer. The `Variable` struct (`src/ui.rs:120-124`) is a simple data struct (`name: String, default: Option<String>`) that could live in `utils` or a shared types module instead. The current arrangement means `variables.rs` can't be used independently of the UI crate.

### Duplicated Parsing Logic

The angle-bracket parsing logic is duplicated in three places:
- `extract_variable_tokens()` in `src/utils/variables.rs:17-66`
- `expand_command()` in `src/utils/variables.rs:88-155` (re-parses independently)
- `highlight_snippet_line()` in `src/ui.rs:243-249` (independently parses `<var>` for syntax highlighting)

Each has its own backslash tracking and `<`/`>` detection. A shared tokenizer would reduce duplication and divergence risk.

### `mod.rs` Re-exports Are Inconsistent

`mod.rs:14-16` re-exports `expand_command`, `extract_variables_for_display`, `parse_variables`, `strip_escape_sequences` from `variables`. But `config`, `toml_helpers`, and `shell_keywords` are not re-exported — callers must use full paths like `crate::utils::config::get_config_dir()`. This inconsistency means some utils are accessed as `crate::utils::foo()` and others as `crate::utils::module::foo()`.

### `shell_keywords.rs` Contains Non-Command Entries

`SHELL_KEYWORDS` includes shell builtins and subcommands that aren't standalone executables:
- `daemon-reload` (line 90) — a `systemctl` subcommand, not a standalone binary
- `case`, `esac`, `select`, `function` (lines 136-139) — bash syntax keywords, not commands
- `return`, `break`, `continue` (lines 127-128) — shell flow control, not executables

This is fine for syntax highlighting purposes (they should be colored as keywords), but the name "shell keywords" is slightly misleading — it's more accurately "tokens to highlight as keywords."

---

## 4. Security Concerns

- **No input sanitization in `expand_command` or `strip_escape_sequences`.** These functions operate on user-owned snippet data, so this is expected. However, `expand_command` inserts user-provided values directly into the command string without escaping. This is by design (the command is executed as-is), but it means variable values can inject arbitrary shell syntax. This is the intended behavior for a snippet tool, not a vulnerability per se, but worth documenting.

- **`migrate_macos_config_dir` uses `std::fs::copy` which preserves file permissions.** If legacy config files have restrictive permissions, they're preserved in the new location. This is fine, but the new directory is created with `create_dir_all` which uses default permissions.

- **TOML helper regex operates on raw file content.** The regex is safe against ReDoS — the pattern `([^"\\]*(?:\\.[^"\\]*)*)` is linear because `\\.` consumes exactly two characters, preventing catastrophic backtracking.

---

## 5. Performance Issues

- **`expand_command` calls `extract_variable_tokens` just to get the list of token names** (`src/utils/variables.rs:89-92`), discarding default values. For typical snippets with 0-3 variables, this is negligible. A combined parse-and-expand pass would avoid the redundant work, but the current approach is clearer and the performance impact is insignificant for snippet-sized strings.

- **`quote_strings_containing_backslashes` allocates a new String** and rebuilds the TOML content even when no strings contain backslashes. The regex iterates all double-quoted strings, checks each, and rebuilds. For large TOML files this could matter, but snippet files are small. Acceptable trade-off for correctness.

- **`SHELL_KEYWORDS_SET` linear search** (`src/ui.rs:312`): `shell_keywords.iter().any(|kw| word == *kw)` is O(n) over ~190 keywords for every word in the command. With HashSet, this should be `shell_keywords.contains(word)` for O(1) lookup. The current code iterates the HashSet's items instead of using the `contains` method.

---

## 6. Test Coverage Gaps

| Area | Gap |
|------|-----|
| `variables.rs` | No test for unmatched `<` without `>` (phantom variable + lost char) |
| `variables.rs` | No test for empty variable name like `<=default>` |
| `variables.rs` | No test for variable name with special chars like `<a-b>` |
| `variables.rs` | No test for deeply nested brackets like `<<a>>` |
| `variables.rs` | No test for `extract_variables_for_display` exact output format |
| `variables.rs` | `test_expand_command_with_default` (line 216-219) tests expansion with no values — the variable falls back to name "name", not the default "default". The test name is misleading. |
| `toml_helpers.rs` | No test for key without value (e.g., bare key on a line) |
| `toml_helpers.rs` | No test for string containing `\\<` (escaped backslash + `<`) in TOML |
| `toml_helpers.rs` | No test for consecutive double-quoted strings on same line |
| `config.rs` | No unit tests for `migrate_macos_config_dir`, `get_legacy_macos_config_dir`, or `copy_recursively` (filesystem-dependent, harder to test) |
| `config.rs` | Tests only check path suffixes, not full path structure or XDG compliance |
| `shell_keywords.rs` | No test that `SHELL_KEYWORDS_SET` contains all entries from `SHELL_KEYWORDS` |
| `shell_keywords.rs` | No test verifying no duplicates in `SHELL_KEYWORDS` |

---

## 7. Priority Ranking

| Priority | Issue | Location |
|----------|-------|----------|
| **High** | Unmatched `<` loses the character and creates phantom variable | `variables.rs:38-63`, `variables.rs:119-144` |
| **Medium** | `expand_command` uses linear scan on HashSet instead of `contains()` | `ui.rs:312` |
| **Medium** | `Variable` struct in `ui.rs` creates utils→ui dependency | `variables.rs:6` |
| **Medium** | Duplicated angle-bracket parsing in 3 locations | `variables.rs`, `ui.rs:243-249` |
| **Medium** | `once_cell::sync::Lazy` vs `std::sync::LazyLock` inconsistency | `toml_helpers.rs:11` vs `shell_keywords.rs:2` |
| **Medium** | `migrate_macos_config_dir` can't recover from interrupted migration | `config.rs:25-27`, `config.rs:72-74` |
| **Low** | `mod.rs` doc says "Shell keyword expansion" — inaccurate | `mod.rs:7` |
| **Low** | `daemon-reload` in shell keywords is a subcommand, not a binary | `shell_keywords.rs:90` |
| **Low** | `mod.rs` re-exports are inconsistent | `mod.rs:14-16` |
| **Low** | `test_expand_command_with_default` test name is misleading | `variables.rs:216` |
| **Low** | Doc claims 19 tests but only 14 exist | `architecture/utils.md:52` |

---

## 8. Recommendations

1. **Fix the unmatched `<` edge case** in `extract_variable_tokens` by tracking whether `>` was found. If not, push the consumed content back as literal text (or better, don't consume it at all by buffering).

2. **Move `Variable` struct to `utils` or a shared types module** to eliminate the `utils → ui` dependency. It's a plain data struct with no UI dependencies.

3. **Standardize on `LazyLock`** and remove the `once_cell` import from `toml_helpers.rs`.

4. **Use `HashSet::contains()`** instead of `.iter().any()` for keyword lookup in `ui.rs:312`.

5. **Fix the doc** to accurately state 14 tests (not 19) and describe `shell_keywords` as a "keyword set for syntax highlighting" rather than "expansion."

6. **Add a `>`-awareness check** to `extract_variable_tokens` so that an unclosed `<` is passed through as literal text rather than consumed.

7. **Consider a shared tokenizer** used by `variables.rs` and `ui.rs` to eliminate the duplicated angle-bracket parsing logic.
