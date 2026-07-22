//! **Layer: Domain/Core**
//!
//! Local mutation transaction boundary.
//!
//! Provides lightweight transaction coordination for operations that affect
//! multiple files (library create/delete, bulk import, restore, repair).
//!
//! The transaction journal is persisted to disk so that interrupted operations
//! can be detected and either rolled forward (commit) or rolled back on
//! startup. The lock prevents concurrent transactions from corrupting shared
//! state.

use crate::error::{SnipError, SnipResult};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Transaction state persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransactionJournal {
    /// Unique transaction identifier (UUID).
    pub id: String,
    /// Human-readable operation name (e.g. "library_delete", "bulk_import").
    pub operation: String,
    /// Unix timestamp (ms) when the transaction was created.
    pub created_at_unix_ms: i64,
    /// Files affected by this transaction.
    pub staged_files: Vec<StagedFile>,
    /// Current state of the transaction.
    pub state: TransactionState,
}

/// A file staged within a transaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StagedFile {
    /// The original file path being modified.
    pub original_path: PathBuf,
    /// Backup of the original file, if created.
    pub backup_path: Option<PathBuf>,
    /// The new/staged replacement path (may be the same as original_path).
    pub staged_path: PathBuf,
    /// SHA-256 hex digest of the staged content for integrity verification.
    pub sha256: String,
}

/// State machine for a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionState {
    /// Transaction is prepared; backups taken, staged files ready.
    Prepared,
    /// Transaction has been committed; staged files are in place.
    Committed,
    /// Transaction was rolled back; backups restored.
    RolledBack,
    /// Transaction failed with an error message.
    Failed(String),
}

/// Transaction lock guard.
///
/// Holds an exclusive lock on the transaction directory. Automatically
/// releases the lock when dropped.
#[derive(Debug)]
pub struct TransactionLock {
    lock_path: PathBuf,
}

impl Drop for TransactionLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

/// Acquire a local mutation transaction lock.
///
/// Uses an atomic file-create to ensure only one transaction can proceed
/// at a time. Returns an error if the lock is already held.
pub fn acquire_transaction_lock(state_dir: &Path) -> SnipResult<TransactionLock> {
    fs::create_dir_all(state_dir)
        .map_err(|e| SnipError::io_error("create state directory", state_dir, e))?;

    let lock_path = state_dir.join("transaction.lock");

    // Atomic lock acquisition: create_new fails if the file already exists.
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(_file) => Ok(TransactionLock { lock_path }),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(SnipError::runtime_error(
            "Transaction lock held",
            Some(
                "Another transaction is in progress. Wait for it to complete or remove the lock file manually.",
            ),
        )),
        Err(e) => Err(SnipError::io_error(
            "acquire transaction lock",
            lock_path,
            e,
        )),
    }
}

/// Begin a new transaction.
///
/// Creates a journal file in the `state_dir` with `Prepared` state.
/// Caller must already hold the transaction lock.
pub fn begin_transaction(
    state_dir: &Path,
    operation: &str,
    affected_files: &[PathBuf],
) -> SnipResult<TransactionJournal> {
    fs::create_dir_all(state_dir)
        .map_err(|e| SnipError::io_error("create state directory", state_dir, e))?;

    let now_ms = chrono::Utc::now().timestamp_millis();

    let journal = TransactionJournal {
        id: uuid::Uuid::new_v4().to_string(),
        operation: operation.to_string(),
        created_at_unix_ms: now_ms,
        staged_files: affected_files
            .iter()
            .map(|p| StagedFile {
                original_path: p.clone(),
                backup_path: None,
                staged_path: p.clone(),
                sha256: String::new(),
            })
            .collect(),
        state: TransactionState::Prepared,
    };

    let journal_path = journal_path(state_dir, &journal.id);
    let content = toml::to_string_pretty(&journal)
        .map_err(|e| SnipError::toml_error("serialize transaction journal", e))?;

    crate::utils::atomic::write_private_atomic(&journal_path, &content, "txn")?;

    Ok(journal)
}

/// Commit a transaction (atomic multi-file commit).
///
/// Marks the journal as `Committed` and removes backup files.
/// The caller is responsible for actually writing the staged files
/// before calling this function.
pub fn commit_transaction(state_dir: &Path, journal: &TransactionJournal) -> SnipResult<()> {
    let mut committed = journal.clone();
    committed.state = TransactionState::Committed;

    let jpath = journal_path(state_dir, &committed.id);
    let content = toml::to_string_pretty(&committed)
        .map_err(|e| SnipError::toml_error("serialize transaction journal", e))?;

    crate::utils::atomic::write_private_atomic(&jpath, &content, "txn")?;

    // Clean up backup files
    for staged in &committed.staged_files {
        if let Some(ref backup) = staged.backup_path {
            let _ = fs::remove_file(backup);
        }
    }

    // Remove the journal file itself (transaction complete)
    let _ = fs::remove_file(&jpath);

    Ok(())
}

/// Rollback a transaction (restore from backups).
///
/// Restores each staged file from its backup, then marks the journal
/// as `RolledBack`.
pub fn rollback_transaction(state_dir: &Path, journal: &TransactionJournal) -> SnipResult<()> {
    // Restore files from backups in reverse order
    for staged in journal.staged_files.iter().rev() {
        if let Some(ref backup) = staged.backup_path
            && backup.exists()
        {
            fs::copy(backup, &staged.original_path).map_err(|e| {
                SnipError::io_error("restore file from backup", staged.original_path.clone(), e)
            })?;
        }
    }

    let mut rolled_back = journal.clone();
    rolled_back.state = TransactionState::RolledBack;

    let jpath = journal_path(state_dir, &rolled_back.id);
    let content = toml::to_string_pretty(&rolled_back)
        .map_err(|e| SnipError::toml_error("serialize transaction journal", e))?;

    crate::utils::atomic::write_private_atomic(&jpath, &content, "txn")?;

    // Remove backups and journal
    for staged in &rolled_back.staged_files {
        if let Some(ref backup) = staged.backup_path {
            let _ = fs::remove_file(backup);
        }
    }
    let _ = fs::remove_file(&jpath);

    Ok(())
}

/// Check for interrupted transactions on startup.
///
/// Returns any journals in `Prepared` state (neither committed nor rolled back).
/// These represent operations that were interrupted and need attention.
pub fn check_interrupted_transactions(state_dir: &Path) -> SnipResult<Vec<TransactionJournal>> {
    if !state_dir.exists() {
        return Ok(Vec::new());
    }

    let mut interrupted = Vec::new();

    for entry in fs::read_dir(state_dir)
        .map_err(|e| SnipError::io_error("read state directory", state_dir, e))?
    {
        let entry =
            entry.map_err(|e| SnipError::io_error("read state directory entry", state_dir, e))?;

        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml")
            && path
                .file_stem()
                .is_some_and(|s| s.to_string_lossy().starts_with("txn-"))
        {
            let content = fs::read_to_string(&path)
                .map_err(|e| SnipError::io_error("read transaction journal", path.clone(), e))?;

            match toml::from_str::<TransactionJournal>(&content) {
                Ok(journal) if journal.state == TransactionState::Prepared => {
                    interrupted.push(journal);
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Corrupt transaction journal, skipping"
                    );
                }
            }
        }
    }

    Ok(interrupted)
}

/// Derive the journal file path for a given transaction ID.
fn journal_path(state_dir: &Path, txn_id: &str) -> PathBuf {
    state_dir.join(format!("txn-{txn_id}.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_acquire_and_release_lock() {
        let dir = TempDir::new().unwrap();
        let lock = acquire_transaction_lock(dir.path()).unwrap();
        let lock_path = lock.lock_path.clone();
        assert!(lock_path.exists());
        drop(lock);
        assert!(!lock_path.exists());
    }

    #[test]
    fn test_acquire_lock_conflict() {
        let dir = TempDir::new().unwrap();
        let _lock1 = acquire_transaction_lock(dir.path()).unwrap();
        let result = acquire_transaction_lock(dir.path());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("lock"), "Expected lock error, got: {msg}");
    }

    #[test]
    fn test_begin_and_commit_transaction() {
        let dir = TempDir::new().unwrap();
        let state_dir = dir.path();
        let file1 = dir.path().join("file1.toml");
        let file2 = dir.path().join("file2.toml");

        let _lock = acquire_transaction_lock(state_dir).unwrap();
        let journal = begin_transaction(state_dir, "test_op", &[file1, file2]).unwrap();

        assert_eq!(journal.operation, "test_op");
        assert_eq!(journal.state, TransactionState::Prepared);
        assert_eq!(journal.staged_files.len(), 2);

        commit_transaction(state_dir, &journal).unwrap();

        // Journal file should be removed after commit
        let jpath = journal_path(state_dir, &journal.id);
        assert!(!jpath.exists());
    }

    #[test]
    fn test_begin_and_rollback_transaction() {
        let dir = TempDir::new().unwrap();
        let state_dir = dir.path();
        let file1 = dir.path().join("file1.toml");

        // Create the file and a backup
        fs::write(&file1, "original").unwrap();
        let backup_dir = dir.path().join("backups");
        fs::create_dir_all(&backup_dir).unwrap();
        let backup_path = backup_dir.join("file1.toml.bak");
        fs::copy(&file1, &backup_path).unwrap();

        let _lock = acquire_transaction_lock(state_dir).unwrap();
        let mut journal =
            begin_transaction(state_dir, "test_op", std::slice::from_ref(&file1)).unwrap();
        journal.staged_files[0].backup_path = Some(backup_path.clone());

        rollback_transaction(state_dir, &journal).unwrap();

        // Backup should be cleaned up
        assert!(!backup_path.exists());
    }

    #[test]
    fn test_check_interrupted_empty() {
        let dir = TempDir::new().unwrap();
        let interrupted = check_interrupted_transactions(dir.path()).unwrap();
        assert!(interrupted.is_empty());
    }

    #[test]
    fn test_transaction_state_serialization() {
        let state = TransactionState::Failed("test error".to_string());
        let json = serde_json::to_string(&state).unwrap();
        let deserialized: TransactionState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, deserialized);
    }
}
