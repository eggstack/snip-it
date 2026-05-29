# Utils Module Review — Improvement Plan

## Modules Reviewed
- `src/utils/variables.rs` (282 lines)
- `src/utils/toml_helpers.rs` (180 lines)
- `src/utils/shell_keywords.rs` (199 lines)
- `src/utils/config.rs` (134 lines)

---

## Claim Verification

### variables.rs

| Claim | Status | Notes |
|-------|--------|-------|
| 277 lines | ❌ Off by 5 | Actual: 282 lines |
| `expand_command` returns `SnipResult<String>` | ❌ Returns `String` | Falls back to var name (not error) |
| `expand_command` returns error for missing vars | ❌ Returns var name | No error returned |
| "Skips escaped `\<` sequences" during parsing | ⚠️ Drops entire `\<` | `expand_command` handles it differently — inconsistent |
| `\<` → `<` in results | ✅ Correct | `expand_command` strips escapes |
| Multiple uses of same var tracked by index | ✅ Correct | via `usage_count` HashMap + `nth(*count)` |
| Trailing backslash preserved | ✅ Correct | Tests confirm |
| 19 tests | ✅ Correct | All pass |

### toml_helpers.rs

| Claim | Status | Notes |
|-------|--------|-------|
| 180 lines | ✅ Correct | |
| `fix_invalid_toml_escapes`: converts `\</\>` strings to single-quoted | ✅ Correct | |
| `quote_strings_containing_backslashes`: converts backslash strings to single-quoted | ⚠️ Only if no single quotes | If single quotes present, backslash is escaped instead |
| Strategy: escape backslash if single quotes present | ✅ Correct | |
| Regex-only single-line strings | ✅ Correct | Trip-quoted ignored |

### shell_keywords.rs

| Claim | Status | Notes |
|-------|--------|-------|
| ~190 `LazyLock<HashSet<&str>>` | ✅ Correct (~190 entries) | Simple data module, no issues |

### config.rs

| Claim | Status | Notes |
|-------|--------|-------|
| All 6 functions correct | ✅ Correct | All match documented signatures |

---

## Bugs Found

### BUG 1: Escape sequence handling inconsistency (High Severity)

`extract_variable_tokens` and `expand_command` handle `\<` differently:

- `expand_command` (line 118-122): `\<` → `<` (outputs literal bracket)
- `extract_variable_tokens` (line 39-41): `\<` → drops the `<` entirely

**Impact**: `extract_variable_tokens("<host> and \<website>")` returns only `["host"]`. When `expand_command` later expands with `host=example.com`, the output is `"example.com and <website>"` — but `<website>` was silently dropped from the token extraction, and the literal `<website>` appears in output because `\<` was not recognized as an escape.

**Severity**: High — silently produces incorrect command output.

### BUG 2: Double-backslash before angle bracket loses a character (Medium Severity)

Input `\\<foo>` (literal backslash then variable):
- Process: `\` set flag, `\` push one backslash (line 111), `\<` the `<` is skipped
- Result: one `\` + `foo` + `>`

Expected: `\<foo>` (escaped `<` becomes literal `<`, so `\<foo>` → `<foo>`)  
Actual: `\foo>`

**Severity**: Medium — corrupts commands that use `\\<` for "literal backslash then variable" pattern.

### BUG 3: `expand_command` return type mismatch (Documentation)

Documented as `SnipResult<String>` (can error on missing variables).  
Actual: Returns `String`, falls back to variable name as placeholder.

This is actually **reasonable behavior**, but documentation is wrong.

---

## Edge Cases (Undocumented)

### Edge Case 1: Chained backslash escapes
`\\\` — three backslashes:
- `\` + `\` → handled as escape pair (one pushed, flag reset, second processed)
- `\` + end → flag set, appended at line 155-157
- Result: `\\` 

One backslash "lost" vs original (3 backslashes became 2). Partial fix attempted but incomplete.

### Edge Case 2: Nested angle brackets without proper closing
Input `echo <<foo>`:
- First `<` matches with first `>`, extracts `foo` as variable.
- Second `<` starts new variable, no closing `>` found.
- Variable `foo` extracted, but second `<` silently dropped.

Actual output would be `echo foo` with no warning.

### Edge Case 3: Backslash at end of variable content
Input `<foo\>`: The backslash is processed when we see `\`. Then when we see `>`, we exit the inner loop, var_content is `foo`, but the backslash has been lost.

Actual: `<foo>` → variable named `foo` (backslash silently dropped).

---

## Potential Improvements

### 1. Fix `\<` inconsistency between parse and expand
Make `extract_variable_tokens` call `continue` (consume both) but track that an escaped `<` was consumed so `expand_command` doesn't output an extra `<`. Or better: normalize `\<` during extraction to a sentinel token that `expand_command` recognizes.

### 2. Make `expand_command` return `SnipResult<String>`
If a variable has no default and no user-provided value, return an error instead of silently substituting the variable name. This prevents malformed commands from executing.

### 3. Add warning for unmatched `<` during expansion
Track unmatched angle brackets and warn the user rather than silently dropping them or creating phantom variables.

### 4. Improve TOML corruption detection in toml_helpers
The regex `r#""([^"\\]*(?:\\.[^"\\]*)*)""#` silently ignores malformed TOML (unclosed quotes, embedded newlines in double-quoted strings). While this is low risk for snippet commands (always single-line), a lint/warning could help detect config file corruption early.

### 5. Docstring for `quote_strings_containing_backslashes` is misleading
The doc says backslashes cause conversion to single-quoted, but if single quotes are present in the content, backslashes are escaped with `\\` in double quotes instead. The behavior is correct; the doc is incomplete.

### 6. `migrate_macos_config_dir` uses `eprintln!` directly
Could use the logging system for migration messages instead of direct stderr writes.

### 7. No test for double-backslash before angle bracket scenario
The test suite has 19 tests but none cover `\\<foo>` or `\\\\<foo>` edge cases.

---

## Summary

| Priority | Issue |
|----------|-------|
| High | BUG 1: `\<` behavior differs between parsing and expansion |
| Medium | BUG 2: `\\<` pattern loses characters |
| Low | Doc: `expand_command` return type mismatch |
| Low | Improve: warn on unmatched `<` |
| Low | Improve: `quote_strings_containing_backslashes` docstring |

**Recommended action**: Fix BUG 1 first — it's the most likely to cause silent data corruption in user snippets.
