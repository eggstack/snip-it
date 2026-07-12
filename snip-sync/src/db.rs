use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use base64::Engine;
use chrono::Utc;
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;
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

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

fn hash_api_key(api_key: &str) -> DbResult<String> {
    let mut salt_bytes = [0u8; 16];
    getrandom::fill(&mut salt_bytes)
        .map_err(|e| DbError::Internal(format!("Failed to generate salt: {}", e)))?;
    let salt_b64 = base64::engine::general_purpose::STANDARD_NO_PAD.encode(salt_bytes);
    let salt = SaltString::from_b64(&salt_b64)
        .map_err(|e| DbError::Internal(format!("Failed to create salt: {}", e)))?;
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        None,
    )
    .map_err(|e| DbError::Internal(format!("Invalid Argon2 params: {}", e)))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let hash = argon2
        .hash_password(api_key.as_bytes(), &salt)
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

fn saturating_i32(v: i64) -> i32 {
    v.min(i32::MAX as i64) as i32
}

type SnippetRow = (String, String, String, String, i64, i64, String, i32, i32);

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
        .create_if_missing(true);

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

        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await?;

        sqlx::query("PRAGMA journal_mode=WAL")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA foreign_keys=ON").execute(&pool).await?;

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
                FOREIGN KEY (user_id) REFERENCES users(id),
                FOREIGN KEY (library_id) REFERENCES libraries(id)
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
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_libraries_user ON libraries(user_id)")
            .execute(&pool)
            .await?;

        sqlx::query(
            "
            CREATE TABLE IF NOT EXISTS rate_limits (
                peer_ip TEXT PRIMARY KEY,
                window_start INTEGER NOT NULL,
                request_count INTEGER NOT NULL
            )
            ",
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
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

        // Use prefix to narrow candidate set; prefix may be NULL for legacy rows
        // that were already hashed before the prefix optimization was added.
        // The IS NULL fallback ensures these users can still authenticate.
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, api_key FROM users WHERE api_key_prefix = ? OR api_key_prefix IS NULL",
        )
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

        sqlx::query(
            "UPDATE snippets
             SET deleted = 1, updated_at = MAX(updated_at, ?)
             WHERE library_id = ? AND user_id = ? AND deleted = 0",
        )
        .bind(now)
        .bind(library_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

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
        let total: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM snippets \
             WHERE user_id = ? \
               AND library_id = ? \
               AND updated_at > ? \
               AND (? OR deleted = 0)",
        )
        .bind(user_id)
        .bind(library_id)
        .bind(since)
        .bind(include_deleted)
        .fetch_one(&self.pool)
        .await?;

        let rows: Vec<SnippetRow> = sqlx::query_as(
            "SELECT id, description, command, tags, created_at, updated_at, device_id, deleted, encrypted \
             FROM snippets \
             WHERE user_id = ? \
               AND library_id = ? \
               AND updated_at > ? \
               AND (? OR deleted = 0) \
             ORDER BY updated_at DESC, id DESC \
             LIMIT ? OFFSET ?",
        )
        .bind(user_id)
        .bind(library_id)
        .bind(since)
        .bind(include_deleted)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let snippets = rows
            .into_iter()
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
                    let tags: Vec<String> = serde_json::from_str(&tags_str).inspect_err(|e| {
                        tracing::warn!(snippet_id = %id, error = %e, "Failed to parse tags JSON, using empty list");
                    }).unwrap_or_default();
                    Snippet {
                        id,
                        description,
                        command,
                        tags,
                        created_at,
                        updated_at,
                        device_id,
                        deleted: deleted != 0,
                        encrypted: encrypted != 0,
                    }
                },
            )
            .collect();

        Ok((snippets, saturating_i32(total.0)))
    }

    #[cfg(test)]
    pub async fn upsert_snippet(
        &self,
        snippet: &Snippet,
        user_id: &str,
        library_id: &str,
    ) -> DbResult<()> {
        let tags_json = serde_json::to_string(&snippet.tags).unwrap_or_else(|_| "[]".to_string());
        let deleted = snippet.deleted as i32;
        let encrypted = snippet.encrypted as i32;

        sqlx::query(
            "INSERT INTO snippets (id, user_id, library_id, description, command, tags, created_at, updated_at, device_id, deleted, encrypted)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                description = excluded.description,
                command = excluded.command,
                tags = excluded.tags,
                updated_at = excluded.updated_at,
                device_id = excluded.device_id,
                deleted = excluded.deleted,
                encrypted = excluded.encrypted
             WHERE excluded.user_id = snippets.user_id AND excluded.library_id = snippets.library_id \
           AND (excluded.updated_at > snippets.updated_at \
                OR (excluded.updated_at = snippets.updated_at AND excluded.device_id > snippets.device_id))",
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
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn upsert_snippet_in_tx(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        snippet: &Snippet,
        user_id: &str,
        library_id: &str,
    ) -> DbResult<()> {
        let tags_json = serde_json::to_string(&snippet.tags).unwrap_or_else(|_| "[]".to_string());
        let deleted = snippet.deleted as i32;
        let encrypted = snippet.encrypted as i32;

        // Upsert with last-write-wins conflict resolution.
        // Tie-breaking uses device_id string comparison, which is correct for
        // lowercase hex UUIDs (lexicographic == numeric for lowercase hex).
        sqlx::query(
            "INSERT INTO snippets (id, user_id, library_id, description, command, tags, created_at, updated_at, device_id, deleted, encrypted)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                description = excluded.description,
                command = excluded.command,
                tags = excluded.tags,
                updated_at = excluded.updated_at,
                device_id = excluded.device_id,
                deleted = excluded.deleted,
                encrypted = excluded.encrypted
             WHERE excluded.user_id = snippets.user_id AND excluded.library_id = snippets.library_id \
           AND (excluded.updated_at > snippets.updated_at \
                OR (excluded.updated_at = snippets.updated_at AND excluded.device_id > snippets.device_id))",
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
        .execute(&mut **tx)
        .await?;

        Ok(())
    }

    pub async fn get_latest_timestamp(&self, user_id: &str, library_id: &str) -> DbResult<i64> {
        let (timestamp,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(updated_at), 0) FROM snippets WHERE user_id = ? AND library_id = ?",
        )
        .bind(user_id)
        .bind(library_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(timestamp)
    }

    pub async fn migrate_plaintext_api_keys(&self) -> DbResult<usize> {
        let rows: Vec<(String, String, Option<String>)> =
            sqlx::query_as("SELECT id, api_key, api_key_prefix FROM users")
                .fetch_all(&self.pool)
                .await?;

        let mut migrated = 0;

        for (user_id, stored, prefix) in rows {
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
                // For plaintext keys, prefix from plaintext; for hashed keys, we can't
                // compute prefix without the original key, so we skip those — they'll
                // still work via the `IS NULL` fallback in get_user_by_api_key.
                if !stored.starts_with("$argon2") {
                    new_prefix = Some(compute_api_key_prefix(&stored));
                    needs_update = true;
                }
            }

            if needs_update {
                sqlx::query("UPDATE users SET api_key = ?, api_key_prefix = ? WHERE id = ?")
                    .bind(&new_hash)
                    .bind(&new_prefix)
                    .bind(&user_id)
                    .execute(&self.pool)
                    .await?;
                migrated += 1;
            }
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
}
