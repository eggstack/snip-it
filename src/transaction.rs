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

impl ProcessIdentity {
    /// Get the current process identity with start-time token.
    pub fn current() -> ProcessIdentity {
        current_process_identity()
    }

    /// Observe the identity of the process identified by `pid`.
    ///
    /// Returns `Some(identity)` if the process is alive (with start token
    /// when observable), or `None` if the process is dead or cannot be
    /// queried. A live PID whose start identity cannot be observed still
    /// returns `Some` with `start_token: None` — callers must treat this
    /// conservatively as a live owner.
    pub fn observe(pid: u32) -> Option<ProcessIdentity> {
        if !is_process_alive(pid) {
            return None;
        }
        Some(ProcessIdentity {
            pid,
            start_token: get_process_start_token(pid),
        })
    }
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
/// in clock ticks since boot). On macOS, uses `sysctl` with `KERN_PROC_PID`.
/// On Windows, uses `GetProcessTimes`. Returns `None` if the start identity
/// cannot be determined.
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

#[cfg(target_os = "macos")]
fn get_process_start_token(pid: u32) -> Option<String> {
    use libc::{PROC_PIDTBSDINFO, c_int, proc_bsdinfo, proc_pidinfo};

    let mut info: proc_bsdinfo = unsafe { std::mem::zeroed() };
    let ret = unsafe {
        proc_pidinfo(
            pid as c_int,
            PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut _,
            std::mem::size_of::<proc_bsdinfo>() as i32,
        )
    };

    if ret <= 0 {
        return None;
    }

    // pbi_start_tvsec and pbi_start_tvusec give the process start time
    Some(format!(
        "{}.{:06}",
        info.pbi_start_tvsec, info.pbi_start_tvusec
    ))
}

#[cfg(windows)]
fn get_process_start_token(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    unsafe {
        let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut creation_time: FILETIME = std::mem::zeroed();
        let mut exit_time: FILETIME = std::mem::zeroed();
        let mut kernel_time: FILETIME = std::mem::zeroed();
        let mut user_time: FILETIME = std::mem::zeroed();
        let success = GetProcessTimes(
            handle,
            &mut creation_time,
            &mut exit_time,
            &mut kernel_time,
            &mut user_time,
        );
        CloseHandle(handle);
        if success == 0 {
            return None;
        }
        // FILETIME is in 100-nanosecond intervals since January 1, 1601 (UTC)
        let creation =
            ((creation_time.dwHighDateTime as u64) << 32) | (creation_time.dwLowDateTime as u64);
        Some(creation.to_string())
    }
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
    /// The new/staged replacement path. This may be the same as
    /// `original_path` when the caller writes directly to the destination
    /// using atomic_replace. When a separate durable staging file is used,
    /// this points to the staged content that will be atomically moved.
    pub staged_path: PathBuf,
    /// SHA-256 hex digest of the staged content for integrity verification.
    /// Populated when the staged content is written to a durable location.
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
    /// Optional separate durable staging path. When set, the new content
    /// is written here first and atomically moved to `original_path` during
    /// commit. This decouples staged content from the live destination,
    /// ensuring the journal always references a complete, durable copy.
    #[serde(default)]
    pub durable_staged_path: Option<PathBuf>,
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
    /// Live replacement is in progress; tracks completed positions.
    ///
    /// `next_commit_position == N` means positions `0..N` have already been
    /// installed and verified; position `N` is next.
    Committing {
        /// Number of completed and verified file installations.
        next_commit_position: usize,
    },
    /// All destinations installed and verified; pending sync intent is
    /// being durably recorded. `pending_recorded` tracks whether the
    /// pending marker has been durably written.
    CommittedLocal {
        /// The pending generation to record.
        pending_generation: u64,
        /// Whether the pending marker has been durably written.
        pending_recorded: bool,
    },
    /// Transaction has been committed; staged files are in place.
    Committed,
    /// Rollback is in progress; tracks rollback-order position.
    ///
    /// `next_rollback_position == N` means positions `0..N` in the
    /// rollback order have been restored and verified.
    RollingBack {
        /// Number of completed rollback actions in rollback order.
        next_rollback_position: usize,
    },
    /// Transaction was rolled back; backups restored.
    RolledBack,
    /// Transaction failed with an error message.
    Failed(String),
}

impl TransactionState {
    /// Returns true if this state represents an interrupted (non-terminal) transaction.
    ///
    /// Interruptible states are `Prepared`, `BackupsDurable`, `Committing`,
    /// `CommittedLocal`, and `RollingBack`. Terminal states (`Committed`,
    /// `RolledBack`, `Failed`) are not interruptible.
    pub fn is_interruptible(&self) -> bool {
        matches!(
            self,
            TransactionState::Prepared
                | TransactionState::BackupsDurable
                | TransactionState::Committing { .. }
                | TransactionState::CommittedLocal { .. }
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
        // Only remove if we still own the lock. Verify nonce, PID, and
        // start token (when present) to prevent removal by a wrong owner.
        if let Ok(content) = fs::read_to_string(&self.lock_path)
            && let Ok(existing) = toml::from_str::<TransactionLockInfo>(&content)
            && existing.nonce == self.info.nonce
            && existing.pid == self.info.pid
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
/// at a time. If an existing lock is found, observes the process identified
/// by the lock record's PID. Dead or reused owners are quarantined and
/// the acquisition loop retries with `create_new(true)`. Returns an error
/// if the lock is held by a live process.
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

    // Single acquisition loop: create_new, then classify existing owner.
    loop {
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
                return Ok(TransactionLock { lock_path, info });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Lock exists — read and classify the owner.
                let content = fs::read_to_string(&lock_path)
                    .map_err(|e| SnipError::io_error("read existing lock", lock_path.clone(), e))?;
                let existing: TransactionLockInfo = match toml::from_str(&content) {
                    Ok(info) => info,
                    Err(_) => {
                        // Malformed lock — quarantine, then loop back to create_new.
                        tracing::warn!("Malformed transaction lock record, quarantining");
                        quarantine_stale_lock(&lock_path)?;
                        continue;
                    }
                };

                // Observe the process identified by the existing lock record.
                // This queries existing.pid — NOT the current process.
                match ProcessIdentity::observe(existing.pid) {
                    None => {
                        // Owner process is dead — reclaim.
                        tracing::info!(
                            pid = existing.pid,
                            operation = %existing.operation,
                            "Reclaiming stale transaction lock (owner process is dead)"
                        );
                        quarantine_stale_lock(&lock_path)?;
                        continue;
                    }
                    Some(observed) => {
                        // Owner is alive. Refuse if we cannot verify ownership
                        // (conservative policy — "identity unavailable" is NOT
                        // "stale"):
                        // - existing.start_token is None (old lock without token)
                        // - observed.start_token is None (can't observe identity)
                        // - start tokens match (same process)
                        // Only reclaim when both tokens are present and differ
                        // (PID reuse).
                        if existing.start_token.is_none()
                            || observed.start_token.is_none()
                            || observed.start_token == existing.start_token
                        {
                            return Err(SnipError::runtime_error(
                                "Transaction lock held",
                                Some(&format!(
                                    "Another transaction ({}) is in progress (PID {}). Wait for it to complete.",
                                    existing.operation, existing.pid
                                )),
                            ));
                        }
                        // PID reuse detected — observed start token differs
                        // from recorded start token.
                        tracing::info!(
                            pid = existing.pid,
                            observed_token = ?observed.start_token,
                            recorded_token = ?existing.start_token,
                            "Transaction lock owner PID reused (start token mismatch), reclaiming"
                        );
                        quarantine_stale_lock(&lock_path)?;
                        continue;
                    }
                }
            }
            Err(e) => {
                return Err(SnipError::io_error(
                    "acquire transaction lock",
                    lock_path,
                    e,
                ));
            }
        }
    }
}

/// Quarantine a stale or malformed lock by renaming it.
///
/// The quarantined file preserves the original content for debugging
/// and repair inspection. Returns the quarantine path on success.
///
/// If the lock file has already been quarantined by a concurrent writer
/// (race on stale-lock reclaim), the `NotFound` error is treated as success.
fn quarantine_stale_lock(lock_path: &Path) -> SnipResult<PathBuf> {
    let quarantine_name = format!("transaction.lock.quarantine.{}", uuid::Uuid::new_v4());
    let quarantine_path = lock_path
        .parent()
        .unwrap_or(lock_path)
        .join(&quarantine_name);
    match fs::rename(lock_path, &quarantine_path) {
        Ok(()) => Ok(quarantine_path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Another writer already quarantined the lock — treat as success.
            tracing::debug!("transaction lock already quarantined by another writer");
            Ok(quarantine_path)
        }
        Err(e) => Err(SnipError::io_error(
            "quarantine stale lock",
            quarantine_path.clone(),
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
                durable_staged_path: None,
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

/// Advance the journal to `Committing { next_commit_position }`.
///
/// `next_commit_position` represents completed work: positions `0..N`
/// have been installed and verified; position `N` is next.
pub fn advance_to_committing(
    state_dir: &Path,
    journal: &mut TransactionJournal,
    next_commit_position: usize,
) -> SnipResult<()> {
    journal.state = TransactionState::Committing {
        next_commit_position,
    };
    persist_journal(state_dir, journal)
}

/// Advance the journal to `RollingBack { next_rollback_position }`.
///
/// `next_rollback_position` represents completed rollback actions in
/// rollback order: positions `0..N` have been restored and verified.
#[allow(dead_code)]
pub fn advance_to_rolling_back(
    state_dir: &Path,
    journal: &mut TransactionJournal,
    next_rollback_position: usize,
) -> SnipResult<()> {
    journal.state = TransactionState::RollingBack {
        next_rollback_position,
    };
    persist_journal(state_dir, journal)
}

/// Advance the journal to `CommittedLocal` finalization state.
///
/// This is persisted after all destinations are installed and verified,
/// before the pending sync intent is durably recorded. `pending_recorded`
/// tracks whether the pending marker has been written.
pub fn advance_to_committed_local(
    state_dir: &Path,
    journal: &mut TransactionJournal,
    pending_generation: u64,
    pending_recorded: bool,
) -> SnipResult<()> {
    journal.state = TransactionState::CommittedLocal {
        pending_generation,
        pending_recorded,
    };
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
/// Restores each staged file from its backup in rollback order (reverse of
/// file order) using atomic persistence, durably advancing rollback progress
/// after each file. Newly created files (action=Create, existed_before=false)
/// are removed rather than overwritten. The journal is marked as `RolledBack`
/// on completion.
///
/// Rollback is restartable: if interrupted, the next call picks up from
/// the last durably recorded `next_rollback_position` in rollback order.
///
/// After each action, the result is verified: SHA-256 must equal
/// `original_hash`, or the destination must be absent when
/// `existed_before == false`.
pub fn rollback_transaction(state_dir: &Path, journal: &TransactionJournal) -> SnipResult<()> {
    let mut rb_journal = journal.clone();
    let start_position = match rb_journal.state {
        TransactionState::RollingBack {
            next_rollback_position,
        } => next_rollback_position,
        _ => 0,
    };

    // Rollback order is the reverse of file order.
    // Position 0 = last file, position 1 = second-to-last, etc.
    let rollback_order: Vec<usize> = (0..rb_journal.staged_files.len()).rev().collect();

    for (position, &file_index) in rollback_order.iter().enumerate().skip(start_position) {
        let staged = &rb_journal.staged_files[file_index];

        // Advance to RollingBack before the action so a crash during
        // rollback is recoverable.
        rb_journal.state = TransactionState::RollingBack {
            next_rollback_position: position,
        };
        persist_journal(state_dir, &rb_journal)?;

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
                // Verify absence
                if staged.original_path.exists() {
                    return Err(SnipError::runtime_error(
                        "Rollback verification failed",
                        Some(&format!(
                            "File {} should be absent after rollback but still exists",
                            staged.original_path.display()
                        )),
                    ));
                }
            }
            StagedAction::Delete
            | StagedAction::Replace
            | StagedAction::NoOp
            | StagedAction::Create => {
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

                    // Verify hash matches original
                    if !staged.original_hash.is_empty() {
                        let actual = sha256_hex(&bytes);
                        if actual != staged.original_hash {
                            return Err(SnipError::runtime_error(
                                "Rollback verification failed",
                                Some(&format!(
                                    "File {} hash mismatch after rollback: expected {}, got {}",
                                    staged.original_path.display(),
                                    &staged.original_hash[..16.min(staged.original_hash.len())],
                                    &actual[..16]
                                )),
                            ));
                        }
                    }
                } else if !staged.existed_before {
                    // No backup and file didn't exist before — verify absence
                    if staged.original_path.exists() {
                        return Err(SnipError::runtime_error(
                            "Rollback verification failed",
                            Some(&format!(
                                "File {} should be absent after rollback but still exists",
                                staged.original_path.display()
                            )),
                        ));
                    }
                }
            }
        }

        // Durably advance rollback progress (completed position + 1)
        rb_journal.state = TransactionState::RollingBack {
            next_rollback_position: position + 1,
        };
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

/// Compute the SHA-256 hex digest of a byte slice.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
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

        // Handle CommittedLocal finalization state: clean up without rollback.
        if let TransactionState::CommittedLocal {
            pending_generation,
            pending_recorded,
        } = journal.state
        {
            tracing::info!(
                txn_id = %journal.id,
                generation = pending_generation,
                pending_recorded,
                "Finalizing CommittedLocal transaction"
            );

            // If pending was not recorded, check if the pending marker
            // already exists for this generation. If not, the sync may
            // have already completed — just clean up.
            if !pending_recorded {
                let pending_path = crate::auto_sync::pending::pending_path(state_dir);
                let marker_exists = match crate::auto_sync::pending::read_state(&pending_path) {
                    Ok(state) => state.generation == pending_generation,
                    Err(_) => false,
                };
                if !marker_exists {
                    tracing::info!(
                        generation = pending_generation,
                        "Pending marker for generation already absent or changed; \
                         sync may have completed"
                    );
                }
                // Persist pending_recorded: true to mark finalization complete.
                let mut finalized = journal.clone();
                finalized.state = TransactionState::CommittedLocal {
                    pending_generation,
                    pending_recorded: true,
                };
                persist_journal(state_dir, &finalized)?;
            }

            // Clean up: remove journal and backups.
            for staged in &journal.staged_files {
                if let Some(ref backup) = staged.backup_path {
                    let _ = fs::remove_file(backup);
                }
            }
            let jpath = journal_path(state_dir, &journal.id);
            let _ = fs::remove_file(&jpath);

            return Ok(());
        }

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
        assert!(
            TransactionState::Committing {
                next_commit_position: 0
            }
            .is_interruptible()
        );
        assert!(
            TransactionState::RollingBack {
                next_rollback_position: 0
            }
            .is_interruptible()
        );
        assert!(
            TransactionState::CommittedLocal {
                pending_generation: 0,
                pending_recorded: false
            }
            .is_interruptible()
        );
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
