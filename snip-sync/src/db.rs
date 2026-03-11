use chrono::Utc;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum DbError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Unauthorized: {0}")]
    Unauthorized(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Conflict: {0}")]
    Conflict(String),
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

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new<P: AsRef<Path>>(path: P) -> DbResult<Self> {
        let conn = Connection::open(path)?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                api_key TEXT UNIQUE NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            
            CREATE TABLE IF NOT EXISTS libraries (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                name TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                deleted_at INTEGER,
                FOREIGN KEY (user_id) REFERENCES users(id),
                UNIQUE(user_id, name)
            );
            
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
            );
            
            CREATE INDEX IF NOT EXISTS idx_snippets_user ON snippets(user_id);
            CREATE INDEX IF NOT EXISTS idx_snippets_library ON snippets(library_id);
            CREATE INDEX IF NOT EXISTS idx_snippets_updated ON snippets(updated_at);
            CREATE INDEX IF NOT EXISTS idx_libraries_user ON libraries(user_id);
            ",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn create_user(&self, api_key: &str) -> DbResult<String> {
        let conn = self.conn.lock().unwrap();
        let user_id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();

        conn.execute(
            "INSERT INTO users (id, api_key, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![user_id, api_key, now, now],
        )?;

        // Create default library
        let default_lib_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO libraries (id, user_id, name, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![default_lib_id, user_id, "default", now],
        )?;

        tracing::info!(
            "Created user {} with default library {}",
            user_id,
            default_lib_id
        );

        Ok(user_id)
    }

    pub fn get_user_by_api_key(&self, api_key: &str) -> DbResult<Option<String>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare("SELECT id FROM users WHERE api_key = ?1")?;
        let mut rows = stmt.query(params![api_key])?;

        if let Some(row) = rows.next()? {
            Ok(Some(row.get(0)?))
        } else {
            Ok(None)
        }
    }

    pub fn create_library(&self, user_id: &str, name: &str) -> DbResult<String> {
        let conn = self.conn.lock().unwrap();

        // Validate name (alphanumeric, dash, underscore, 1-64 chars)
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(DbError::Conflict(
                "Invalid library name. Use only letters, numbers, dash, and underscore."
                    .to_string(),
            ));
        }
        if name.len() > 64 {
            return Err(DbError::Conflict(
                "Library name too long (max 64 characters)".to_string(),
            ));
        }

        let lib_id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();

        // Use INSERT OR IGNORE to check if it already exists
        let result = conn.execute(
            "INSERT OR IGNORE INTO libraries (id, user_id, name, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![lib_id, user_id, name, now],
        )?;

        if result == 0 {
            return Err(DbError::Conflict(format!(
                "Library '{}' already exists",
                name
            )));
        }

        Ok(lib_id)
    }

    pub fn list_libraries(
        &self,
        user_id: &str,
        limit: i32,
        offset: i32,
    ) -> DbResult<(Vec<Library>, i32)> {
        let conn = self.conn.lock().unwrap();

        // Get total count
        let total: i32 = conn.query_row(
            "SELECT COUNT(*) FROM libraries WHERE user_id = ?1 AND deleted_at IS NULL",
            params![user_id],
            |row| row.get(0),
        )?;

        let mut stmt = conn.prepare(
            "SELECT l.id, l.name, l.created_at, 
                    (SELECT COUNT(*) FROM snippets WHERE library_id = l.id AND deleted = 0) as count
             FROM libraries l
             WHERE l.user_id = ?1 AND l.deleted_at IS NULL
             ORDER BY l.name
             LIMIT ?2 OFFSET ?3",
        )?;

        let rows = stmt.query_map(params![user_id, limit, offset], |row| {
            Ok(Library {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                snippet_count: row.get(3)?,
            })
        })?;

        let mut libraries = Vec::new();
        for row in rows {
            libraries.push(row?);
        }

        Ok((libraries, total))
    }

    pub fn delete_library(&self, user_id: &str, library_id: &str) -> DbResult<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().timestamp();

        // First, soft delete the library
        let result = conn.execute(
            "UPDATE libraries SET deleted_at = ?1 
             WHERE id = ?2 AND user_id = ?3 AND deleted_at IS NULL",
            params![now, library_id, user_id],
        )?;

        if result == 0 {
            return Err(DbError::NotFound(
                "Library not found or already deleted".to_string(),
            ));
        }

        // Cascade delete all snippets in this library
        conn.execute(
            "UPDATE snippets SET deleted = 1, updated_at = ?1 WHERE library_id = ?2 AND deleted = 0",
            params![now, library_id],
        )?;

        Ok(())
    }

    pub fn get_default_library(&self, user_id: &str) -> DbResult<String> {
        let conn = self.conn.lock().unwrap();

        let lib_id: String = conn.query_row(
            "SELECT id FROM libraries WHERE user_id = ?1 AND name = 'default' AND deleted_at IS NULL",
            params![user_id],
            |row| row.get(0),
        ).map_err(|_| DbError::NotFound("Default library not found".to_string()))?;

        Ok(lib_id)
    }

    pub fn get_snippets(
        &self,
        user_id: &str,
        library_id: &str,
        since: i64,
        limit: i32,
        offset: i32,
    ) -> DbResult<(Vec<Snippet>, i32)> {
        let conn = self.conn.lock().unwrap();

        // Get total count
        let total: i32 = conn.query_row(
            "SELECT COUNT(*) FROM snippets WHERE user_id = ?1 AND library_id = ?2 AND updated_at > ?3 AND deleted = 0",
            params![user_id, library_id, since],
            |row| row.get(0),
        )?;

        let mut stmt = conn.prepare(
            "SELECT id, description, command, tags, created_at, updated_at, device_id, deleted, encrypted 
             FROM snippets 
             WHERE user_id = ?1 AND library_id = ?2 AND updated_at > ?3 AND deleted = 0
             ORDER BY updated_at DESC
             LIMIT ?4 OFFSET ?5",
        )?;

        let rows = stmt.query_map(params![user_id, library_id, since, limit, offset], |row| {
            let tags_str: String = row.get(3)?;
            let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();

            Ok(Snippet {
                id: row.get(0)?,
                description: row.get(1)?,
                command: row.get(2)?,
                tags,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                device_id: row.get(6)?,
                deleted: row.get::<_, i32>(7)? != 0,
                encrypted: row.get::<_, i32>(8)? != 0,
            })
        })?;

        let mut snippets = Vec::new();
        for row in rows {
            snippets.push(row?);
        }

        Ok((snippets, total))
    }

    pub fn upsert_snippet(
        &self,
        snippet: &Snippet,
        user_id: &str,
        library_id: &str,
    ) -> DbResult<()> {
        let conn = self.conn.lock().unwrap();
        let tags_json = serde_json::to_string(&snippet.tags).unwrap_or_else(|_| "[]".to_string());

        conn.execute(
            "INSERT INTO snippets (id, user_id, library_id, description, command, tags, created_at, updated_at, device_id, deleted, encrypted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                description = excluded.description,
                command = excluded.command,
                tags = excluded.tags,
                updated_at = excluded.updated_at,
                device_id = excluded.device_id,
                deleted = excluded.deleted,
                encrypted = excluded.encrypted
             WHERE excluded.user_id = snippets.user_id AND excluded.library_id = snippets.library_id AND excluded.updated_at > snippets.updated_at",
            params![
                snippet.id,
                user_id,
                library_id,
                snippet.description,
                snippet.command,
                tags_json,
                snippet.created_at,
                snippet.updated_at,
                snippet.device_id,
                snippet.deleted as i32,
                snippet.encrypted as i32
            ],
        )?;

        Ok(())
    }

    pub fn get_latest_timestamp(&self, user_id: &str, library_id: &str) -> DbResult<i64> {
        let conn = self.conn.lock().unwrap();

        let mut stmt =
            conn.prepare("SELECT COALESCE(MAX(updated_at), 0) FROM snippets WHERE user_id = ?1 AND library_id = ?2")?;

        let timestamp: i64 = stmt.query_row(params![user_id, library_id], |row| row.get(0))?;
        Ok(timestamp)
    }
}
