# Architecture Review Skill

## Purpose
Guide agents through reviewing snip-it architecture documents against actual code.

## Review Process

### 1. Read the Architecture Document
```bash
cat architecture/<module>.md
```

### 2. Trace Claims to Code
For each claim in the document:
- Verify file paths exist
- Check struct definitions match
- Verify function signatures
- Confirm behavioral descriptions

### 3. Interrogate the Code
Look for:
- **Bugs**: Logic errors, edge cases, error handling gaps
- **Design Issues**: Tight coupling, unclear responsibilities, dead code
- **Security Concerns**: Especially in encryption, sync, server modules
- **Performance Issues**: Unnecessary allocations, O(n²) algorithms
- **Test Coverage Gaps**: Missing tests for critical paths

### 4. Write Findings
Output to `plans/<module>_review.md` with:
- Document Accuracy (verified correct + discrepancies)
- Bugs & Issues (with file:line locations)
- Design Issues
- Security Concerns
- Performance Issues
- Priority Ranking table (critical/high/medium/low)
- Recommendations

## Key Files to Check

| Module | Primary Source Files |
|--------|---------------------|
| overview | `src/main.rs`, project root |
| cli | `src/main.rs`, `src/commands/` |
| clipboard | `src/clipboard.rs` |
| config | `src/config.rs`, `src/utils/config.rs` |
| core | `src/library.rs`, `src/error.rs` |
| encryption | `src/encryption.rs` |
| logging | `src/logging.rs` |
| proto | `snip-proto/` |
| server | `snip-sync/src/` |
| sync | `src/sync.rs`, `src/sync_commands.rs` |
| ui | `src/ui/` |
| utils | `src/utils/` |

## Common Issues Found

1. **Argon2 memory cost**: Check `encryption.rs` for `ARGON2_MEMORY_COST_KIB`. Currently `1 << 14` (16 MiB). OWASP minimum is 19 MiB.
2. **Rate limiting gaps**: All endpoints should use `authenticate_and_rate_limit()`. Check `snip-sync/src/main.rs`.
3. **CORS configuration**: `CORS_ALLOW_ALL` env var enables permissive mode. When not set and no origins configured, cross-origin requests are blocked.
4. **Sync timestamp updates**: `last_sync` is NOT updated when encryption failures occur (`has_failures` check in `sync_commands.rs`).
5. **Dead code**: Look for `#[allow(dead_code)]`, unused variables prefixed with `_`, and unreachable branches.
6. **TOCTOU races**: File existence checks should use `fs::read_to_string()` error handling instead of `exists()` + `read()` patterns.
