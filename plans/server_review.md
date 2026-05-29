# Server (snip-sync) Module Review

## Document Accuracy

### Verified Correct
- Architecture diagram is accurate: gRPC (tonic) on :50051 + HTTP (axum) on :50050, sharing SnipSyncService
- Database uses SQLite via sqlx with in-memory support for tests
- API key storage uses Argon2id hashing with SHA-256 prefix for indexed lookup
- `migrate_plaintext_api_keys()` backfills hashes for legacy data
- gRPC RPCs match the proto definitions: Health, Register, GetSnippets, PushSnippets, Sync, CreateLibrary, ListLibraries, DeleteLibrary, ListPremadeLibraries, GetPremadeLibrary
- Input validation constants match: max_command=1024, max_description=1024, max_tags=50, max_tag_length=100
- HTTP server exposes `/health` (no auth) and `/metrics` (Basic auth)
- Rate limiter is in-memory per-key sliding window (120 req/min default, cleanup every 60s)
- Metrics are Prometheus counters: requests_total, sync_operations_total, library_operations_total, rate_limit_hits, auth_failures
- PremadeManager scans directory for `.toml` files with path traversal prevention
- Config loaded from `config.toml` (or `CONFIG_PATH`), overridable by env vars
- CORS configurable via `CORS_ALLOWED_ORIGINS` env var or config file

### Discrepancies

1. **Schema mismatch — `users` table missing `device_id` column** (doc line 47 vs `db.rs:116-123`): The architecture doc lists `device_id` as a column in the `users` table. The actual schema has `id`, `api_key`, `api_key_prefix`, `created_at`, `updated_at` — no `device_id`. The `device_id` is returned from `Register` as a UUID generated in-memory and stored as the `users.id` field itself.

2. **Schema mismatch — `snippets` table missing from architecture `users` table** (doc line 47 vs `db.rs:134-143`): The `libraries` table actually has `deleted_at` (not mentioned in doc) and the doc doesn't list the `deleted_at` column.

3. **Schema mismatch — `snippets` table `deleted` is INTEGER, not bool** (doc line 49 vs `db.rs:160`): Doc says `deleted` but actual column is `deleted INTEGER NOT NULL DEFAULT 0`. This is correct for SQLite but the doc implies a boolean column type.

4. **Database file size claim** (doc line 39): Doc says "~1000 lines" — actual file is 1002 lines. Essentially correct.

5. **Rate limiter file size** (doc line 115): Doc says "47 lines" — actual file is 47 lines. Correct.

6. **Metrics file size** (doc line 124): Doc says "67 lines" — actual file is 67 lines. Correct.

7. **Premade file size** (doc line 140): Doc says "214 lines" — actual file is 214 lines. Correct.

8. **CORS behavior inconsistency** (doc line 111 vs `main.rs:954-956`): Doc says "Leave empty to allow all origins" (in `config.toml:55`). Code at `main.rs:954-956` logs "requests from any origin will be allowed" when origins is empty, but then at `main.rs:998-1003` when origins is empty it creates `CorsLayer::new()` with NO allow_origin — which actually **blocks all cross-origin requests**. The config comment and log message are misleading; the actual behavior is deny-all when origins is empty.

---

## Bugs & Issues

### Critical

1. **CORS misconfiguration — empty origins blocks requests, not allows all** (`main.rs:998-1003`): When `cors_allowed_origins` is empty (the default per `config.toml:55`), `CorsLayer::new()` is used which has NO allowed origins. The log message at line 955-956 says "requests from any origin will be allowed" and the config comment says "Leave empty to allow all origins" — both are wrong. This will cause browsers to reject cross-origin requests. The log message at lines 999-1001 contradicts the earlier log, suggesting this was noticed but not fixed.

2. **TOCTOU race in path traversal check** (`premade.rs:191-198`): The code canonicalizes the path then checks `starts_with`, but then calls `fs::read_to_string` on the *non-canonicalized* `path` (line 207). Between the check and the read, the filesystem could change (symlink race). The comment on line 190 acknowledges this but the code doesn't fully mitigate it — it should read from the canonicalized path instead.

3. **Rate limiter not applied to `get_snippets` or `list_libraries`** (`main.rs:364-439`, `main.rs:706-754`): These endpoints perform no rate limiting, allowing unbounded read requests. While reads are less dangerous than writes, an attacker could still DoS the server with expensive queries. The architecture doc doesn't mention this asymmetry.

### High

4. **Registration uses client-supplied `device_id` as rate limit key, but `device_id` is not validated** (`main.rs:330-337`): The `RegisterRequest` includes a `device_id` field that is used as the rate limit key. A client can send any arbitrary `device_id` string to bypass rate limiting entirely. The `device_id` from `RegisterRequest` is completely unused — the server generates its own UUID and returns it.

5. **`register` rate-limits by `req.device_id` which is user-provided** (`main.rs:332`): Since the client controls `req.device_id`, they can rotate it to register unlimited users. Rate limiting on registration is effectively bypassed.

6. **`list_libraries` has no rate limiting** (`main.rs:706-754`): Unlike all other mutating endpoints, `list_libraries` skips rate limit checking. While the architecture doc lists all endpoints, it doesn't note this inconsistency.

7. **`list_premade_libraries` rate-limits AFTER auth check** (`main.rs:822-833`): Auth is checked first (line 812-820), then rate limit (line 822-833). For all other endpoints, rate limit is checked before auth. This means an invalid API key triggers both an auth failure metric AND a rate limit check (wasting a rate limit token for the invalid key).

8. **`get_premade_library` rate-limits AFTER auth check** (`main.rs:872-883`): Same ordering issue as `list_premade_libraries`.

### Medium

9. **`upsert_snippet` WHERE clause silently drops updates on same-timestamp** (`db.rs:478`): The `WHERE excluded.updated_at > snippets.updated_at` clause means if two devices push the same snippet ID with the same `updated_at`, the update is silently dropped. This could cause data loss during sync if two devices edit a snippet at exactly the same second.

10. **`sync` response always returns `skipped_count: 0` and `skipped_ids: []`** (`main.rs:651-652`): The `SyncResponse` fields `skipped_count` and `skipped_ids` are never populated, even when snippets fail validation or upsert. This provides no feedback to the client about partial failures.

11. **`sync` doesn't count skipped snippets** (`main.rs:583-610`): When validation fails or upsert fails during sync, the snippet is silently skipped. The response claims success. Clients have no way to know snippets were dropped.

12. **Default library not created with `deleted_at`** (`db.rs:202`): The default library INSERT doesn't set `deleted_at`, relying on NULL default. This is correct but could be made explicit for clarity.

13. **`push_snippets` returns `success: rejected == 0`** (`main.rs:526`): A single validation failure makes the entire response `success: false`, even if most snippets were accepted. The response includes counts, but the boolean may mislead clients.

14. **Inconsistent default limits** (`main.rs:406-408` vs `main.rs:613-616`): `get_snippets` defaults to limit=100 when not specified, while `sync` defaults to limit=1000. This inconsistency could cause unexpected behavior.

15. **`register` doesn't validate `device_id` in request** (`main.rs:328`): The `RegisterRequest.device_id` is completely ignored — the server generates its own. There's no validation that the client sent a meaningful device_id. This is a wasted field in the proto.

16. **No validation of `api_key` in non-register endpoints** (`main.rs:371-380`): If `req.api_key` is empty, `get_user_by_api_key("")` will compute a prefix from an empty string and attempt verification. This is wasteful but not a bug since it returns None.

### Low

17. **`verify_snippet_ownership` is dead code** (`db.rs:374-388`): Marked `#[allow(dead_code)]` and never called in the server. It exists but serves no purpose in snip-sync.

18. **`DbError::Unauthorized` is dead code** (`db.rs:22-23`): The `Unauthorized` variant is defined but never constructed anywhere in the codebase.

19. **`DbError::Conflict` overused for validation** (`db.rs:244-253`): Library name validation errors are reported as `Conflict`, but they're really validation errors. This results in `Status::invalid_argument` in the gRPC handler (`main.rs:701`) which is correct, but the internal error type is misleading.

20. **Argon2 memory cost is very low** (`db.rs:12`): `ARGON2_MEMORY_KIB = 1 << 6 = 64 KiB` is extremely low for password hashing. OWASP recommends at least 19 MiB (19456 KiB) for Argon2id. This makes API keys easier to brute-force if the database is compromised.

21. **No maximum message size on gRPC server** (`main.rs:1080-1086`): The `tonic::transport::Server::builder()` doesn't call `max_decoding_message_size()`. The default is 4MB but this should be explicit and possibly lower given snippet sizes.

22. **`fix_invalid_toml_escapes` only handles `\<` and `\>`** (`premade.rs:10-66`): Other invalid TOML escapes (like `\n` in non-literal strings) are passed through unchanged, which could cause parsing failures for other escape sequences.

23. **`list_libraries` snippet count subquery is expensive** (`db.rs:293`): For each library, a correlated subquery counts non-deleted snippets. With many libraries, this is O(n) separate queries. A JOIN or window function would be more efficient.

---

## Design Issues

### Tight Coupling

1. **`SnipSyncService` contains all business logic** (`main.rs:246-308`): The service struct holds DB, rate limiter, config, metrics, and premade manager. All RPC handlers live in a single `impl` block with duplicated auth+rate-limit boilerplate. Extracting middleware or helper functions would reduce duplication.

2. **Auth + rate-limit boilerplate repeated in every RPC** (`main.rs:328-362`, `main.rs:446-469`, etc.): The pattern of rate-limit check → auth check → library ownership check is copy-pasted across 7 endpoints. A tonic interceptor or middleware layer would centralize this.

### Unclear Responsibilities

3. **`Config::ensure_config_file()` runs before `Config::load()`** (`main.rs:924-925`): `ensure_config_file` creates a default config if none exists, then `load()` reads it. If `load()` fails to parse the just-created file, it silently falls back to defaults. The default file should always parse correctly, but this ordering is fragile.

4. **`AppState` is unused for gRPC** (`main.rs:985-988`): `AppState` is created for the axum routes but the gRPC service doesn't use it. The `config` and `metrics` are passed directly to `SnipSyncService`. This split is unnecessary.

### Dead Code

5. **`verify_snippet_ownership` is unused** (`db.rs:374-388`): Should be removed or integrated into the service.

6. **`DbError::Unauthorized` is unused** (`db.rs:22-23`): Should be removed.

7. **`record_request` takes `_method` string but ignores it** (`main.rs:255`): The method name is logged nowhere; it's a no-op parameter.

---

## Security Concerns

### Critical

1. **No TLS** (`main.rs:920-922`): The server explicitly warns "TLS is not enabled" but provides no built-in option. API keys are transmitted in plaintext over gRPC. Production deployments must use a reverse proxy, but there's no enforcement or documentation of this requirement. API keys in transit are vulnerable to interception.

2. **Registration rate limit is bypassable** (`main.rs:330-337`): Client-controlled `device_id` makes rate limiting on registration useless. An attacker can register unlimited users.

3. **Argon2 memory cost is dangerously low** (`db.rs:12`): 64 KiB is far below OWASP minimum of 19 MiB. If the database is leaked, API keys are significantly easier to brute-force.

4. **Metrics endpoint with weak/no credentials** (`config.toml:44-50`): The config template comments out credentials, and the warning about empty strings is buried. Default deployment has metrics disabled (good), but the endpoint returns 404 rather than 403, which leaks whether credentials are configured.

### High

5. **API key transmitted in plaintext in gRPC metadata** (`snip_proto/src/snip_proto.rs:5`): API keys are sent as string fields in proto messages. Without TLS, these are visible on the wire.

6. **No request size limits on gRPC** (`main.rs:1080-1086`): While tonic has a default 4MB decode limit, there's no explicit `max_decoding_message_size` or `max_encoding_message_size` set. Large payloads could consume memory.

7. **`get_premade_library` reads entire file into memory** (`premade.rs:207-208`): `fs::read_to_string` loads the entire file. A malicious or oversized TOML file could cause memory exhaustion. No size limit is enforced.

8. **SQL injection not possible** but worth noting: All queries use parameterized bindings via sqlx, which is correct.

### Medium

9. **No CSRF protection on HTTP endpoints** (`main.rs:1066-1078`): The `/health` and `/metrics` endpoints don't validate Origin headers beyond CORS. Since `/health` is unauthenticated, this is low risk, but `/metrics` should ideally also check Origin.

10. **API key prefix collision** (`db.rs:98-101`): With only 8 hex characters of base64-encoded SHA-256, the prefix space is ~4 billion. With many users, prefix collisions increase, degrading the index-based lookup to a linear scan of matching-prefix rows.

11. **`get_user_by_api_key` falls back to full table scan** (`db.rs:224`): `OR api_key_prefix IS NULL` means for legacy rows without prefixes, every auth check scans all NULL-prefix rows. With many migrated users, this is O(n) per auth check.

---

## Performance Issues

1. **Rate limiter holds Mutex across entire `allow()` call** (`rate_limiter.rs:34`): Every request acquires the global mutex. Under high concurrency, this becomes a bottleneck. A sharded or lock-free approach would scale better.

2. **Rate limiter never shrinks HashMap** (`rate_limiter.rs:22-25`): The cleanup task retains keys with any timestamps in the window. Keys with empty vectors are removed, but the HashMap itself never shrinks. Over time with many unique keys, memory grows unboundedly.

3. **`list_libraries` N+1 query** (`db.rs:291-302`): For each library row, a correlated subquery counts snippets. With 100 libraries, this executes 101 SQL queries. Should use a JOIN or batch count.

4. **`migrate_plaintext_api_keys` loads all users** (`db.rs:510-513`): Fetches all users into memory. For large user bases, this could be problematic. Should use batched/paginated migration.

5. **No connection pooling configuration** (`db.rs:112`): `SqlitePool::connect` uses default pool settings. For production, `max_connections` should be configurable.

6. **`fix_invalid_toml_escapes` scans entire file content** (`premade.rs:10-66`): This is O(n) in file size for every premade library load. For large files, this adds startup latency.

---

## Test Coverage Gaps

1. **No integration tests for snip-sync**: The `snip-sync/tests/` directory doesn't exist. All tests are unit tests in `db.rs`.

2. **No tests for gRPC handlers**: Zero test coverage for any `SnippetSync` trait implementation. The auth flow, rate limiting, validation, sync logic, and error handling in `main.rs` are completely untested.

3. **No tests for HTTP endpoints**: `/health` and `/metrics` handlers are untested. Metrics auth flow is untested.

4. **No tests for RateLimiter**: The `RateLimiter` struct has no unit tests despite being security-critical.

5. **No tests for PremadeManager**: Path traversal prevention, TOML escaping, file scanning — all untested.

6. **No tests for Config loading**: Config file parsing, env var override precedence, defaults — untested.

7. **No tests for CORS behavior**: CORS configuration and layer construction are untested.

8. **No test for `upsert_snippet` same-timestamp edge case**: The test at `db.rs:728-769` tests older timestamps but not equal timestamps.

9. **No test for concurrent rate limit access**: RateLimiter uses Mutex but concurrent access patterns aren't tested.

10. **No test for `fix_invalid_toml_escapes` with single-quoted strings containing `'`**: The function has a fallback path at line 44-54 that doubles backslashes when single-quote fix isn't possible, but this path has no dedicated test.

---

## Priority Ranking

| Priority | ID | Description | Location |
|----------|----|-------------|----------|
| **Critical** | 1 | CORS empty origins blocks instead of allowing all — misconfigures default deployment | `main.rs:998-1003`, `config.toml:55` |
| **Critical** | 2 | Registration rate limit bypassable via client-controlled device_id | `main.rs:330-337` |
| **Critical** | 3 | Argon2 memory cost 64 KiB — far below OWASP minimum 19 MiB | `db.rs:12` |
| **Critical** | 4 | No TLS — API keys transmitted in plaintext | `main.rs:920-922` |
| **High** | 5 | TOCTOU race in premade path traversal check | `premade.rs:191-207` |
| **High** | 6 | Rate limiter not applied to get_snippets, list_libraries | `main.rs:364-439, 706-754` |
| **High** | 7 | Auth+rate-limit boilerplate duplicated across 7 endpoints | `main.rs` (all RPC handlers) |
| **High** | 8 | sync response never reports skipped/failed snippets | `main.rs:651-652` |
| **High** | 9 | Same-timestamp upsert silently drops updates | `db.rs:478` |
| **Medium** | 10 | Rate limiter mutex is global bottleneck | `rate_limiter.rs:34` |
| **Medium** | 11 | list_libraries N+1 query pattern | `db.rs:291-302` |
| **Medium** | 12 | No gRPC max message size set explicitly | `main.rs:1080-1086` |
| **Medium** | 13 | get_premade_library reads entire file unbounded | `premade.rs:207-208` |
| **Medium** | 14 | migrate_plaintext_api_keys loads all users into memory | `db.rs:510-513` |
| **Medium** | 15 | Inconsistent default limits (100 vs 1000) | `main.rs:406-408, 613-616` |
| **Low** | 16 | Dead code: verify_snippet_ownership, DbError::Unauthorized | `db.rs:374-388, 22-23` |
| **Low** | 17 | record_request ignores method name | `main.rs:255` |
| **Low** | 18 | AppState created but not used by gRPC | `main.rs:985-988` |
| **Low** | 19 | No tests for gRPC handlers, HTTP endpoints, rate limiter, premade manager | (all of `snip-sync/src/`) |

---

## Recommendations

### Immediate Fixes (Critical)
1. Fix CORS: when `cors_allowed_origins` is empty, use `CorsLayer::very_permissive()` or `CorsLayer::new().allow_origin(Any)` to match documented behavior, OR update docs/config to reflect deny-all behavior.
2. Fix registration rate limiting: rate limit by IP address or a server-generated token, not client-provided device_id.
3. Increase Argon2 memory cost to at least 19456 KiB (19 MiB) per OWASP guidelines.
4. Document TLS requirement prominently and add optional built-in TLS support via `tonic`'s TLS features.

### Short-Term (High)
5. Fix TOCTOU in `premade.rs`: read from canonicalized path after validation.
6. Add rate limiting to `get_snippets` and `list_libraries`.
7. Extract auth+rate-limit middleware to reduce boilerplate.
8. Populate `skipped_count` and `skipped_ids` in sync response.
9. Handle same-timestamp upserts explicitly (e.g., tie-break by device_id).

### Medium-Term (Medium)
10. Shard the rate limiter or use a lock-free data structure.
11. Replace N+1 query in `list_libraries` with a JOIN.
12. Set explicit `max_decoding_message_size` on gRPC server.
13. Add file size limit for premade library reading.
14. Paginate `migrate_plaintext_api_keys`.
15. Make default limits consistent (100 everywhere).

### Cleanup (Low)
16. Remove dead code (`verify_snippet_ownership`, `DbError::Unauthorized`).
17. Remove unused `_method` parameter from `record_request`.
18. Remove `AppState` or consolidate with `SnipSyncService`.
19. Add integration tests covering gRPC handlers, HTTP endpoints, rate limiter, and premade manager.
