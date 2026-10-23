use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use base64::Engine;
use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::AssertSqlSafe;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

const ARGON2_MEMORY_KIB: u32 = 1 << 14; // 16 MiB — OWASP minimum for Argon2id
const ARGON2_ITERATIONS: u32 = 3;
const ARGON2_PARALLELISM: u32 = 4;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("Internal error: {0}")]
    Internal(String),
}

pub type DbResult<T> = Result<T, DbError>;

/// Result of [`Database::upsert_snippet_in_tx`].
///
/// `snippets.id` is a *global* primary key, so an insert that collides with a
/// row owned by a different user or library cannot be written at all. SQLite
/// reports a false `WHERE` on `DO UPDATE` as a silent no-op (0 rows affected,
/// no error), which is why the outcome has to be classified explicitly instead
/// of treating "no rows affected" as success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    /// The row was inserted or updated.
    Written,
    /// A newer version of the same record already exists under the same user
    /// and library; last-write-wins correctly kept it.
    Stale,
    /// A row with the same id already belongs to a different user and/or
    /// library. The write was discarded — reporting this as `Stale` would
    /// silently lose the snippet.
    ForeignScope,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Snippet {
    pub id: String,
    pub description: String,
    pub command: String,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub device_id: String,
    pub deleted: bool,
    pub encrypted: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Library {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub snippet_count: i64,
}

/// One page of snippets plus the paging facts derived from the page.
#[derive(Debug, Clone)]
pub struct SnippetPage {
    /// The rows of this page (never more than the requested limit).
    pub snippets: Vec<Snippet>,
    /// True when rows exist beyond this page.
    pub has_more: bool,
    /// Exact number of rows in the filtered range.
    pub total: i32,
}

/// The cursor the delta filter selects on.
///
/// The server keeps two cursors side by side. The **timestamp cursor**
/// (`visible_at`) is second-granular and stays the filter for clients that
/// send no sequence cursor. The **write-sequence cursor** (`sequence`) is
/// server-assigned, strictly increasing per library, and exact. The request
/// picks the form: a non-zero sequence cursor means the sequence form, and
/// anything else falls back to the timestamp form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaCursor {
    /// The timestamp cursor: `visible_at >= value`.
    Timestamp(i64),
    /// The write-sequence cursor: `sequence >= value`.
    Sequence(i64),
}

impl DeltaCursor {
    /// The SQL predicate and its bound value for this cursor form.
    ///
    /// The predicate is a literal, never caller input, so the assembled
    /// statement stays free of injected values.
    fn predicate(self) -> (&'static str, i64) {
        match self {
            DeltaCursor::Timestamp(value) => ("AND visible_at >= ?", value),
            DeltaCursor::Sequence(value) => ("AND sequence >= ?", value),
        }
    }
}

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

fn hash_api_key(api_key: &str) -> DbResult<String> {
    let mut salt_bytes = [0u8; 16];
    getrandom::fill(&mut salt_bytes)
        .map_err(|e| DbError::Internal(format!("Failed to generate salt: {}", e)))?;
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        None,
    )
    .map_err(|e| DbError::Internal(format!("Invalid Argon2 params: {}", e)))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let hash = argon2
        .hash_password_with_salt(api_key.as_bytes(), &salt_bytes)
        .map_err(|e| DbError::Internal(format!("Failed to hash API key: {}", e)))?
        .to_string();
    Ok(hash)
}

fn verify_api_key(api_key: &str, hash: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let params = match Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        None,
    ) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    argon2.verify_password(api_key.as_bytes(), &parsed).is_ok()
}

/// Computes the first 8 chars of base64(SHA-256(api_key)) for indexed lookup.
fn compute_api_key_prefix(api_key: &str) -> String {
    let hash = Sha256::digest(api_key.as_bytes());
    let encoded = base64::engine::general_purpose::STANDARD_NO_PAD.encode(hash);
    encoded.chars().take(8).collect()
}

/// Derives a stable prefix from a stored hash for legacy users whose
/// `api_key_prefix` is NULL. The prefix is independent of the original
/// API key (which is unrecoverable from the Argon2 hash), so legacy users
/// cannot authenticate after backfill — but the fallback that turned
/// NULL-prefix users into an O(N) Argon2id DoS amplifier is removed.
fn derive_prefix_from_hash(stored_hash: &str) -> String {
    let hash = Sha256::digest(stored_hash.as_bytes());
    let encoded = base64::engine::general_purpose::STANDARD_NO_PAD.encode(hash);
    encoded.chars().take(8).collect()
}

fn saturating_i32(v: i64) -> i32 {
    v.min(i32::MAX as i64) as i32
}

type SnippetRow = (String, String, String, String, i64, i64, String, i32, i32);

/// Count the rows of the filtered range.
///
/// `>=` (not `>`) is required for delta correctness: the watermark returned
/// to the client is the maximum cursor value of the library, so a strict `>`
/// would skip the boundary row itself. Re-delivering boundary rows is safe —
/// merges are idempotent by snippet identity.
async fn count_snippets(
    conn: &mut sqlx::SqliteConnection,
    user_id: &str,
    library_id: &str,
    cursor: DeltaCursor,
    include_deleted: bool,
) -> DbResult<i32> {
    let (predicate, cursor_value) = cursor.predicate();
    // The only dynamic part of the statement is `predicate`, which is a
    // fixed literal chosen by the cursor form — no caller input reaches the
    // SQL text, and every value is bound as a parameter.
    let sql = AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM snippets \
         WHERE user_id = ? \
           AND library_id = ? \
           {predicate} \
           AND (? OR deleted = 0)"
    ));
    let total: (i64,) = sqlx::query_as(sql)
        .bind(user_id)
        .bind(library_id)
        .bind(cursor_value)
        .bind(include_deleted)
        .fetch_one(&mut *conn)
        .await?;

    Ok(saturating_i32(total.0))
}

/// Read one page of the filtered range, newest first.
async fn fetch_snippets(
    conn: &mut sqlx::SqliteConnection,
    user_id: &str,
    library_id: &str,
    cursor: DeltaCursor,
    limit: i32,
    offset: i32,
    include_deleted: bool,
) -> DbResult<Vec<Snippet>> {
    let (predicate, cursor_value) = cursor.predicate();
    // See `count_snippets`: the only dynamic part is a fixed literal.
    let sql = AssertSqlSafe(format!(
        "SELECT id, description, command, tags, created_at, updated_at, device_id, deleted, encrypted \
         FROM snippets \
         WHERE user_id = ? \
           AND library_id = ? \
           {predicate} \
           AND (? OR deleted = 0) \
         ORDER BY updated_at DESC, id DESC \
         LIMIT ? OFFSET ?"
    ));
    let rows: Vec<SnippetRow> = sqlx::query_as(sql)
        .bind(user_id)
        .bind(library_id)
        .bind(cursor_value)
        .bind(include_deleted)
        .bind(limit)
        .bind(offset)
        .fetch_all(&mut *conn)
        .await?;

    rows.into_iter()
        .map(
            |(
                id,
                description,
                command,
                tags_str,
                created_at,
                updated_at,
                device_id,
                deleted,
                encrypted,
            )| {
                // Surface corruption to repair flows instead of
                // silently substituting an empty tag list.
                let tags: Vec<String> = serde_json::from_str(&tags_str).map_err(|e| {
                    DbError::Internal(format!(
                        "corrupt tags JSON for snippet {id} (row must be repaired): {e}"
                    ))
                })?;
                Ok(Snippet {
                    id,
                    description,
                    command,
                    tags,
                    created_at,
                    updated_at,
                    device_id,
                    deleted: deleted != 0,
                    encrypted: encrypted != 0,
                })
            },
        )
        .collect::<DbResult<Vec<Snippet>>>()
}

const MIGRATION_BATCH_SIZE: i64 = 100;

/// Bump the library's write-sequence counter and return the new value.
///
/// The counter row is created on first use. The bump runs inside the
/// caller's transaction, so it is committed with the row write or rolled
/// back with it: a failed write never consumes a sequence value, and a
/// consumed value is never handed out twice.
async fn bump_library_sequence(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    library_id: &str,
) -> DbResult<i64> {
    sqlx::query(
        "INSERT INTO library_sequences (library_id, sequence) VALUES (?, 0) \
         ON CONFLICT(library_id) DO NOTHING",
    )
    .bind(library_id)
    .execute(&mut **tx)
    .await?;

    let (sequence,): (i64,) = sqlx::query_as(
        "UPDATE library_sequences SET sequence = sequence + 1 WHERE library_id = ? RETURNING sequence",
    )
    .bind(library_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(sequence)
}

impl Database {
    pub async fn connect(url: &str, max_connections: u32) -> DbResult<Self> {
        // The public configuration uses a SQLite file path for convenience,
        // while tests and advanced deployments may provide a full sqlite URL.
        // Sqlx's generic `Pool::connect` leaves file creation disabled, which
        // makes a fresh `snip-sync init` fail on its first start. Normalize
        // both forms and explicitly create the file when needed.
        let options = if url.starts_with("sqlite:") {
            SqliteConnectOptions::from_str(url)?
        } else {
            SqliteConnectOptions::new().filename(url)
        }
        .create_if_missing(true)
        .busy_timeout(Duration::from_secs(5));

        let filename = options.get_filename();
        if filename != Path::new(":memory:")
            && let Some(parent) = filename.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| {
                DbError::Internal(format!(
                    "Failed to create database directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        // `foreign_keys` is a **per-connection** SQLite setting (unlike
        // `journal_mode`, which SQLite persists in the database file header).
        // Running the pragma against the pool set it on whichever single
        // connection it happened to check out — at most 1 of
        // `max_connections` — leaving the schema's declared FKs unenforced on
        // the rest. `after_connect` runs for every connection the pool opens.
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query("PRAGMA foreign_keys=ON").execute(conn).await?;
                    Ok(())
                })
            })
            .connect_with(options)
            .await?;

        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await?;

        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                api_key TEXT UNIQUE NOT NULL,
                api_key_prefix TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )
            ",
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_users_api_key_prefix ON users(api_key_prefix)")
            .execute(&pool)
            .await?;

        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS libraries (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                name TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                deleted_at INTEGER,
                FOREIGN KEY (user_id) REFERENCES users(id),
                UNIQUE(user_id, name)
            )
            ",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS snippets (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                library_id TEXT NOT NULL,
                description TEXT NOT NULL,
                command TEXT NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                device_id TEXT NOT NULL,
                deleted INTEGER NOT NULL DEFAULT 0,
                encrypted INTEGER NOT NULL DEFAULT 0,
                visible_at INTEGER NOT NULL DEFAULT 0,
                sequence INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (user_id) REFERENCES users(id),
                FOREIGN KEY (library_id) REFERENCES libraries(id)
            )
            ",
        )
        .execute(&pool)
        .await?;

        // The per-library write-sequence counter. One row per library holds
        // the last sequence the server handed out; the next write takes the
        // next value. The counter lives in the database, so a server restart
        // does not reset it.
        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS library_sequences (
                library_id TEXT PRIMARY KEY,
                sequence INTEGER NOT NULL DEFAULT 0
            )
            ",
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_snippets_user ON snippets(user_id)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_snippets_library ON snippets(library_id)")
            .execute(&pool)
            .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_snippets_updated ON snippets(updated_at)")
            .execute(&pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_snippets_user_library_updated \
             ON snippets(user_id, library_id, updated_at, deleted)",
        )
        .execute(&pool)
        .await?;
        // The delta cursor lives in `visible_at`, not in the client-chosen
        // `updated_at`; this index serves the `visible_at >= since` range.
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_snippets_user_library_visible \
             ON snippets(user_id, library_id, visible_at, deleted)",
        )
        .execute(&pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_libraries_user ON libraries(user_id)")
            .execute(&pool)
            .await?;

        // The write-sequence cursor owns its column, its counter, and its
        // indexes together: the unique index cannot be created before the
        // column exists, and the column cannot be backfilled before the
        // counter table exists. The migration runs inside `connect` so the
        // server never serves a database whose sequence cursor is half
        // installed.
        let db = Self { pool };
        db.migrate_snippet_sequence().await?;

        Ok(db)
    }

    pub async fn ping(&self) -> DbResult<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn create_user(&self, api_key: &str) -> DbResult<String> {
        let user_id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        let api_key_hash = hash_api_key(api_key)?;
        let api_key_prefix = compute_api_key_prefix(api_key);

        let mut tx = self.pool.begin().await?;

        sqlx::query("INSERT INTO users (id, api_key, api_key_prefix, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
            .bind(&user_id)
            .bind(&api_key_hash)
            .bind(&api_key_prefix)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;

        let default_lib_id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO libraries (id, user_id, name, created_at) VALUES (?, ?, ?, ?)")
            .bind(&default_lib_id)
            .bind(&user_id)
            .bind("default")
            .bind(now)
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;

        tracing::info!(
            "Created user {} with default library {}",
            user_id,
            default_lib_id
        );

        Ok(user_id)
    }

    pub async fn get_user_by_api_key(&self, api_key: &str) -> DbResult<Option<String>> {
        let prefix = compute_api_key_prefix(api_key);

        // Prefix is always populated for new users (see create_user) and is
        // backfilled by migrate_plaintext_api_keys. The legacy NULL-prefix
        // fallback was removed because N NULL-prefix users turned every auth
        // attempt into O(N) Argon2id verification — a DoS amplifier.
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT id, api_key FROM users WHERE api_key_prefix = ?")
                .bind(&prefix)
                .fetch_all(&self.pool)
                .await?;

        for (user_id, stored_hash) in rows {
            if verify_api_key(api_key, &stored_hash) {
                return Ok(Some(user_id));
            }
        }

        Ok(None)
    }

    pub async fn create_library(&self, user_id: &str, name: &str) -> DbResult<String> {
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(DbError::Validation(
                "Invalid library name. Use only letters, numbers, dash, and underscore."
                    .to_string(),
            ));
        }
        if name.is_empty() || name.len() > 64 {
            return Err(DbError::Validation(
                "Library name must be 1-64 characters".to_string(),
            ));
        }

        let lib_id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();

        let result = sqlx::query(
            "INSERT OR IGNORE INTO libraries (id, user_id, name, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(&lib_id)
        .bind(user_id)
        .bind(name)
        .bind(now)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(DbError::Conflict(format!(
                "Library '{}' already exists",
                name
            )));
        }

        Ok(lib_id)
    }

    pub async fn list_libraries(
        &self,
        user_id: &str,
        limit: i32,
        offset: i32,
    ) -> DbResult<(Vec<Library>, i32)> {
        let total: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM libraries WHERE user_id = ? AND deleted_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
            "SELECT l.id, l.name, l.created_at, COUNT(s.id) as snippet_count \
             FROM libraries l \
             LEFT JOIN snippets s ON s.library_id = l.id AND s.deleted = 0 \
             WHERE l.user_id = ? AND l.deleted_at IS NULL \
             GROUP BY l.id \
             ORDER BY l.name LIMIT ? OFFSET ?",
        )
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let libraries = rows
            .into_iter()
            .map(|(id, name, created_at, snippet_count)| Library {
                id,
                name,
                created_at,
                snippet_count,
            })
            .collect();

        Ok((libraries, saturating_i32(total.0)))
    }

    pub async fn delete_library(&self, user_id: &str, library_id: &str) -> DbResult<()> {
        let now = Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;

        let result = sqlx::query(
            "UPDATE libraries SET deleted_at = ? WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
        )
        .bind(now)
        .bind(library_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

        if result.rows_affected() == 0 {
            tx.rollback().await?;
            return Err(DbError::NotFound(
                "Library not found or already deleted".to_string(),
            ));
        }

        // Tombstone the rows. Every tombstoned row takes a fresh write
        // sequence, so a client that syncs on the sequence cursor sees the
        // deletion instead of keeping the row it already had.
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM snippets WHERE library_id = ? AND user_id = ? AND deleted = 0",
        )
        .bind(library_id)
        .bind(user_id)
        .fetch_all(&mut *tx)
        .await?;

        for (id,) in rows {
            let sequence = bump_library_sequence(&mut tx, library_id).await?;
            sqlx::query(
                "UPDATE snippets
                 SET deleted = 1, updated_at = MAX(updated_at, ?), visible_at = MAX(visible_at, ?), sequence = ?
                 WHERE id = ?",
            )
            .bind(now)
            .bind(now)
            .bind(sequence)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_default_library(&self, user_id: &str) -> DbResult<String> {
        let (lib_id,): (String,) = sqlx::query_as(
            "SELECT id FROM libraries WHERE user_id = ? AND name = 'default' AND deleted_at IS NULL",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| DbError::NotFound("Default library not found".to_string()))?;

        Ok(lib_id)
    }

    pub async fn verify_library_ownership(
        &self,
        user_id: &str,
        library_id: &str,
    ) -> DbResult<bool> {
        let (count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM libraries WHERE id = ? AND user_id = ? AND deleted_at IS NULL",
        )
        .bind(library_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count > 0)
    }

    pub async fn get_snippets(
        &self,
        user_id: &str,
        library_id: &str,
        since: i64,
        limit: i32,
        offset: i32,
        include_deleted: bool,
    ) -> DbResult<(Vec<Snippet>, i32)> {
        // The read-only `GetSnippets` RPC carries a timestamp cursor only, so
        // it always filters on the timestamp cursor.
        let page = self
            .get_snippets_page(
                user_id,
                library_id,
                DeltaCursor::Timestamp(since),
                limit,
                offset,
                include_deleted,
            )
            .await?;
        Ok((page.snippets, page.total))
    }

    /// Fetch one page of snippets together with the paging facts derived
    /// from it.
    ///
    /// The page is read with `LIMIT limit + 1`, so `has_more` comes from the
    /// rows themselves instead of a second `COUNT(*)` scan over the same
    /// range. The exact `total` is taken from the page whenever the page is
    /// the last one (the range then holds exactly `offset + rows` rows); when
    /// the range continues past the page — or the requested offset is past
    /// the end of the range — the range is still counted so `total` keeps
    /// its exact meaning.
    pub async fn get_snippets_page(
        &self,
        user_id: &str,
        library_id: &str,
        cursor: DeltaCursor,
        limit: i32,
        offset: i32,
        include_deleted: bool,
    ) -> DbResult<SnippetPage> {
        let mut tx = self.pool.begin().await?;

        let page_limit = limit.max(0);
        let mut snippets = fetch_snippets(
            &mut tx,
            user_id,
            library_id,
            cursor,
            page_limit.saturating_add(1),
            offset,
            include_deleted,
        )
        .await?;

        let has_more = snippets.len() > page_limit as usize;
        snippets.truncate(page_limit as usize);

        let total = if !has_more && (!snippets.is_empty() || offset == 0) {
            offset.saturating_add(snippets.len() as i32)
        } else {
            count_snippets(&mut tx, user_id, library_id, cursor, include_deleted).await?
        };

        tx.commit().await?;

        Ok(SnippetPage {
            snippets,
            has_more,
            total,
        })
    }

    #[cfg(test)]
    pub async fn upsert_snippet(
        &self,
        snippet: &Snippet,
        user_id: &str,
        library_id: &str,
    ) -> DbResult<()> {
        // Delegate to the production write path so a test fixture gets the
        // same cursor bookkeeping as a real push.
        let mut tx = self.pool.begin().await?;
        self.upsert_snippet_in_tx(&mut tx, snippet, user_id, library_id)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn upsert_snippet_in_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        snippet: &Snippet,
        user_id: &str,
        library_id: &str,
    ) -> DbResult<UpsertOutcome> {
        let tags_json = serde_json::to_string(&snippet.tags).unwrap_or_else(|_| "[]".to_string());
        let deleted = snippet.deleted as i32;
        let encrypted = snippet.encrypted as i32;

        // The server assigns the write sequence. The counter is bumped
        // **first** so this transaction takes SQLite's write lock before it
        // reads the watermark: two concurrent writes to one library serialize
        // here and receive distinct sequence values instead of both reading
        // the same counter. The bump shares the transaction with the row
        // write, so a failed write consumes no sequence value.
        let sequence = bump_library_sequence(tx, library_id).await?;

        // The timestamp delta cursor is the library's `visible_at` watermark,
        // not the client-chosen `updated_at`: a row stored *below* the
        // watermark would never be delivered to any client, because the
        // timestamp delta pull filters on `visible_at >= since`. The stored
        // `updated_at` keeps the client's logical modification time verbatim
        // — that is what the server's last-write-wins rule and the client's
        // merge order compare — while `visible_at` carries the cursor position
        // the row is delivered at.
        let (watermark,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(visible_at), 0) FROM snippets WHERE user_id = ? AND library_id = ?",
        )
        .bind(user_id)
        .bind(library_id)
        .fetch_one(&mut **tx)
        .await?;
        let visible_at = snippet.updated_at.max(watermark);

        // Upsert with last-write-wins conflict resolution.
        // Tie-breaking uses device_id string comparison, which is correct for
        // lowercase hex UUIDs (lexicographic == numeric for lowercase hex).
        // When updated_at and device_id are equal, fall back to content
        // comparison (command, description) to provide a deterministic
        // winner that mirrors the client's fingerprint tie-break.
        //
        // The conflict comparison uses the client's raw `updated_at`, so the
        // rule is unchanged by the cursor bookkeeping; only `visible_at` and
        // `sequence` move, and neither moves backwards.
        let result = sqlx::query(
            "INSERT INTO snippets (id, user_id, library_id, description, command, tags, created_at, updated_at, device_id, deleted, encrypted, visible_at, sequence)
              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
              ON CONFLICT(id) DO UPDATE SET
                 description = excluded.description,
                 command = excluded.command,
                 tags = excluded.tags,
                 updated_at = excluded.updated_at,
                 device_id = excluded.device_id,
                 deleted = excluded.deleted,
                 encrypted = excluded.encrypted,
                 visible_at = MAX(snippets.visible_at, excluded.visible_at),
                 sequence = excluded.sequence
              WHERE excluded.user_id = snippets.user_id AND excluded.library_id = snippets.library_id \
            AND (excluded.updated_at > snippets.updated_at \
                 OR (excluded.updated_at = snippets.updated_at AND excluded.device_id > snippets.device_id) \
                 OR (excluded.updated_at = snippets.updated_at AND excluded.device_id = snippets.device_id AND excluded.command > snippets.command) \
                 OR (excluded.updated_at = snippets.updated_at AND excluded.device_id = snippets.device_id AND excluded.command = snippets.command AND excluded.description > snippets.description))",
        )
        .bind(&snippet.id)
        .bind(user_id)
        .bind(library_id)
        .bind(&snippet.description)
        .bind(&snippet.command)
        .bind(&tags_json)
        .bind(snippet.created_at)
        .bind(snippet.updated_at)
        .bind(&snippet.device_id)
        .bind(deleted)
        .bind(encrypted)
        .bind(visible_at)
        .bind(sequence)
        .execute(&mut **tx)
        .await?;

        if result.rows_affected() > 0 {
            return Ok(UpsertOutcome::Written);
        }

        // Zero rows written. Either the existing record under this same
        // user/library is newer (normal last-write-wins dedupe), or the id is
        // already held by a different scope and the write was dropped. Only a
        // scope probe can tell them apart.
        let foreign: Option<(String, String)> = sqlx::query_as(
            "SELECT user_id, library_id FROM snippets \
             WHERE id = ? AND (user_id != ? OR library_id != ?) LIMIT 1",
        )
        .bind(&snippet.id)
        .bind(user_id)
        .bind(library_id)
        .fetch_optional(&mut **tx)
        .await?;

        Ok(match foreign {
            Some(_) => UpsertOutcome::ForeignScope,
            None => UpsertOutcome::Stale,
        })
    }

    pub async fn get_latest_timestamp(&self, user_id: &str, library_id: &str) -> DbResult<i64> {
        // The cursor is the `visible_at` watermark, not `MAX(updated_at)`:
        // `updated_at` is chosen by the client and may sit below the current
        // watermark, which would make the cursor non-monotone.
        let (timestamp,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(visible_at), 0) FROM snippets WHERE user_id = ? AND library_id = ?",
        )
        .bind(user_id)
        .bind(library_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(timestamp)
    }

    /// The library's write-sequence watermark.
    ///
    /// This is the cursor the server hands back to a client that syncs on the
    /// sequence path. It is the highest sequence any row of the library
    /// holds, so it never moves backwards and never skips a row.
    pub async fn get_latest_sequence(&self, user_id: &str, library_id: &str) -> DbResult<i64> {
        let (sequence,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(sequence), 0) FROM snippets WHERE user_id = ? AND library_id = ?",
        )
        .bind(user_id)
        .bind(library_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(sequence)
    }

    /// Add the write-sequence cursor: its column, its per-library counter,
    /// and the unique index that turns a counter race into a loud failure.
    ///
    /// The backfill gives every row a sequence that preserves the
    /// pre-migration `updated_at` ordering (ranked inside the row's library,
    /// ties broken by the row id), so a migrated database selects the same
    /// rows as an unmigrated one for the first sync and every client's stored
    /// cursor stays valid. The counter is then seeded from the highest
    /// sequence of each library, so the next write can never collide with a
    /// backfilled row.
    ///
    /// Idempotent: it is safe to run on every start.
    pub async fn migrate_snippet_sequence(&self) -> DbResult<bool> {
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('snippets')")
                .fetch_all(&self.pool)
                .await?;

        let migrated = if columns.iter().any(|name| name == "sequence") {
            false
        } else {
            sqlx::query("ALTER TABLE snippets ADD COLUMN sequence INTEGER NOT NULL DEFAULT 0")
                .execute(&self.pool)
                .await?;
            // The rank of the row inside its library, ordered by the
            // pre-migration `updated_at` and then by the row id. The
            // backfilled ordering therefore preserves the pre-migration
            // ordering, and every row of a library gets a distinct sequence.
            sqlx::query(
                "WITH ranked AS (
                     SELECT id,
                            ROW_NUMBER() OVER (
                                PARTITION BY user_id, library_id
                                ORDER BY updated_at ASC, id ASC
                            ) AS rn
                     FROM snippets
                 )
                 UPDATE snippets
                 SET sequence = (SELECT rn FROM ranked WHERE ranked.id = snippets.id)",
            )
            .execute(&self.pool)
            .await?;
            true
        };

        // The counter must never hand out a value a row already holds, so it
        // is seeded from the highest sequence of each library. The seed runs
        // through a subquery with a `WHERE` clause: SQLite needs one to tell
        // the upsert's `ON CONFLICT` apart from a join's `ON`.
        sqlx::query(
            "INSERT INTO library_sequences (library_id, sequence)
             SELECT library_id, highest
             FROM (
                 SELECT library_id, MAX(sequence) AS highest
                 FROM snippets
                 GROUP BY library_id
             ) WHERE true
             ON CONFLICT(library_id) DO UPDATE
                SET sequence = MAX(library_sequences.sequence, excluded.sequence)",
        )
        .execute(&self.pool)
        .await?;

        // The unique index is the enforcement mechanism for the concurrent
        // counter race: two rows sharing a (library_id, sequence) are
        // rejected loudly instead of silently making one of them invisible.
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_snippets_library_sequence \
             ON snippets(library_id, sequence)",
        )
        .execute(&self.pool)
        .await?;

        // The range index for the sequence form of the delta filter.
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_snippets_user_library_sequence \
             ON snippets(user_id, library_id, sequence, deleted)",
        )
        .execute(&self.pool)
        .await?;

        if migrated {
            tracing::info!("Migrated snippets table: added the sequence write-cursor column");
        }

        Ok(migrated)
    }

    /// Add the `visible_at` cursor column to an existing database.
    ///
    /// The column carries the delta cursor position of a row; `updated_at`
    /// keeps the client's logical modification time. Existing rows are
    /// backfilled with their current `updated_at`, so a migrated database
    /// selects exactly the same rows as before the migration and every
    /// client's stored cursor stays valid.
    pub async fn migrate_snippet_visible_at(&self) -> DbResult<bool> {
        let columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info('snippets')")
                .fetch_all(&self.pool)
                .await?;
        if columns.iter().any(|name| name == "visible_at") {
            return Ok(false);
        }

        sqlx::query("ALTER TABLE snippets ADD COLUMN visible_at INTEGER NOT NULL DEFAULT 0")
            .execute(&self.pool)
            .await?;
        sqlx::query("UPDATE snippets SET visible_at = updated_at")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_snippets_user_library_visible \
             ON snippets(user_id, library_id, visible_at, deleted)",
        )
        .execute(&self.pool)
        .await?;

        tracing::info!("Migrated snippets table: added the visible_at delta-cursor column");
        Ok(true)
    }

    pub async fn migrate_plaintext_api_keys(&self) -> DbResult<usize> {
        let mut migrated = 0;
        let mut last_id = String::new();

        loop {
            let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
                "SELECT id, api_key, api_key_prefix FROM users \
                 WHERE id > ? ORDER BY id LIMIT ?",
            )
            .bind(&last_id)
            .bind(MIGRATION_BATCH_SIZE)
            .fetch_all(&self.pool)
            .await?;

            if rows.is_empty() {
                break;
            }

            // Each batch commits atomically: a crash mid-batch leaves every
            // row in the batch at its pre-migration state, so auth never sees
            // a half-migrated user (hash updated but prefix stale).
            let mut tx = self.pool.begin().await?;
            for (user_id, stored, prefix) in rows {
                last_id = user_id.clone();
                let mut needs_update = false;
                let mut new_hash = stored.clone();
                let mut new_prefix = prefix.clone();

                // Migrate plaintext to hashed
                if !stored.starts_with("$argon2") {
                    new_hash = hash_api_key(&stored)?;
                    needs_update = true;
                }

                // Backfill prefix if missing
                if prefix.is_none() {
                    if stored.starts_with("$argon2") {
                        // Legacy hashed key: derive a stable prefix from the stored hash.
                        // The original API key is unrecoverable, so this user will no
                        // longer be able to authenticate — but the NULL-prefix fallback
                        // that amplified auth into an O(N) Argon2id DoS is removed.
                        new_prefix = Some(derive_prefix_from_hash(&stored));
                    } else {
                        new_prefix = Some(compute_api_key_prefix(&stored));
                    }
                    needs_update = true;
                }

                if needs_update {
                    sqlx::query("UPDATE users SET api_key = ?, api_key_prefix = ? WHERE id = ?")
                        .bind(&new_hash)
                        .bind(&new_prefix)
                        .bind(&user_id)
                        .execute(&mut *tx)
                        .await?;
                    migrated += 1;
                }
            }
            tx.commit().await?;
        }

        if migrated > 0 {
            tracing::info!("Migrated {} API keys (hash or prefix update)", migrated);
        }

        Ok(migrated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_db() -> Database {
        Database::connect("sqlite::memory:", 5).await.unwrap()
    }

    #[tokio::test]
    async fn test_file_database_is_created_from_path() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested").join("snippets.db");

        let _db = Database::connect(path.to_str().unwrap(), 1).await.unwrap();

        assert!(path.exists());
    }

    #[tokio::test]
    async fn test_create_and_verify_user() {
        let db = setup_db().await;
        let api_key = "test-api-key-12345";

        let user_id = db.create_user(api_key).await.unwrap();
        assert!(!user_id.is_empty());

        // Should find user with correct API key
        let found = db.get_user_by_api_key(api_key).await.unwrap();
        assert_eq!(found, Some(user_id));

        // Should not find user with wrong API key
        let not_found = db.get_user_by_api_key("wrong-key").await.unwrap();
        assert_eq!(not_found, None);
    }

    #[tokio::test]
    async fn test_api_key_is_hashed() {
        let db = setup_db().await;
        let api_key = "plaintext-key-should-be-hashed";

        let _user_id = db.create_user(api_key).await.unwrap();

        // Verify the stored key is an Argon2 hash, not plaintext
        let rows: Vec<(String, String, Option<String>)> =
            sqlx::query_as("SELECT id, api_key, api_key_prefix FROM users")
                .fetch_all(&db.pool)
                .await
                .unwrap();

        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].1.starts_with("$argon2"),
            "API key should be stored as Argon2 hash"
        );
        // Verify prefix is stored
        assert!(rows[0].2.is_some(), "API key prefix should be stored");
        assert_eq!(rows[0].2.as_ref().unwrap().len(), 8);
    }

    #[tokio::test]
    async fn test_create_default_library() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();

        let lib_id = db.get_default_library(&user_id).await.unwrap();
        assert!(!lib_id.is_empty());

        let (libraries, total) = db.list_libraries(&user_id, 10, 0).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(libraries[0].name, "default");
    }

    #[tokio::test]
    async fn test_create_additional_library() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();

        let lib_id = db.create_library(&user_id, "work").await.unwrap();
        assert!(!lib_id.is_empty());

        let (_libraries, total) = db.list_libraries(&user_id, 10, 0).await.unwrap();
        assert_eq!(total, 2);

        // Duplicate should fail with Conflict
        let err = db.create_library(&user_id, "work").await.unwrap_err();
        assert!(matches!(err, DbError::Conflict(_)));
    }

    #[tokio::test]
    async fn test_invalid_library_names() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();

        // Empty name
        assert!(db.create_library(&user_id, "").await.is_err());

        // Name with slash
        assert!(db.create_library(&user_id, "my/lib").await.is_err());

        // Name too long
        assert!(db.create_library(&user_id, &"a".repeat(65)).await.is_err());

        // Valid names
        assert!(db.create_library(&user_id, "valid-name").await.is_ok());
        assert!(db.create_library(&user_id, "valid_name").await.is_ok());
        assert!(db.create_library(&user_id, "ValidName123").await.is_ok());
    }

    #[tokio::test]
    async fn test_upsert_and_get_snippets() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        let snippet = Snippet {
            id: "snip-1".to_string(),
            description: "Test snippet".to_string(),
            command: "echo hello".to_string(),
            tags: vec!["test".to_string()],
            created_at: 100,
            updated_at: 100,
            device_id: "device-1".to_string(),
            deleted: false,
            encrypted: false,
        };

        db.upsert_snippet(&snippet, &user_id, &lib_id)
            .await
            .unwrap();

        let (snippets, total) = db
            .get_snippets(&user_id, &lib_id, 0, 10, 0, false)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(snippets[0].description, "Test snippet");
        assert_eq!(snippets[0].command, "echo hello");
        assert_eq!(snippets[0].tags, vec!["test"]);
    }

    /// Delta queries must include rows at exactly the watermark: the cursor
    /// has second granularity and the client stores `MAX(visible_at)` as
    /// `last_sync`. With strict `>`, a row written in the same second as the
    /// watermark was invisible to every later incremental sync.
    #[tokio::test]
    async fn test_delta_query_includes_watermark_boundary_row() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        let snippet = Snippet {
            id: "boundary-snip".to_string(),
            description: "Boundary".to_string(),
            command: "echo boundary".to_string(),
            tags: vec![],
            created_at: 500,
            updated_at: 500,
            device_id: "device-1".to_string(),
            deleted: false,
            encrypted: false,
        };
        db.upsert_snippet(&snippet, &user_id, &lib_id)
            .await
            .unwrap();

        let latest = db.get_latest_timestamp(&user_id, &lib_id).await.unwrap();
        assert_eq!(latest, 500);

        // A subsequent incremental sync sends `since = MAX(visible_at)`.
        let (snippets, total) = db
            .get_snippets(&user_id, &lib_id, latest, 10, 0, false)
            .await
            .unwrap();
        assert_eq!(total, 1, "row at exactly `since` must be returned");
        assert_eq!(snippets[0].id, "boundary-snip");
    }

    /// A row whose client-chosen `updated_at` sits *below* the library's
    /// watermark must still be delivered: the delta cursor lives in
    /// `visible_at`, which is clamped to the watermark, while `updated_at`
    /// keeps the client's logical modification time.
    #[tokio::test]
    async fn test_insert_below_watermark_is_still_delivered() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        let make = |id: &str, updated_at: i64| Snippet {
            id: id.to_string(),
            description: format!("desc-{id}"),
            command: format!("echo {id}"),
            tags: vec![],
            created_at: updated_at,
            updated_at,
            device_id: "device-1".to_string(),
            deleted: false,
            encrypted: false,
        };

        let mut tx = db.pool.begin().await.unwrap();
        assert_eq!(
            db.upsert_snippet_in_tx(&mut tx, &make("newer", 1000), &user_id, &lib_id)
                .await
                .unwrap(),
            UpsertOutcome::Written
        );
        assert_eq!(
            db.upsert_snippet_in_tx(&mut tx, &make("older", 500), &user_id, &lib_id)
                .await
                .unwrap(),
            UpsertOutcome::Written
        );
        tx.commit().await.unwrap();

        let watermark = db.get_latest_timestamp(&user_id, &lib_id).await.unwrap();
        assert_eq!(watermark, 1000);

        let (snippets, total) = db
            .get_snippets(&user_id, &lib_id, watermark, 10, 0, false)
            .await
            .unwrap();
        assert_eq!(
            total, 2,
            "the row pushed below the watermark must still be delivered by a delta pull"
        );

        // The stored logical time is the client's own value, unchanged.
        let older = snippets
            .iter()
            .find(|s| s.id == "older")
            .expect("the older row must be present");
        assert_eq!(older.updated_at, 500);
    }

    /// The cursor bookkeeping must not change the server's last-write-wins
    /// rule: the comparison uses the client's raw `updated_at`, so a newer
    /// push of a row that was stored below the watermark still wins.
    #[tokio::test]
    async fn test_update_below_the_watermark_still_uses_the_raw_timestamp() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        let make = |id: &str, command: &str, updated_at: i64| Snippet {
            id: id.to_string(),
            description: "desc".to_string(),
            command: command.to_string(),
            tags: vec![],
            created_at: 0,
            updated_at,
            device_id: "device-1".to_string(),
            deleted: false,
            encrypted: false,
        };

        // `other` sets the watermark at 1000.
        let mut tx = db.pool.begin().await.unwrap();
        db.upsert_snippet_in_tx(
            &mut tx,
            &make("other", "echo other", 1000),
            &user_id,
            &lib_id,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        // This row lands below the watermark.
        let mut tx = db.pool.begin().await.unwrap();
        assert_eq!(
            db.upsert_snippet_in_tx(&mut tx, &make("row", "echo one", 500), &user_id, &lib_id)
                .await
                .unwrap(),
            UpsertOutcome::Written
        );
        tx.commit().await.unwrap();

        // A newer push of the same row must win: the comparison reads the raw
        // `updated_at`, not the clamped cursor.
        let mut tx = db.pool.begin().await.unwrap();
        assert_eq!(
            db.upsert_snippet_in_tx(&mut tx, &make("row", "echo two", 700), &user_id, &lib_id)
                .await
                .unwrap(),
            UpsertOutcome::Written
        );
        tx.commit().await.unwrap();

        // A genuinely older push is still rejected.
        let mut tx = db.pool.begin().await.unwrap();
        assert_eq!(
            db.upsert_snippet_in_tx(&mut tx, &make("row", "echo older", 600), &user_id, &lib_id)
                .await
                .unwrap(),
            UpsertOutcome::Stale
        );
        tx.commit().await.unwrap();

        let (snippets, _) = db
            .get_snippets(&user_id, &lib_id, 0, 10, 0, false)
            .await
            .unwrap();
        let row = snippets
            .iter()
            .find(|s| s.id == "row")
            .expect("the updated snippet must be present");
        assert_eq!(row.command, "echo two");
        assert_eq!(row.updated_at, 700);
    }

    /// An existing database is upgraded in place: the backfill keeps every
    /// row at its current cursor position, so the rows a client pulls do not
    /// change across the migration.
    #[tokio::test]
    async fn test_migrate_snippet_visible_at_backfills_the_cursor() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("legacy.sqlite");
        let db = Database::connect(db_path.to_str().unwrap(), 1)
            .await
            .unwrap();
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        // Simulate a pre-migration database: the column does not exist.
        sqlx::query("DROP INDEX IF EXISTS idx_snippets_user_library_visible")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("ALTER TABLE snippets DROP COLUMN visible_at")
            .execute(db.pool())
            .await
            .unwrap();
        let snippet = Snippet {
            id: "legacy-1".to_string(),
            description: "legacy".to_string(),
            command: "echo legacy".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 100,
            device_id: "device-1".to_string(),
            deleted: false,
            encrypted: false,
        };
        sqlx::query(
            "INSERT INTO snippets (id, user_id, library_id, description, command, tags, created_at, updated_at, device_id, deleted, encrypted)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&snippet.id)
        .bind(&user_id)
        .bind(&lib_id)
        .bind(&snippet.description)
        .bind(&snippet.command)
        .bind("[]")
        .bind(snippet.created_at)
        .bind(snippet.updated_at)
        .bind(&snippet.device_id)
        .bind(0)
        .bind(0)
        .execute(db.pool())
        .await
        .unwrap();

        assert!(db.migrate_snippet_visible_at().await.unwrap());
        // A second run is a no-op.
        assert!(!db.migrate_snippet_visible_at().await.unwrap());

        assert_eq!(
            db.get_latest_timestamp(&user_id, &lib_id).await.unwrap(),
            100
        );
        let (snippets, total) = db
            .get_snippets(&user_id, &lib_id, 100, 10, 0, false)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(snippets[0].id, "legacy-1");
        assert_eq!(snippets[0].updated_at, 100);
    }

    #[tokio::test]
    async fn test_delete_library_preserves_future_tombstone_timestamp() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let library_id = db.create_library(&user_id, "future-delete").await.unwrap();
        let future_timestamp = Utc::now().timestamp() + 100;

        let snippet = Snippet {
            id: "future-snippet".to_string(),
            description: "desc".to_string(),
            command: "echo hi".to_string(),
            tags: vec![],
            created_at: 0,
            updated_at: future_timestamp,
            device_id: "device".to_string(),
            deleted: false,
            encrypted: false,
        };
        db.upsert_snippet(&snippet, &user_id, &library_id)
            .await
            .unwrap();

        db.delete_library(&user_id, &library_id).await.unwrap();

        let (snippets, total) = db
            .get_snippets(&user_id, &library_id, future_timestamp - 1, 10, 0, true)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert!(snippets[0].deleted);
        assert_eq!(snippets[0].updated_at, future_timestamp);
    }

    #[tokio::test]
    async fn test_upsert_updates_existing() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        let snippet = Snippet {
            id: "snip-1".to_string(),
            description: "Original".to_string(),
            command: "echo old".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 100,
            device_id: "device-1".to_string(),
            deleted: false,
            encrypted: false,
        };
        db.upsert_snippet(&snippet, &user_id, &lib_id)
            .await
            .unwrap();

        // Update with newer timestamp
        let updated = Snippet {
            id: "snip-1".to_string(),
            description: "Updated".to_string(),
            command: "echo new".to_string(),
            tags: vec!["updated".to_string()],
            created_at: 100,
            updated_at: 200,
            device_id: "device-1".to_string(),
            deleted: false,
            encrypted: false,
        };
        db.upsert_snippet(&updated, &user_id, &lib_id)
            .await
            .unwrap();

        let (snippets, _) = db
            .get_snippets(&user_id, &lib_id, 0, 10, 0, false)
            .await
            .unwrap();
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].description, "Updated");
        assert_eq!(snippets[0].command, "echo new");
    }

    #[tokio::test]
    async fn test_upsert_rejects_older_timestamp() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        // Insert snippet at updated_at: 200
        let snippet = Snippet {
            id: "snip-1".to_string(),
            description: "Newer".to_string(),
            command: "echo newer".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 200,
            device_id: "d".to_string(),
            deleted: false,
            encrypted: false,
        };
        db.upsert_snippet(&snippet, &user_id, &lib_id)
            .await
            .unwrap();

        // Attempt to upsert same ID with older timestamp (should be rejected)
        let older = Snippet {
            id: "snip-1".to_string(),
            description: "Older".to_string(),
            command: "echo older".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 100,
            device_id: "d".to_string(),
            deleted: false,
            encrypted: false,
        };
        db.upsert_snippet(&older, &user_id, &lib_id).await.unwrap();

        // Newer data should be preserved
        let (snippets, _) = db
            .get_snippets(&user_id, &lib_id, 0, 10, 0, false)
            .await
            .unwrap();
        assert_eq!(snippets.len(), 1);
        assert_eq!(snippets[0].description, "Newer");
        assert_eq!(snippets[0].command, "echo newer");
    }

    #[tokio::test]
    async fn test_get_latest_timestamp() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        // Empty library should return 0
        let ts = db.get_latest_timestamp(&user_id, &lib_id).await.unwrap();
        assert_eq!(ts, 0);

        let snippet = Snippet {
            id: "snip-1".to_string(),
            description: "Test".to_string(),
            command: "echo".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 500,
            device_id: "d".to_string(),
            deleted: false,
            encrypted: false,
        };
        db.upsert_snippet(&snippet, &user_id, &lib_id)
            .await
            .unwrap();

        let ts = db.get_latest_timestamp(&user_id, &lib_id).await.unwrap();
        assert_eq!(ts, 500);
    }

    #[tokio::test]
    async fn test_delete_library() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.create_library(&user_id, "temp").await.unwrap();

        // Add a snippet
        let snippet = Snippet {
            id: "snip-1".to_string(),
            description: "Test".to_string(),
            command: "echo".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 100,
            device_id: "d".to_string(),
            deleted: false,
            encrypted: false,
        };
        db.upsert_snippet(&snippet, &user_id, &lib_id)
            .await
            .unwrap();

        // Delete the library
        db.delete_library(&user_id, &lib_id).await.unwrap();

        // Should fail to delete again
        assert!(db.delete_library(&user_id, &lib_id).await.is_err());

        // Snippet should be marked deleted
        let (snippets, _) = db
            .get_snippets(&user_id, &lib_id, 0, 10, 0, false)
            .await
            .unwrap();
        assert_eq!(snippets.len(), 0);
    }

    #[tokio::test]
    async fn test_migrate_plaintext_api_keys() {
        let db = setup_db().await;
        let user_id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();

        // Insert a plaintext API key directly (simulating old schema)
        sqlx::query("INSERT INTO users (id, api_key, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(&user_id)
            .bind("plaintext-api-key")
            .bind(now)
            .bind(now)
            .execute(&db.pool)
            .await
            .unwrap();

        // Run migration
        let migrated = db.migrate_plaintext_api_keys().await.unwrap();
        assert_eq!(migrated, 1);

        // Verify the key is now hashed
        let (stored_hash,): (String,) = sqlx::query_as("SELECT api_key FROM users WHERE id = ?")
            .bind(&user_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();

        assert!(stored_hash.starts_with("$argon2"));

        // Verify prefix was backfilled
        let (prefix,): (Option<String>,) =
            sqlx::query_as("SELECT api_key_prefix FROM users WHERE id = ?")
                .bind(&user_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(
            prefix.is_some(),
            "API key prefix should be backfilled by migration"
        );

        // Verify the original key still works
        let found = db.get_user_by_api_key("plaintext-api-key").await.unwrap();
        assert_eq!(found, Some(user_id));

        // Running migration again should migrate 0
        let migrated_again = db.migrate_plaintext_api_keys().await.unwrap();
        assert_eq!(migrated_again, 0);
    }

    #[tokio::test]
    async fn test_api_key_prefix_lookup() {
        let db = setup_db().await;

        // Create multiple users
        let user1 = db.create_user("key-one-alpha").await.unwrap();
        let user2 = db.create_user("key-two-beta").await.unwrap();
        let user3 = db.create_user("key-three-gamma").await.unwrap();

        // Each user's key should resolve correctly
        assert_eq!(
            db.get_user_by_api_key("key-one-alpha").await.unwrap(),
            Some(user1)
        );
        assert_eq!(
            db.get_user_by_api_key("key-two-beta").await.unwrap(),
            Some(user2)
        );
        assert_eq!(
            db.get_user_by_api_key("key-three-gamma").await.unwrap(),
            Some(user3)
        );
        assert_eq!(db.get_user_by_api_key("nonexistent").await.unwrap(), None);

        // Verify all rows have prefixes
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT api_key_prefix FROM users WHERE api_key_prefix IS NOT NULL")
                .fetch_all(&db.pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[tokio::test]
    async fn test_library_ownership_verification() {
        let db = setup_db().await;
        let user1_id = db.create_user("user1-api-key").await.unwrap();
        let user2_id = db.create_user("user2-api-key").await.unwrap();
        let lib_id = db.create_library(&user1_id, "private").await.unwrap();

        assert!(
            db.verify_library_ownership(&user1_id, &lib_id)
                .await
                .unwrap()
        );
        assert!(
            !db.verify_library_ownership(&user2_id, &lib_id)
                .await
                .unwrap()
        );

        let nonexistent_lib = Uuid::new_v4().to_string();
        assert!(
            !db.verify_library_ownership(&user1_id, &nonexistent_lib)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_library_ownership_prevents_cross_user_access() {
        let db = setup_db().await;
        let user1_id = db.create_user("user1-key").await.unwrap();
        let user2_id = db.create_user("user2-key").await.unwrap();
        let user1_lib = db.create_library(&user1_id, "my-library").await.unwrap();

        let snippet = Snippet {
            id: "snip-1".to_string(),
            description: "User 1 secret".to_string(),
            command: "secret command".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 100,
            device_id: "device-1".to_string(),
            deleted: false,
            encrypted: false,
        };

        db.upsert_snippet(&snippet, &user1_id, &user1_lib)
            .await
            .unwrap();

        let (snippets, _) = db
            .get_snippets(&user1_id, &user1_lib, 0, 10, 0, false)
            .await
            .unwrap();
        assert_eq!(snippets.len(), 1);

        let (snippets, _) = db
            .get_snippets(&user2_id, &user1_lib, 0, 10, 0, false)
            .await
            .unwrap();
        assert_eq!(snippets.len(), 0);
    }

    #[tokio::test]
    async fn test_deleted_library_not_owned() {
        let db = setup_db().await;
        let user_id = db.create_user("test-key").await.unwrap();
        let lib_id = db.create_library(&user_id, "temp-lib").await.unwrap();

        assert!(
            db.verify_library_ownership(&user_id, &lib_id)
                .await
                .unwrap()
        );

        db.delete_library(&user_id, &lib_id).await.unwrap();

        assert!(
            !db.verify_library_ownership(&user_id, &lib_id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_get_snippets_include_deleted() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        let snippet = Snippet {
            id: "snip-1".to_string(),
            description: "Test".to_string(),
            command: "echo".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 100,
            device_id: "d".to_string(),
            deleted: false,
            encrypted: false,
        };
        db.upsert_snippet(&snippet, &user_id, &lib_id)
            .await
            .unwrap();

        // Soft-delete the snippet
        let deleted = Snippet {
            id: "snip-1".to_string(),
            description: "Test".to_string(),
            command: "echo".to_string(),
            tags: vec![],
            created_at: 100,
            updated_at: 200,
            device_id: "d".to_string(),
            deleted: true,
            encrypted: false,
        };
        db.upsert_snippet(&deleted, &user_id, &lib_id)
            .await
            .unwrap();

        // Without include_deleted: should not see deleted snippet
        let (snippets, _) = db
            .get_snippets(&user_id, &lib_id, 0, 10, 0, false)
            .await
            .unwrap();
        assert_eq!(snippets.len(), 0);

        // With include_deleted: should see deleted snippet
        let (snippets, _) = db
            .get_snippets(&user_id, &lib_id, 0, 10, 0, true)
            .await
            .unwrap();
        assert_eq!(snippets.len(), 1);
        assert!(snippets[0].deleted);
        assert_eq!(snippets[0].updated_at, 200);
    }

    // ---------------------------------------------------------------------
    // Write-sequence cursor
    // ---------------------------------------------------------------------

    fn sequence_snippet(id: &str, updated_at: i64) -> Snippet {
        Snippet {
            id: id.to_string(),
            description: "Test".to_string(),
            command: "echo".to_string(),
            tags: vec![],
            created_at: updated_at,
            updated_at,
            device_id: "d".to_string(),
            deleted: false,
            encrypted: false,
        }
    }

    async fn snippet_sequences(db: &Database, library_id: &str) -> Vec<(String, i64)> {
        sqlx::query_as("SELECT id, sequence FROM snippets WHERE library_id = ? ORDER BY sequence")
            .bind(library_id)
            .fetch_all(db.pool())
            .await
            .unwrap()
    }

    /// The sequence watermark is the highest sequence the library holds.
    #[tokio::test]
    async fn test_sequence_watermark_is_the_library_maximum() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        assert_eq!(db.get_latest_sequence(&user_id, &lib_id).await.unwrap(), 0);

        db.upsert_snippet(&sequence_snippet("a", 100), &user_id, &lib_id)
            .await
            .unwrap();
        db.upsert_snippet(&sequence_snippet("b", 200), &user_id, &lib_id)
            .await
            .unwrap();

        assert_eq!(db.get_latest_sequence(&user_id, &lib_id).await.unwrap(), 2);
        assert_eq!(snippet_sequences(&db, &lib_id).await.len(), 2);
    }

    /// Every write takes a sequence strictly greater than the library's
    /// current maximum, whatever timestamp the client pushed — including a
    /// push whose timestamp is older than the rows already stored.
    #[tokio::test]
    async fn test_write_assigns_a_sequence_above_the_library_maximum() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        db.upsert_snippet(&sequence_snippet("a", 300), &user_id, &lib_id)
            .await
            .unwrap();
        db.upsert_snippet(&sequence_snippet("b", 100), &user_id, &lib_id)
            .await
            .unwrap();
        db.upsert_snippet(&sequence_snippet("c", 200), &user_id, &lib_id)
            .await
            .unwrap();

        let rows = snippet_sequences(&db, &lib_id).await;
        let sequences: Vec<i64> = rows.iter().map(|(_, sequence)| *sequence).collect();
        assert_eq!(sequences, vec![1, 2, 3]);
        assert_eq!(db.get_latest_sequence(&user_id, &lib_id).await.unwrap(), 3);
    }

    /// Two concurrent pushes to one library take distinct sequence values.
    ///
    /// The counter is bumped inside the write transaction, so the two
    /// transactions serialize on it; the unique index on
    /// `(library_id, sequence)` is the enforcement that turns a shared
    /// value into a loud failure instead of a silent one.
    #[tokio::test]
    async fn test_concurrent_pushes_to_one_library_get_distinct_sequences() {
        // A file-backed database: the in-memory database shares one cache
        // across the pool, which serializes writes with a table lock
        // instead of the write lock a real deployment uses.
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("concurrent.sqlite");
        let db = Database::connect(db_path.to_str().unwrap(), 5)
            .await
            .unwrap();
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        let first_snippet = sequence_snippet("a", 100);
        let second_snippet = sequence_snippet("b", 200);
        let first = db.upsert_snippet(&first_snippet, &user_id, &lib_id);
        let second = db.upsert_snippet(&second_snippet, &user_id, &lib_id);
        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        second.unwrap();

        let rows = snippet_sequences(&db, &lib_id).await;
        assert_eq!(rows.len(), 2);
        assert_ne!(
            rows[0].1, rows[1].1,
            "concurrent writes to one library must not share a sequence value"
        );
    }

    /// A rolled-back write consumes no sequence value: the counter bump
    /// shares the transaction with the row write.
    #[tokio::test]
    async fn test_rolled_back_write_does_not_consume_a_sequence_value() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        let mut tx = db.pool().begin().await.unwrap();
        db.upsert_snippet_in_tx(&mut tx, &sequence_snippet("a", 100), &user_id, &lib_id)
            .await
            .unwrap();
        tx.rollback().await.unwrap();

        assert_eq!(db.get_latest_sequence(&user_id, &lib_id).await.unwrap(), 0);
        assert!(snippet_sequences(&db, &lib_id).await.is_empty());

        // The next write takes the first sequence value.
        db.upsert_snippet(&sequence_snippet("b", 100), &user_id, &lib_id)
            .await
            .unwrap();
        assert_eq!(db.get_latest_sequence(&user_id, &lib_id).await.unwrap(), 1);
    }

    /// The unique index is the enforcement of the counter race: two rows
    /// that share a `(library_id, sequence)` are rejected loudly.
    #[tokio::test]
    async fn test_unique_index_rejects_a_shared_sequence_value() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        let insert = |id: &str, sequence: i64| {
            sqlx::query(
                "INSERT INTO snippets (id, user_id, library_id, description, command, tags, created_at, updated_at, device_id, deleted, encrypted, visible_at, sequence)
                 VALUES (?, ?, ?, 'd', 'echo', '[]', 1, 1, 'dev', 0, 0, 1, ?)",
            )
            .bind(id)
            .bind(&user_id)
            .bind(&lib_id)
            .bind(sequence)
        };

        insert("a", 1).execute(db.pool()).await.unwrap();
        let error = insert("b", 1).execute(db.pool()).await.unwrap_err();
        assert!(
            matches!(error, sqlx::Error::Database(_)),
            "a shared sequence value must be rejected by the unique index, got {error:?}"
        );
    }

    /// The migration backfills the sequence so that the backfilled ordering
    /// preserves the pre-migration `updated_at` ordering, and seeds the
    /// counter from the backfill so the next write cannot collide.
    #[tokio::test]
    async fn test_migrate_snippet_sequence_backfills_the_updated_at_ordering() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("legacy.sqlite");
        let db = Database::connect(db_path.to_str().unwrap(), 1)
            .await
            .unwrap();
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        // Simulate a pre-migration database: no sequence column, no indexes.
        sqlx::query("DROP INDEX IF EXISTS idx_snippets_library_sequence")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("DROP INDEX IF EXISTS idx_snippets_user_library_sequence")
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("ALTER TABLE snippets DROP COLUMN sequence")
            .execute(db.pool())
            .await
            .unwrap();

        // Three legacy rows, inserted out of `updated_at` order.
        for (id, updated_at) in [("c", 300_i64), ("a", 100), ("b", 200)] {
            sqlx::query(
                "INSERT INTO snippets (id, user_id, library_id, description, command, tags, created_at, updated_at, device_id, deleted, encrypted)
                 VALUES (?, ?, ?, 'd', 'echo', '[]', ?, ?, 'dev', 0, 0)",
            )
            .bind(id)
            .bind(&user_id)
            .bind(&lib_id)
            .bind(updated_at)
            .bind(updated_at)
            .execute(db.pool())
            .await
            .unwrap();
        }

        assert!(db.migrate_snippet_sequence().await.unwrap());
        // A second run is a no-op.
        assert!(!db.migrate_snippet_sequence().await.unwrap());

        let rows: Vec<(String, i64, i64)> =
            sqlx::query_as("SELECT id, updated_at, sequence FROM snippets ORDER BY sequence")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(rows.len(), 3);
        // The backfilled ordering preserves the pre-migration `updated_at`
        // ordering.
        assert_eq!(rows[0].0, "a");
        assert_eq!(rows[1].0, "b");
        assert_eq!(rows[2].0, "c");
        assert!(rows[0].2 < rows[1].2 && rows[1].2 < rows[2].2);

        // The counter is seeded from the backfill, so the next write takes a
        // fresh value instead of colliding with a backfilled row.
        assert_eq!(db.get_latest_sequence(&user_id, &lib_id).await.unwrap(), 3);
        db.upsert_snippet(&sequence_snippet("d", 400), &user_id, &lib_id)
            .await
            .unwrap();
        assert_eq!(db.get_latest_sequence(&user_id, &lib_id).await.unwrap(), 4);
    }

    /// A client that holds a sequence cursor receives exactly the rows
    /// written after that sequence.
    #[tokio::test]
    async fn test_sequence_cursor_selects_the_rows_written_after_it() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        db.upsert_snippet(&sequence_snippet("a", 100), &user_id, &lib_id)
            .await
            .unwrap();
        db.upsert_snippet(&sequence_snippet("b", 200), &user_id, &lib_id)
            .await
            .unwrap();
        db.upsert_snippet(&sequence_snippet("c", 300), &user_id, &lib_id)
            .await
            .unwrap();

        let page = db
            .get_snippets_page(&user_id, &lib_id, DeltaCursor::Sequence(2), 100, 0, true)
            .await
            .unwrap();
        assert_eq!(page.total, 2);
        let mut ids: Vec<&str> = page
            .snippets
            .iter()
            .map(|snippet| snippet.id.as_str())
            .collect();
        ids.sort_unstable();
        assert_eq!(ids, vec!["b", "c"]);

        // The cursor is inclusive: the boundary row is re-delivered, which
        // is safe because merges are idempotent by snippet identity.
        let page = db
            .get_snippets_page(&user_id, &lib_id, DeltaCursor::Sequence(3), 100, 0, true)
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.snippets[0].id, "c");
    }

    /// An old client that sends no sequence cursor receives the rows the
    /// timestamp filter would have selected.
    #[tokio::test]
    async fn test_timestamp_cursor_still_selects_the_timestamp_rows() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        db.upsert_snippet(&sequence_snippet("a", 100), &user_id, &lib_id)
            .await
            .unwrap();
        db.upsert_snippet(&sequence_snippet("b", 200), &user_id, &lib_id)
            .await
            .unwrap();

        let timestamp_page = db
            .get_snippets_page(&user_id, &lib_id, DeltaCursor::Timestamp(0), 100, 0, true)
            .await
            .unwrap();
        let sequence_page = db
            .get_snippets_page(&user_id, &lib_id, DeltaCursor::Sequence(0), 100, 0, true)
            .await
            .unwrap();

        assert_eq!(timestamp_page.total, 2);
        assert_eq!(timestamp_page.total, sequence_page.total);
        let mut timestamp_ids: Vec<&str> = timestamp_page
            .snippets
            .iter()
            .map(|snippet| snippet.id.as_str())
            .collect();
        let mut sequence_ids: Vec<&str> = sequence_page
            .snippets
            .iter()
            .map(|snippet| snippet.id.as_str())
            .collect();
        timestamp_ids.sort_unstable();
        sequence_ids.sort_unstable();
        assert_eq!(timestamp_ids, sequence_ids);
    }

    /// The delta filter stays scoped per user and per library: a sequence
    /// cursor from one library must not select another library's rows, and
    /// another user's rows are never selected.
    #[tokio::test]
    async fn test_sequence_cursor_is_scoped_to_its_library_and_user() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_a = db.get_default_library(&user_id).await.unwrap();
        let lib_b = db.create_library(&user_id, "other").await.unwrap();

        let other_user = db.create_user("other-key").await.unwrap();
        let other_lib = db.get_default_library(&other_user).await.unwrap();

        db.upsert_snippet(&sequence_snippet("a", 100), &user_id, &lib_a)
            .await
            .unwrap();
        // Both libraries hold sequence value 1: the sequence space is
        // per library.
        db.upsert_snippet(&sequence_snippet("b", 100), &user_id, &lib_b)
            .await
            .unwrap();
        db.upsert_snippet(&sequence_snippet("c", 100), &other_user, &other_lib)
            .await
            .unwrap();

        let page = db
            .get_snippets_page(&user_id, &lib_a, DeltaCursor::Sequence(1), 100, 0, true)
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.snippets[0].id, "a");

        // Another user's library is never selected.
        let page = db
            .get_snippets_page(
                &other_user,
                &other_lib,
                DeltaCursor::Sequence(1),
                100,
                0,
                true,
            )
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.snippets[0].id, "c");
    }

    /// A tombstone takes a fresh write sequence, so a client on the sequence
    /// cursor sees the deletion.
    #[tokio::test]
    async fn test_delete_library_gives_the_tombstone_a_fresh_sequence() {
        let db = setup_db().await;
        let user_id = db.create_user("key").await.unwrap();
        let lib_id = db.get_default_library(&user_id).await.unwrap();

        db.upsert_snippet(&sequence_snippet("a", 100), &user_id, &lib_id)
            .await
            .unwrap();
        assert_eq!(db.get_latest_sequence(&user_id, &lib_id).await.unwrap(), 1);

        db.delete_library(&user_id, &lib_id).await.unwrap();

        assert_eq!(db.get_latest_sequence(&user_id, &lib_id).await.unwrap(), 2);
        let page = db
            .get_snippets_page(&user_id, &lib_id, DeltaCursor::Sequence(2), 100, 0, true)
            .await
            .unwrap();
        assert_eq!(page.total, 1);
        assert!(page.snippets[0].deleted);
    }
}
