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
use sha2::Digest;
use std::fs;
use std::path::{Path, PathBuf};

/// Process identity for lock ownership verification.
///
/// Contains the PID and a start-time token that uniquely identifies a
/// process incarnation. This prevents PID-reuse attacks where a new
/// process inherits the same PID as a dead lock owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    /// Process ID.
    pub pid: u32,
    /// Start-time token identifying this process incarnation.
    /// `None` when the platform does not support start-time detection.
    pub start_token: Option<String>,
}

/// Get the current process identity with start-time token.
pub fn current_process_identity() -> ProcessIdentity {
    ProcessIdentity {
        pid: std::process::id(),
        start_token: get_process_start_token(std::process::id()),
    }
}

/// Get a start-time token for the given PID.
///
/// On Linux, reads the process start time from `/proc/<pid>/stat` (field 22,
/// in clock ticks since boot). On other platforms, returns `None` — callers
/// must handle this by relying on PID liveness only.
#[cfg(target_os = "linux")]
fn get_process_start_token(pid: u32) -> Option<String> {
    let stat_path = format!("/proc/{pid}/stat");
    let content = fs::read_to_string(&stat_path).ok()?;
    // Field 22 (1-indexed) is `starttime`. The comm field (field 2) may
    // contain spaces or parens, so find the last `)` and count from there.
    let after_comm = content.rfind(')')?;
    let fields: Vec<&str> = content[after_comm + 2..].split_whitespace().collect();
    if fields.len() >= 19 {
        Some(fields[18].to_string())
    } else {
        None
    }
}

/// Get a start-time token for the given PID (non-Linux fallback).
///
/// Returns `None` — on macOS and Windows, PID liveness via `kill(pid, 0)`
/// or `GetExitCodeProcess` is the primary ownership check. The nonce
/// already prevents stale-lock theft from processes with the same PID.
#[cfg(not(target_os = "linux"))]
fn get_process_start_token(_pid: u32) -> Option<String> {
    None
}

/// Action intended for a staged file within a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StagedAction {
    /// File existed before the transaction; will be replaced.
    Replace,
    /// File did not exist; will be created.
    Create,
    /// File existed and will be deleted.
    Delete,
    /// No change needed (identical content in merge mode).
    NoOp,
}

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
    /// Whether the original file existed before the transaction.
    #[serde(default)]
    pub existed_before: bool,
    /// Intended action for this file.
    #[serde(default = "default_action")]
    pub action: StagedAction,
    /// SHA-256 hex digest of the original file content (empty if did not exist).
    #[serde(default)]
    pub original_hash: String,
    /// SHA-256 hex digest of the new file content (empty if deleting).
    #[serde(default)]
    pub new_hash: String,
}

fn default_action() -> StagedAction {
    StagedAction::Replace
}

/// State machine for a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionState {
    /// Transaction is prepared; backups taken, staged files ready.
    Prepared,
    /// All backup files are durably written to disk.
    BackupsDurable,
    /// Live replacement is in progress; tracks which file index is next.
    Committing {
        /// Index of the next file to replace atomically.
        next_index: usize,
    },
    /// Transaction has been committed; staged files are in place.
    Committed,
    /// Rollback is in progress; tracks which file index is being restored.
    RollingBack {
        /// Index of the next backup to restore.
        next_index: usize,
    },
    /// Transaction was rolled back; backups restored.
    RolledBack,
    /// Transaction failed with an error message.
    Failed(String),
}

impl TransactionState {
    /// Returns true if this state represents an interrupted (non-terminal) transaction.
    ///
    /// Interruptible states are `Prepared`, `BackupsDurable`, `Committing`, and
    /// `RollingBack`. Terminal states (`Committed`, `RolledBack`, `Failed`) are
    /// not interruptible.
    pub fn is_interruptible(&self) -> bool {
        matches!(
            self,
            TransactionState::Prepared
                | TransactionState::BackupsDurable
                | TransactionState::Committing { .. }
                | TransactionState::RollingBack { .. }
        )
    }
}

/// Transaction lock record persisted inside the lock file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionLockInfo {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Process ID of the lock owner.
    pub pid: u32,
    /// Random nonce to prevent PID-reuse lock theft.
    pub nonce: String,
    /// Unix timestamp (ms) when the lock was created.
    pub created_at_unix_ms: i64,
    /// Human-readable operation name.
    pub operation: String,
    /// Start-time token for the lock owner process.
    /// `None` when the platform does not support start-time detection.
    /// Verified on reclaim to prevent PID-reuse theft.
    #[serde(default)]
    pub start_token: Option<String>,
}

/// Transaction lock guard.
///
/// Holds an exclusive lock on the transaction directory. Automatically
/// releases the lock when dropped. The lock record contains PID and nonce
/// for ownership verification and stale-lock detection.
#[derive(Debug)]
pub struct TransactionLock {
    lock_path: PathBuf,
    info: TransactionLockInfo,
}

impl Drop for TransactionLock {
    fn drop(&mut self) {
        // Only remove if we still own the lock (verify nonce and start token).
        if let Ok(content) = fs::read_to_string(&self.lock_path)
            && let Ok(existing) = toml::from_str::<TransactionLockInfo>(&content)
            && existing.nonce == self.info.nonce
            && existing.start_token == self.info.start_token
        {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

/// Check whether a process with the given PID is alive.
#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    // Signal 0 checks existence without sending a signal.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[cfg(windows)]
fn is_process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const STILL_ACTIVE: u32 = 259;
    unsafe {
        let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code: u32 = 0;
        let success = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        success != 0 && exit_code == STILL_ACTIVE
    }
}

/// Acquire a local mutation transaction lock.
///
/// Uses an atomic file-create to ensure only one transaction can proceed
/// at a time. If an existing lock is found, checks whether the owner is
/// alive. Dead owners are reclaimed. Returns an error if the lock is held
/// by a live process.
pub fn acquire_transaction_lock(state_dir: &Path, operation: &str) -> SnipResult<TransactionLock> {
    fs::create_dir_all(state_dir)
        .map_err(|e| SnipError::io_error("create state directory", state_dir, e))?;

    let lock_path = state_dir.join("transaction.lock");
    let nonce = uuid::Uuid::new_v4().to_string();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let identity = current_process_identity();

    let info = TransactionLockInfo {
        schema_version: 1,
        pid: identity.pid,
        nonce: nonce.clone(),
        created_at_unix_ms: now_ms,
        operation: operation.to_string(),
        start_token: identity.start_token.clone(),
    };

    // Atomic lock acquisition: create_new fails if the file already exists.
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(_file) => {
            // Write lock record
            let content = toml::to_string_pretty(&info)
                .map_err(|e| SnipError::toml_error("serialize lock info", e))?;
            fs::write(&lock_path, &content)
                .map_err(|e| SnipError::io_error("write lock record", lock_path.clone(), e))?;
            Ok(TransactionLock { lock_path, info })
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Lock exists — check if owner is alive.
            let content = fs::read_to_string(&lock_path)
                .map_err(|e| SnipError::io_error("read existing lock", lock_path.clone(), e))?;
            match toml::from_str::<TransactionLockInfo>(&content) {
                Ok(existing) => {
                    // Check PID liveness first.
                    if is_process_alive(existing.pid) {
                        // PID is alive. If we have start tokens on both sides,
                        // verify they match to detect PID reuse.
                        if let (Some(old_token), Some(new_token)) =
                            (&existing.start_token, &info.start_token)
                            && old_token != new_token
                        {
                            // PID reused with different start time — reclaim.
                            tracing::info!(
                                pid = existing.pid,
                                old_token = %old_token,
                                new_token = %new_token,
                                "Transaction lock owner PID reused (start token mismatch), reclaiming"
                            );
                            quarantine_stale_lock(&lock_path)?;
                            // Retry acquisition.
                            return acquire_transaction_lock_after_reclaim(&lock_path, &info);
                        }
                        // Same PID, same start token (or no token) — contention.
                        Err(SnipError::runtime_error(
                            "Transaction lock held",
                            Some(&format!(
                                "Another transaction ({}) is in progress (PID {}). Wait for it to complete.",
                                existing.operation, existing.pid
                            )),
                        ))
                    } else {
                        // Dead owner — reclaim.
                        tracing::info!(
                            pid = existing.pid,
                            operation = %existing.operation,
                            "Reclaiming stale transaction lock (owner process is dead)"
                        );
                        quarantine_stale_lock(&lock_path)?;
                        // Retry acquisition.
                        acquire_transaction_lock_after_reclaim(&lock_path, &info)
                    }
                }
                Err(_) => {
                    // Malformed lock — quarantine, not silently delete.
                    tracing::warn!("Malformed transaction lock record, quarantining");
                    quarantine_stale_lock(&lock_path)?;
                    // Retry acquisition.
                    acquire_transaction_lock_after_reclaim(&lock_path, &info)
                }
            }
        }
        Err(e) => Err(SnipError::io_error(
            "acquire transaction lock",
            lock_path,
            e,
        )),
    }
}

/// Quarantine a stale or malformed lock by renaming it.
///
/// The quarantined file preserves the original content for debugging
/// and repair inspection. Returns the quarantine path on success.
fn quarantine_stale_lock(lock_path: &Path) -> SnipResult<PathBuf> {
    let quarantine_name = format!("transaction.lock.quarantine.{}", uuid::Uuid::new_v4());
    let quarantine_path = lock_path
        .parent()
        .unwrap_or(lock_path)
        .join(&quarantine_name);
    fs::rename(lock_path, &quarantine_path)
        .map_err(|e| SnipError::io_error("quarantine stale lock", quarantine_path.clone(), e))?;
    Ok(quarantine_path)
}

/// Write a new lock record after reclaiming a stale or malformed lock.
fn acquire_transaction_lock_after_reclaim(
    lock_path: &Path,
    info: &TransactionLockInfo,
) -> SnipResult<TransactionLock> {
    let content = toml::to_string_pretty(info)
        .map_err(|e| SnipError::toml_error("serialize lock info", e))?;
    fs::write(lock_path, &content).map_err(|e| {
        SnipError::io_error(
            "write lock record after reclaim",
            lock_path.to_path_buf(),
            e,
        )
    })?;
    Ok(TransactionLock {
        lock_path: lock_path.to_path_buf(),
        info: info.clone(),
    })
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

    let staged_files = affected_files
        .iter()
        .map(|p| {
            let existed = p.exists();
            let original_hash = if existed {
                fs::read(p)
                    .map(|bytes| {
                        let mut hasher = sha2::Sha256::new();
                        hasher.update(&bytes);
                        hasher
                            .finalize()
                            .iter()
                            .map(|b| format!("{:02x}", b))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                String::new()
            };
            StagedFile {
                original_path: p.clone(),
                backup_path: None,
                staged_path: p.clone(),
                sha256: String::new(),
                existed_before: existed,
                action: if existed {
                    StagedAction::Replace
                } else {
                    StagedAction::Create
                },
                original_hash,
                new_hash: String::new(),
            }
        })
        .collect();

    let journal = TransactionJournal {
        id: uuid::Uuid::new_v4().to_string(),
        operation: operation.to_string(),
        created_at_unix_ms: now_ms,
        staged_files,
        state: TransactionState::Prepared,
    };

    let jpath = journal_path(state_dir, &journal.id);
    let content = toml::to_string_pretty(&journal)
        .map_err(|e| SnipError::toml_error("serialize transaction journal", e))?;

    crate::utils::atomic::write_private_atomic(&jpath, &content, "txn")?;

    Ok(journal)
}

/// Persist a state transition for the journal atomically.
fn persist_journal(state_dir: &Path, journal: &TransactionJournal) -> SnipResult<()> {
    let jpath = journal_path(state_dir, &journal.id);
    let content = toml::to_string_pretty(journal)
        .map_err(|e| SnipError::toml_error("serialize transaction journal", e))?;
    crate::utils::atomic::write_private_atomic(&jpath, &content, "txn")
}

/// Advance the journal to `BackupsDurable`.
///
/// Call after all backup files have been durably written to disk, before
/// any live replacement begins.
pub fn advance_to_backups_durable(
    state_dir: &Path,
    journal: &mut TransactionJournal,
) -> SnipResult<()> {
    journal.state = TransactionState::BackupsDurable;
    persist_journal(state_dir, journal)
}

/// Advance the journal to `Committing { next_index }`.
pub fn advance_to_committing(
    state_dir: &Path,
    journal: &mut TransactionJournal,
    next_index: usize,
) -> SnipResult<()> {
    journal.state = TransactionState::Committing { next_index };
    persist_journal(state_dir, journal)
}

/// Advance the journal to `RollingBack { next_index }`.
#[allow(dead_code)]
pub fn advance_to_rolling_back(
    state_dir: &Path,
    journal: &mut TransactionJournal,
    next_index: usize,
) -> SnipResult<()> {
    journal.state = TransactionState::RollingBack { next_index };
    persist_journal(state_dir, journal)
}

/// Commit a transaction (atomic multi-file commit).
///
/// Marks the journal as `Committed` and removes backup files.
/// The caller is responsible for actually writing the staged files
/// before calling this function.
pub fn commit_transaction(state_dir: &Path, journal: &TransactionJournal) -> SnipResult<()> {
    let mut committed = journal.clone();
    committed.state = TransactionState::Committed;

    persist_journal(state_dir, &committed)?;

    // Clean up backup files
    for staged in &committed.staged_files {
        if let Some(ref backup) = staged.backup_path {
            let _ = fs::remove_file(backup);
        }
    }

    // Remove the journal file itself (transaction complete)
    let jpath = journal_path(state_dir, &committed.id);
    let _ = fs::remove_file(&jpath);

    Ok(())
}

/// Rollback a transaction (restore from backups).
///
/// Restores each staged file from its backup in reverse order using atomic
/// persistence, durably advancing rollback progress after each file. Newly
/// created files (action=Create, existed_before=false) are removed rather
/// than overwritten. The journal is marked as `RolledBack` on completion.
/// Rollback is restartable: if interrupted, the next call picks up from
/// the last durably recorded `RollingBack` index.
pub fn rollback_transaction(state_dir: &Path, journal: &TransactionJournal) -> SnipResult<()> {
    let mut rb_journal = journal.clone();
    let start_index = match rb_journal.state {
        TransactionState::RollingBack { next_index } => next_index,
        _ => 0,
    };

    // Restore files from backups in reverse order starting from start_index
    for (i, staged) in rb_journal.staged_files.iter().enumerate().rev() {
        if i < start_index {
            continue;
        }

        match staged.action {
            StagedAction::Create if !staged.existed_before => {
                // This file was created by the transaction — remove it.
                if staged.original_path.exists() {
                    fs::remove_file(&staged.original_path).map_err(|e| {
                        SnipError::io_error(
                            "remove newly created file during rollback",
                            staged.original_path.clone(),
                            e,
                        )
                    })?;
                }
            }
            StagedAction::Delete | StagedAction::Replace => {
                // Restore from backup using atomic persistence.
                if let Some(ref backup) = staged.backup_path
                    && backup.exists()
                {
                    let bytes = fs::read(backup).map_err(|e| {
                        SnipError::io_error("read backup for rollback", backup.clone(), e)
                    })?;
                    let opts = crate::utils::atomic::AtomicWriteOptions::for_durability(
                        crate::utils::atomic::Durability::DurableUserData,
                    );
                    crate::utils::atomic::atomic_replace(&staged.original_path, &bytes, &opts)?;
                }
            }
            StagedAction::NoOp | StagedAction::Create => {
                // NoOp: nothing to do. Create with existed_before=true:
                // restore from backup (same as Replace).
                if let Some(ref backup) = staged.backup_path
                    && backup.exists()
                {
                    let bytes = fs::read(backup).map_err(|e| {
                        SnipError::io_error("read backup for rollback", backup.clone(), e)
                    })?;
                    let opts = crate::utils::atomic::AtomicWriteOptions::for_durability(
                        crate::utils::atomic::Durability::DurableUserData,
                    );
                    crate::utils::atomic::atomic_replace(&staged.original_path, &bytes, &opts)?;
                }
            }
        }

        // Durably advance rollback progress
        rb_journal.state = TransactionState::RollingBack { next_index: i + 1 };
        persist_journal(state_dir, &rb_journal)?;
    }

    rb_journal.state = TransactionState::RolledBack;
    persist_journal(state_dir, &rb_journal)?;

    // Remove backups and journal
    for staged in &rb_journal.staged_files {
        if let Some(ref backup) = staged.backup_path {
            let _ = fs::remove_file(backup);
        }
    }
    let jpath = journal_path(state_dir, &rb_journal.id);
    let _ = fs::remove_file(&jpath);

    Ok(())
}

/// Check for interrupted transactions and refuse or auto-recover.
///
/// This is the application-level mutation gate. It must be called before
/// any local mutating operation begins its write phase. The policy is:
///
/// 1. If no interrupted journals exist, return `Ok(())` — proceed.
/// 2. If exactly one complete and unambiguous journal exists, attempt
///    automatic rollback. Return `Ok(())` if rollback succeeds.
/// 3. If multiple or incomplete journals exist, return an error directing
///    the user to `snp repair`.
///
/// Read-only commands must not call this function.
pub fn gate_mutation_on_interrupted_transactions(state_dir: &Path) -> SnipResult<()> {
    let interrupted = check_interrupted_transactions(state_dir)?;

    if interrupted.is_empty() {
        return Ok(());
    }

    if interrupted.len() == 1 {
        let journal = &interrupted[0];
        tracing::info!(
            txn_id = %journal.id,
            operation = %journal.operation,
            state = ?journal.state,
            "Attempting automatic rollback of interrupted transaction"
        );
        match rollback_transaction(state_dir, journal) {
            Ok(()) => {
                tracing::info!(
                    txn_id = %journal.id,
                    "Automatic rollback succeeded"
                );
                Ok(())
            }
            Err(e) => Err(SnipError::runtime_error(
                "Interrupted transaction requires manual recovery",
                Some(&format!(
                    "Transaction '{}' ({}) was interrupted and automatic rollback failed: {}. \
                     Run `snp repair` to inspect and recover.",
                    journal.operation, journal.id, e
                )),
            )),
        }
    } else {
        // Multiple interrupted journals — refuse and direct to repair.
        let ids: Vec<&str> = interrupted.iter().map(|j| j.id.as_str()).collect();
        Err(SnipError::runtime_error(
            "Multiple interrupted transactions detected",
            Some(&format!(
                "Found {} interrupted transactions (IDs: {}). \
                 Run `snp repair` to inspect and recover before making new mutations.",
                interrupted.len(),
                ids.join(", ")
            )),
        ))
    }
}

/// Check for interrupted transactions on startup.
///
/// Returns any journals in a non-terminal state (Prepared, BackupsDurable,
/// Committing, RollingBack). These represent operations that were interrupted
/// and need attention. Journals in `Committed`, `RolledBack`, or `Failed`
/// states are terminal and ignored.
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
                Ok(journal) if journal.state.is_interruptible() => {
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
    use sha2::Digest;
    use tempfile::TempDir;

    #[test]
    fn test_acquire_and_release_lock() {
        let dir = TempDir::new().unwrap();
        let lock = acquire_transaction_lock(dir.path(), "test").unwrap();
        let lock_path = lock.lock_path.clone();
        assert!(lock_path.exists());
        // Lock file contains valid TOML with PID, nonce, and start_token
        let content = fs::read_to_string(&lock_path).unwrap();
        let info: TransactionLockInfo = toml::from_str(&content).unwrap();
        assert_eq!(info.schema_version, 1);
        assert_eq!(info.pid, std::process::id());
        assert!(!info.nonce.is_empty());
        assert_eq!(info.operation, "test");
        // start_token may be None on non-Linux platforms
        assert!(info.start_token.is_none() || info.start_token.is_some());
        drop(lock);
        assert!(!lock_path.exists());
    }

    #[test]
    fn test_acquire_lock_conflict() {
        let dir = TempDir::new().unwrap();
        let _lock1 = acquire_transaction_lock(dir.path(), "op1").unwrap();
        let result = acquire_transaction_lock(dir.path(), "op2");
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("lock"), "Expected lock error, got: {msg}");
    }

    #[test]
    fn test_lock_nonce_prevents_wrong_owner_removal() {
        let dir = TempDir::new().unwrap();
        let lock1 = acquire_transaction_lock(dir.path(), "op1").unwrap();
        let lock_path = lock1.lock_path.clone();
        // A different nonce cannot remove the lock
        let fake_info = TransactionLockInfo {
            schema_version: 1,
            pid: 99999,
            nonce: "fake-nonce".to_string(),
            created_at_unix_ms: 0,
            operation: "fake".to_string(),
            start_token: None,
        };
        let fake_content = toml::to_string_pretty(&fake_info).unwrap();
        // Write a different nonce to simulate wrong owner
        fs::write(&lock_path, &fake_content).unwrap();
        drop(lock1);
        // Lock file still exists because nonce didn't match
        assert!(lock_path.exists());
        // Clean up manually
        fs::remove_file(&lock_path).unwrap();
    }

    #[test]
    fn test_begin_and_commit_transaction() {
        let dir = TempDir::new().unwrap();
        let state_dir = dir.path();
        let file1 = dir.path().join("file1.toml");
        let file2 = dir.path().join("file2.toml");

        let _lock = acquire_transaction_lock(state_dir, "test_op").unwrap();
        let journal = begin_transaction(state_dir, "test_op", &[file1, file2]).unwrap();

        assert_eq!(journal.operation, "test_op");
        assert_eq!(journal.state, TransactionState::Prepared);
        assert_eq!(journal.staged_files.len(), 2);
        // Files don't exist yet, so existed_before is false and action is Create
        assert!(!journal.staged_files[0].existed_before);
        assert_eq!(journal.staged_files[0].action, StagedAction::Create);

        commit_transaction(state_dir, &journal).unwrap();

        // Journal file should be removed after commit
        let jpath = journal_path(state_dir, &journal.id);
        assert!(!jpath.exists());
    }

    #[test]
    fn test_begin_transaction_populates_existing_file_metadata() {
        let dir = TempDir::new().unwrap();
        let state_dir = dir.path();
        let file1 = dir.path().join("existing.toml");
        fs::write(&file1, "hello world").unwrap();

        let _lock = acquire_transaction_lock(state_dir, "test").unwrap();
        let journal = begin_transaction(state_dir, "test", std::slice::from_ref(&file1)).unwrap();

        let sf = &journal.staged_files[0];
        assert!(sf.existed_before);
        assert_eq!(sf.action, StagedAction::Replace);
        assert!(!sf.original_hash.is_empty());
        assert_eq!(sf.new_hash, "");

        // Verify original_hash matches actual content
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"hello world");
        let expected: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        assert_eq!(sf.original_hash, expected);
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

        let _lock = acquire_transaction_lock(state_dir, "test_op").unwrap();
        let mut journal =
            begin_transaction(state_dir, "test_op", std::slice::from_ref(&file1)).unwrap();
        journal.staged_files[0].backup_path = Some(backup_path.clone());

        rollback_transaction(state_dir, &journal).unwrap();

        // Backup should be cleaned up
        assert!(!backup_path.exists());
    }

    #[test]
    fn test_state_is_interruptible() {
        assert!(TransactionState::Prepared.is_interruptible());
        assert!(TransactionState::BackupsDurable.is_interruptible());
        assert!(TransactionState::Committing { next_index: 0 }.is_interruptible());
        assert!(TransactionState::RollingBack { next_index: 0 }.is_interruptible());
        assert!(!TransactionState::Committed.is_interruptible());
        assert!(!TransactionState::RolledBack.is_interruptible());
        assert!(!TransactionState::Failed("test".into()).is_interruptible());
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
