# Remediation Patterns for snp

## Key Patterns Used in Remediation

### 1. Security: Keychain Integration (keyring crate)
- `keyring = "3"` for cross-platform OS keychain access
- `Entry::new(service, user)` to create credential entries
- `entry.set_password()` / `entry.get_password()` for storage/retrieval
- Graceful fallback: if keychain unavailable, store plaintext with warning
- Migration: detect plaintext on load, move to keychain, save marker

### 2. Security: Rate Limiting
- Rate limit check BEFORE auth check (cheaper operation first)
- Use server-controlled keys (IP address) not client-controlled (device_id)
- Pattern: `rate_limiter.allow(&key, limit, window).await`

### 3. Security: CORS Configuration
- Read env vars at server startup: `std::env::var("CORS_ALLOW_ALL")`
- `CorsLayer::new().allow_origin(Any)` for permissive mode
- Log configuration for debugging

### 4. Bug Fixes: Race Conditions
- Use generation counters (`AtomicU64`) instead of `AtomicBool`
- Increment counter on each new schedule
- Sleeping thread checks if its generation matches current counter
- Prevents stale timers from affecting new operations

### 5. Bug Fixes: Error Propagation
- Return `Err()` instead of silent defaults on data loss conditions
- Backup files before returning errors so callers can recover
- Use `?` operator for propagation in callers

### 6. Bug Fixes: Data Integrity
- Check for existing entries before inserting (prevent duplicates)
- Validate input parameters (e.g., interval >= 1)
- Use tie-breaking for concurrent updates (device_id as tiebreaker)

### 7. Code Quality: Extract Repeated Patterns
- Identify copy-pasted auth+rate-limit blocks
- Extract into helper method: `authenticate_and_rate_limit(&self, api_key)`
- Reduces code duplication and ensures consistency

### 8. Code Quality: Module Splitting
- Move independent types to appropriate modules (e.g., Variable struct)
- Break large files into submodules with re-exports
- Maintain public API via re-exports in mod.rs

### 9. Performance: SQL Optimization
- Replace correlated subqueries with JOINs
- Use `LEFT JOIN ... GROUP BY` for counts
- Add indexes for frequently queried columns

### 10. Clippy Compliance
- Use `sort_by_key` instead of `sort_by` for simple key extraction
- Collapse nested `if` into match arm guards where practical
- Use `#[allow(clippy::...)]` for complex patterns that can't be collapsed

## Testing Approach
- Unit tests for individual functions
- Integration tests with TempDir for file system operations
- Server tests with `sqlite::memory:` for database isolation
- Run `cargo clippy --all-targets -- -D warnings` before committing
- Run `cargo fmt --check` to verify formatting
