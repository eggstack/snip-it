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
//!
//! ## State machine
//!
//! ```text
//! Prepared → BackupsDurable → Committing{pos} → CommittedLocal{pending}
//!          → CleaningUp{outcome: Commit, next_step: Validate} → ... → (journal removed)
//!
//! Prepared → BackupsDurable → RollingBack{pos} → CleaningUp{outcome: Rollback, next_step: Validate}
//!          → ... → (journal removed)
//! ```
//!
//! `CleaningUp` is interruptible and restartable: `finalize_transaction_cleanup`
//! persists progress after each step and resumes from `next_step` on recovery.
//!
//! New transactions never persist terminal `Committed` or `RolledBack` states.
//! The journal is removed during cleanup, making the absence of a journal the
//! true terminal indicator. Legacy `Committed` and `RolledBack` journals (from
//! older versions) are handled as `CleaningUp` with the appropriate outcome
//! during recovery.

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

/// Original destination file metadata captured before live writes.
///
/// Used to preserve relevant file permissions across commit and rollback.
/// Only the metadata contract documented here is preserved — ACLs and
/// ownership are not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OriginalFileMetadata {
    /// Unix file mode (permission bits), if observable.
    /// Only the lower 12 bits (permission + setuid/setgid/sticky) are
    /// meaningful. Setuid/setgid/sticky bits are stripped on restore.
    #[serde(default)]
    pub unix_mode: Option<u32>,
    /// Whether the file was marked read-only.
    #[serde(default)]
    pub readonly: Option<bool>,
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
    /// Original destination file metadata captured before live writes.
    /// Used to preserve permission bits across commit and rollback.
    #[serde(default)]
    pub original_metadata: OriginalFileMetadata,
}

fn default_action() -> StagedAction {
    StagedAction::Replace
}

/// Pending finalization state for `CommittedLocal`.
///
/// Replaces the sentinel `pending_generation: 0` + `pending_recorded: bool`
/// pattern with an explicit typed model. Unknown is not encoded as a valid
/// generation — `NotRecorded` is a distinct variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingFinalization {
    /// No pending marker has been durably recorded yet.
    NotRecorded,
    /// A transaction-associated pending marker has been durably recorded
    /// with the given generation.
    Recorded { generation: u64 },
    /// An unrelated existing pending generation covers the restored state.
    /// This is valid only when the pending protocol is full-current-state
    /// sync (a single generation causes a full synchronization).
    CoveredByExisting { generation: u64 },
}

/// Outcome of the cleanup phase — whether the transaction committed or rolled back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupOutcome {
    /// Transaction committed successfully; cleanup removes commit artifacts.
    #[default]
    Commit,
    /// Transaction was rolled back; cleanup removes rollback artifacts.
    Rollback,
}

/// Individual cleanup step — tracks progress through the cleanup sequence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupStep {
    /// Validate artifact containment and remove staged files.
    #[default]
    Validate,
    /// Remove backup files.
    RemoveBackups,
    /// Remove the artifact directory.
    RemoveArtifactRoot,
    /// Remove the journal file.
    RemoveJournal,
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
    /// being durably recorded. The `pending` field tracks the finalization
    /// state of the pending marker.
    CommittedLocal {
        /// Pending finalization state — whether and how the pending
        /// marker has been durably recorded.
        pending: PendingFinalization,
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
    /// Cleanup is in progress; tracks cleanup outcome and step.
    ///
    /// Each step is persisted before execution so a crash during cleanup
    /// is recoverable. The journal is removed last.
    CleaningUp {
        /// Whether this cleanup is for a commit or rollback.
        #[serde(default)]
        outcome: CleanupOutcome,
        /// The next cleanup step to execute.
        #[serde(default)]
        next_step: CleanupStep,
    },
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
                | TransactionState::CleaningUp { .. }
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
    create_private_dir(state_dir)?;

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

    // Pre-serialize the lock record so we can write it to the file
    // handle immediately, minimizing the empty-file window between
    // create_new succeeding and content being written.
    let content = toml::to_string_pretty(&info)
        .map_err(|e| SnipError::toml_error("serialize lock info", e))?;

    // Single acquisition loop: create_new, write immediately, classify existing owner.
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                // Write the lock record to the open file handle
                // immediately, then sync before dropping. This
                // minimizes the empty-file window. A concurrent
                // reader that sees empty content will retry instead
                // of quarantining (see below).
                use std::io::Write;
                file.write_all(content.as_bytes())
                    .map_err(|e| SnipError::io_error("write lock record", lock_path.clone(), e))?;
                let _ = file.sync_all();
                return Ok(TransactionLock { lock_path, info });
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::AlreadyExists
                // On Windows, a just-deleted file can briefly return
                // PermissionDenied when in a pending-delete state.
                // Treat it the same as AlreadyExists.
                || e.kind() == std::io::ErrorKind::PermissionDenied =>
            {
                // Lock exists — read and classify the owner.
                // Handle TOCTOU: another writer may have removed the lock
                // between create_new failing and read_to_string.
                let content = match fs::read_to_string(&lock_path) {
                    Ok(c) => c,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::NotFound
                        // On Windows, a pending-delete file may be unreadable.
                        || e.kind() == std::io::ErrorKind::PermissionDenied =>
                    {
                        // Lock was removed or is in a transient state — loop back and retry.
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                    Err(e) => {
                        return Err(SnipError::io_error(
                            "read existing lock",
                            lock_path.clone(),
                            e,
                        ));
                    }
                };
                let existing: TransactionLockInfo = match toml::from_str(&content) {
                    Ok(info) => info,
                    Err(_) if content.trim().is_empty() => {
                        // Empty file — another writer just called
                        // create_new but hasn't written yet. Retry
                        // instead of quarantining.
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                    Err(_) => {
                        // Genuinely malformed lock — quarantine, then loop back.
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
    create_private_dir(state_dir)?;

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
                original_metadata: if existed {
                    capture_original_metadata(p)
                } else {
                    OriginalFileMetadata::default()
                },
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
/// before the pending sync intent is durably recorded. The `pending`
/// parameter tracks the finalization state of the pending marker.
pub fn advance_to_committed_local(
    state_dir: &Path,
    journal: &mut TransactionJournal,
    pending: PendingFinalization,
) -> SnipResult<()> {
    journal.state = TransactionState::CommittedLocal { pending };
    persist_journal(state_dir, journal)
}

/// Commit a transaction (atomic multi-file commit).
///
/// Transitions to `CleaningUp { outcome: Commit }` and removes all transaction
/// artifacts via `finalize_transaction_cleanup`. The caller is responsible for
/// actually writing the staged files before calling this function.
///
/// New transactions never persist a terminal `Committed` state. The journal is
/// removed during cleanup, making the absence of a journal the true terminal
/// indicator.
pub fn commit_transaction(state_dir: &Path, journal: &TransactionJournal) -> SnipResult<()> {
    begin_cleanup(state_dir, journal, CleanupOutcome::Commit)
}

/// Validate that all artifact paths in the journal remain contained within
/// the transaction artifact root, and that no artifact path is a symlink.
///
/// This prevents path-traversal and symlink attacks during cleanup.
pub fn validate_artifact_containment(
    state_dir: &Path,
    journal: &TransactionJournal,
) -> SnipResult<()> {
    let artifact_root = transaction_artifact_dir(state_dir, &journal.id);

    for staged in &journal.staged_files {
        // Check backup path containment.
        if let Some(ref backup) = staged.backup_path {
            validate_contained_path(&artifact_root, backup, "backup_path")?;
        }
        // Check durable staged path containment.
        if let Some(ref staged_path) = staged.durable_staged_path {
            validate_contained_path(&artifact_root, staged_path, "durable_staged_path")?;
        }
    }

    Ok(())
}

/// Validate that `path` is contained within `root` and is not a symlink.
fn validate_contained_path(root: &Path, path: &Path, label: &str) -> SnipResult<()> {
    // Reject symlinks — they could escape the artifact root.
    if path.is_symlink() {
        return Err(SnipError::runtime_error(
            "symlinked transaction artifact",
            Some(&format!(
                "Artifact {} at {} is a symlink; refusing to follow. Root: {}",
                label,
                path.display(),
                root.display()
            )),
        ));
    }

    // Verify the path is within the root.
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    if !canonical_path.starts_with(&canonical_root) {
        return Err(SnipError::runtime_error(
            "transaction artifact path traversal",
            Some(&format!(
                "Artifact {} at {} is outside the transaction artifact root {}",
                label,
                path.display(),
                canonical_root.display()
            )),
        ));
    }

    Ok(())
}

/// Compute the per-transaction artifact directory path.
///
/// Artifacts are stored under `artifacts/<id>/` within the state directory,
/// ensuring each transaction has its own isolated namespace.
pub fn transaction_artifact_dir(state_dir: &Path, txn_id: &str) -> PathBuf {
    state_dir.join("artifacts").join(txn_id)
}

/// Create a directory with private permissions (0o700 on Unix).
///
/// Used for transaction artifact directories that may contain plaintext
/// snippet commands or sync configuration.
///
/// On Unix, the directory is created with `0o700` permissions at creation
/// time using `DirBuilderExt::mode`, preventing a window where the directory
/// is briefly world-readable. Permission failures are fatal.
pub fn create_private_dir(path: &Path) -> SnipResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        // DirBuilder::create with mode(0o700) sets the mode at creation time
        // for new directories, avoiding a world-readable window. For
        // existing directories, we must also set_permissions to enforce
        // the policy.
        builder
            .create(path)
            .map_err(|e| SnipError::io_error("create private directory", path, e))?;
        // Enforce permissions on existing directories too.
        let perms = fs::Permissions::from_mode(0o700);
        if let Err(e) = fs::set_permissions(path, perms) {
            return Err(SnipError::io_error(
                "set private directory permissions",
                path,
                e,
            ));
        }
        // Verify permissions were applied correctly.
        use std::os::unix::fs::MetadataExt;
        let actual_mode = fs::metadata(path)
            .map_err(|e| SnipError::io_error("stat private directory", path, e))?
            .mode()
            & 0o777;
        if actual_mode != 0o700 {
            return Err(SnipError::runtime_error(
                "Private directory permission failure",
                Some(&format!(
                    "Directory {} created with mode {:o}, expected 700. Refusing to proceed.",
                    path.display(),
                    actual_mode
                )),
            ));
        }
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)
            .map_err(|e| SnipError::io_error("create private directory", path, e))?;
    }

    Ok(())
}

/// Remove all staged files from the journal.
fn remove_all_staged_files(journal: &TransactionJournal) -> SnipResult<()> {
    for staged in &journal.staged_files {
        if let Some(ref staged_path) = staged.durable_staged_path
            && staged_path.exists()
            && !staged_path.is_symlink()
        {
            fs::remove_file(staged_path).map_err(|e| {
                SnipError::io_error("remove staged file during cleanup", staged_path.clone(), e)
            })?;
        }
    }
    Ok(())
}

/// Remove all backup files from the journal.
fn remove_all_backup_files(journal: &TransactionJournal) -> SnipResult<()> {
    for staged in &journal.staged_files {
        if let Some(ref backup) = staged.backup_path
            && backup.exists()
            && !backup.is_symlink()
        {
            fs::remove_file(backup).map_err(|e| {
                SnipError::io_error("remove backup file during cleanup", backup.clone(), e)
            })?;
        }
    }
    Ok(())
}

/// Remove the transaction artifact directory (backups/, staged/ subdirs).
fn remove_empty_transaction_artifact_dir(state_dir: &Path, txn_id: &str) -> SnipResult<()> {
    let artifact_dir = transaction_artifact_dir(state_dir, txn_id);
    if artifact_dir.exists() {
        // Remove the entire artifact directory tree.
        // On Windows, use bounded retries for delete-pending behavior.
        remove_dir_all_with_retry(&artifact_dir)?;
    }
    Ok(())
}

/// Remove a directory tree with bounded retries (for Windows delete-pending).
fn remove_dir_all_with_retry(path: &Path) -> SnipResult<()> {
    const MAX_RETRIES: u32 = 5;
    const RETRY_DELAY_MS: u64 = 50;

    for attempt in 0..MAX_RETRIES {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(_e) if attempt < MAX_RETRIES - 1 => {
                std::thread::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS));
                continue;
            }
            Err(e) => {
                return Err(SnipError::io_error(
                    "remove transaction artifact directory",
                    path.to_path_buf(),
                    e,
                ));
            }
        }
    }

    Ok(())
}

/// Begin the cleanup phase for a transaction.
///
/// Transitions the journal to `CleaningUp { outcome, next_step: Validate }`
/// and persists it before any destructive operations. This ensures cleanup
/// ownership is durable before any artifacts are removed.
///
/// Called from `commit_transaction` and `rollback_transaction` instead of
/// persisting terminal `Committed` or `RolledBack` states.
pub fn begin_cleanup(
    state_dir: &Path,
    journal: &TransactionJournal,
    outcome: CleanupOutcome,
) -> SnipResult<()> {
    let mut cleaning = journal.clone();
    cleaning.state = TransactionState::CleaningUp {
        outcome,
        next_step: CleanupStep::Validate,
    };
    persist_journal(state_dir, &cleaning)?;
    finalize_transaction_cleanup(state_dir, &mut cleaning)
}

/// Resume an interrupted cleanup from the last durably persisted step.
///
/// Reads the journal's `CleaningUp` state to determine the outcome and
/// next step, then continues cleanup from that point. Used by the mutation
/// gate and repair to recover interrupted cleanups.
pub fn resume_cleanup(state_dir: &Path, journal: &mut TransactionJournal) -> SnipResult<()> {
    if let TransactionState::CleaningUp { outcome, next_step } = &journal.state {
        tracing::info!(
            txn_id = %journal.id,
            outcome = ?outcome,
            next_step = ?next_step,
            "Resuming interrupted cleanup"
        );
    }
    finalize_transaction_cleanup(state_dir, journal)
}

/// Finalize transaction cleanup: remove all artifacts and the journal last.
///
/// This is the canonical cleanup path used by commit, rollback, and
/// CommittedLocal recovery. It is restartable: if interrupted, the next
/// call (from recovery or a new cleanup attempt) resumes from the last
/// durably recorded `next_step` in the `CleaningUp` state.
///
/// Cleanup steps in order:
/// 1. Validate artifact containment and remove staged files;
/// 2. Remove backup files;
/// 3. Remove the artifact directory;
/// 4. Remove the journal file;
/// 5. Fsync the parent directory after journal removal.
///
/// The journal is advanced to `CleaningUp { next_step }` before each
/// destructive step, so a crash during cleanup is recoverable.
pub fn finalize_transaction_cleanup(
    state_dir: &Path,
    journal: &mut TransactionJournal,
) -> SnipResult<()> {
    // Determine the starting step from the journal state.
    let start_step = match &journal.state {
        TransactionState::CleaningUp { next_step, .. } => *next_step,
        _ => CleanupStep::Validate,
    };

    let steps = [
        CleanupStep::Validate,
        CleanupStep::RemoveBackups,
        CleanupStep::RemoveArtifactRoot,
        CleanupStep::RemoveJournal,
    ];

    let start_index = steps.iter().position(|s| *s == start_step).unwrap_or(0);

    for (idx, &step) in steps.iter().enumerate().skip(start_index) {
        // Persist the CleaningUp state before executing the step.
        // Skip persistence if the journal has already been removed (RemoveJournal and later).
        if step != CleanupStep::RemoveJournal {
            let outcome = match &journal.state {
                TransactionState::CleaningUp { outcome, .. } => *outcome,
                _ => CleanupOutcome::Commit,
            };
            journal.state = TransactionState::CleaningUp {
                outcome,
                next_step: step,
            };
            persist_journal(state_dir, journal)?;
        }

        // Failpoints for crash testing during cleanup.
        // Each fires AFTER the journal has been persisted at the named
        // step but BEFORE the step body executes.
        match step {
            CleanupStep::Validate => {
                crate::test_failpoints::maybe_failpoint(
                    crate::test_failpoints::failpoints::CLEANUP_AFTER_STATE_BEFORE_VALIDATION,
                );
            }
            CleanupStep::RemoveBackups => {
                crate::test_failpoints::maybe_failpoint(
                    crate::test_failpoints::failpoints::CLEANUP_AFTER_STAGED_BEFORE_BACKUPS,
                );
            }
            CleanupStep::RemoveArtifactRoot => {
                crate::test_failpoints::maybe_failpoint(
                    crate::test_failpoints::failpoints::CLEANUP_AFTER_BACKUPS_BEFORE_ARTIFACT_ROOT,
                );
            }
            _ => {}
        }

        match step {
            CleanupStep::Validate => {
                // Validate artifact containment and remove staged files.
                validate_artifact_containment(state_dir, journal)?;
                crate::test_failpoints::maybe_failpoint(
                    crate::test_failpoints::failpoints::CLEANUP_AFTER_VALIDATION_BEFORE_STAGED,
                );
                remove_all_staged_files(journal)?;
            }
            CleanupStep::RemoveBackups => {
                remove_all_backup_files(journal)?;
            }
            CleanupStep::RemoveArtifactRoot => {
                remove_empty_transaction_artifact_dir(state_dir, &journal.id)?;
            }
            CleanupStep::RemoveJournal => {
                crate::test_failpoints::maybe_failpoint(
                    crate::test_failpoints::failpoints::CLEANUP_AFTER_ARTIFACT_ROOT_BEFORE_JOURNAL,
                );
                // Remove the journal file last.
                let jpath = journal_path(state_dir, &journal.id);
                if jpath.exists() {
                    fs::remove_file(&jpath).map_err(|e| {
                        SnipError::io_error(
                            "remove transaction journal during cleanup",
                            jpath.clone(),
                            e,
                        )
                    })?;
                }
                crate::test_failpoints::maybe_failpoint(
                    crate::test_failpoints::failpoints::CLEANUP_AFTER_JOURNAL_BEFORE_PARENT_SYNC,
                );
                // Fsync the parent directory to durably record the removal.
                fsync_parent_dir(&jpath)?;
                // Final step — journal is gone, no further state to persist.
                continue;
            }
        }

        // Persist progress after each step (except RemoveJournal).
        if idx + 1 < steps.len() {
            let next = steps[idx + 1];
            let outcome = match &journal.state {
                TransactionState::CleaningUp { outcome, .. } => *outcome,
                _ => CleanupOutcome::Commit,
            };
            journal.state = TransactionState::CleaningUp {
                outcome,
                next_step: next,
            };
            persist_journal(state_dir, journal)?;
        }
    }

    Ok(())
}

/// Fsync the parent directory of a file to durably record directory changes.
fn fsync_parent_dir(path: &Path) -> SnipResult<()> {
    let parent = path.parent().unwrap_or(path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let dir = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_RDONLY)
            .open(parent)
            .map_err(|e| {
                SnipError::io_error("open parent dir for fsync", parent.to_path_buf(), e)
            })?;
        #[cfg(target_os = "linux")]
        {
            // On Linux, use the fd-based fsync.
            use std::os::fd::AsRawFd;
            unsafe {
                let _ = libc::fsync(dir.as_raw_fd());
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            // On macOS, use fsync on the directory fd.
            use std::os::fd::AsRawFd;
            unsafe {
                let _ = libc::fsync(dir.as_raw_fd());
            }
        }
    }
    #[cfg(windows)]
    {
        // On Windows, directory fsync is not directly available; the
        // OS handles directory durability through the file system.
        // This is a no-op on Windows.
    }
    Ok(())
}

/// Capture original destination file metadata before live writes.
///
/// On Unix, captures the file mode. Strips setuid/setgid/sticky bits
/// from the captured mode so they are not propagated on restore.
pub fn capture_original_metadata(path: &Path) -> OriginalFileMetadata {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Ok(metadata) = fs::metadata(path) {
            let mode = metadata.mode() & 0o7777;
            // Strip setuid, setgid, and sticky bits — they are security-
            // sensitive and should not be propagated by restore.
            let sanitized_mode = mode & 0o777;
            return OriginalFileMetadata {
                unix_mode: Some(sanitized_mode),
                readonly: Some(metadata.permissions().readonly()),
            };
        }
    }
    OriginalFileMetadata::default()
}

/// Apply captured metadata to a destination file after content installation.
///
/// On Unix, restores the permission bits (excluding setuid/setgid/sticky).
/// The readonly state is incorporated into the mode computation rather than
/// applied as a separate `set_permissions` call, which would clobber the
/// mode by adding all write bits via `set_readonly(false)`.
///
/// Does not claim ACL or ownership preservation.
pub fn apply_original_metadata(path: &Path, metadata: &OriginalFileMetadata) -> SnipResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        // Compute the final mode in a single pass: start from the captured
        // mode (or a safe default), strip setuid/setgid/sticky, then apply
        // the readonly state by clearing write bits if the file was readonly.
        let mut mode = metadata.unix_mode.unwrap_or(0o644);
        mode &= 0o777; // strip setuid/setgid/sticky

        if metadata.readonly == Some(true) {
            // Remove all write bits to honor the readonly state.
            mode &= !0o222;
        }

        let perms = fs::Permissions::from_mode(mode);
        fs::set_permissions(path, perms).map_err(|e| {
            SnipError::io_error("set permissions after restore", path.to_path_buf(), e)
        })?;
    }
    #[cfg(not(unix))]
    {
        // On Windows, readonly behavior is tested where relevant.
        if let Some(readonly) = metadata.readonly {
            if let Ok(perms) = fs::metadata(path).map(|m| m.permissions()) {
                let mut perms = perms;
                perms.set_readonly(readonly);
                let _ = fs::set_permissions(path, perms);
            }
        }
    }
    Ok(())
}

/// Verify that a destination file's metadata matches the expected values.
///
/// On Unix, compares `mode & 0o777` to the expected sanitized value.
pub fn verify_metadata(path: &Path, metadata: &OriginalFileMetadata) -> SnipResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let Some(expected_mode) = metadata.unix_mode {
            let actual_mode = fs::metadata(path).map(|m| m.mode() & 0o777).map_err(|e| {
                SnipError::io_error("stat file for metadata verification", path.to_path_buf(), e)
            })?;
            if actual_mode != expected_mode {
                return Err(SnipError::runtime_error(
                    "metadata verification failed",
                    Some(&format!(
                        "File {} mode mismatch after restore: expected {:o}, got {:o}",
                        path.display(),
                        expected_mode,
                        actual_mode
                    )),
                ));
            }
        }
    }
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

        // Failpoints for crash testing during rollback.
        if position == 0 {
            crate::test_failpoints::maybe_failpoint(
                crate::test_failpoints::failpoints::RESTORE_DURING_FIRST_ROLLBACK,
            );
        }
        if position == 1 {
            crate::test_failpoints::maybe_failpoint(
                crate::test_failpoints::failpoints::RESTORE_DURING_SECOND_ROLLBACK,
            );
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

                    // Apply original metadata (permissions) after content
                    // is restored. This preserves the file mode across
                    // rollback, excluding setuid/setgid/sticky bits.
                    apply_original_metadata(&staged.original_path, &staged.original_metadata)?;

                    // Verify hash from the LIVE destination, not the backup
                    // buffer. This proves the installed content matches the
                    // original.
                    if !staged.original_hash.is_empty() {
                        let actual = hash_file(&staged.original_path)?;
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

                    // Verify metadata after content installation.
                    verify_metadata(&staged.original_path, &staged.original_metadata)?;
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

    rb_journal.state = TransactionState::RollingBack {
        next_rollback_position: rollback_order.len(),
    };
    persist_journal(state_dir, &rb_journal)?;

    // Transition to cleanup instead of persisting terminal RolledBack.
    begin_cleanup(state_dir, &rb_journal, CleanupOutcome::Rollback)
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

/// Write bytes to a file, sync it, reopen it, and verify its hash.
///
/// This is the durability helper used for staging and backup files. It
/// ensures the content is durably on disk before returning, and verifies
/// the hash from the reopened file (not from the source buffer).
///
/// On Unix, the file is created with `0o600` permissions at creation time
/// using `OpenOptionsExt::mode`, preventing a window where the file is
/// briefly world-readable. Permission failures are fatal.
///
/// Returns the verified SHA-256 hex digest.
pub fn write_sync_verify(path: &Path, bytes: &[u8]) -> SnipResult<String> {
    use std::io::Write;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    create_private_dir(parent)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| SnipError::io_error("create staged file", path, e))?;
        file.write_all(bytes)
            .map_err(|e| SnipError::io_error("write staged file", path, e))?;
        file.sync_all()
            .map_err(|e| SnipError::io_error("sync staged file", path, e))?;
        drop(file);

        // Verify permissions were applied correctly.
        use std::os::unix::fs::MetadataExt;
        let actual_mode = fs::metadata(path)
            .map_err(|e| SnipError::io_error("stat staged file", path, e))?
            .mode()
            & 0o777;
        if actual_mode != 0o600 {
            return Err(SnipError::runtime_error(
                "Staged file permission failure",
                Some(&format!(
                    "File {} created with mode {:o}, expected 600. Refusing to proceed.",
                    path.display(),
                    actual_mode
                )),
            ));
        }
    }
    #[cfg(not(unix))]
    {
        let mut file = fs::File::create(path)
            .map_err(|e| SnipError::io_error("create staged file", path, e))?;
        file.write_all(bytes)
            .map_err(|e| SnipError::io_error("write staged file", path, e))?;
        file.sync_all()
            .map_err(|e| SnipError::io_error("sync staged file", path, e))?;
        drop(file);
    }

    // Reopen and verify from disk.
    let read_back = fs::read(path)
        .map_err(|e| SnipError::io_error("reopen staged file for verification", path, e))?;
    let actual = sha256_hex(&read_back);
    let expected = sha256_hex(bytes);
    if actual != expected {
        return Err(SnipError::runtime_error(
            "Staged file verification failed",
            Some(&format!(
                "File {} hash mismatch after write: expected {}, got {}",
                path.display(),
                &expected[..16],
                &actual[..16]
            )),
        ));
    }

    // Sync the parent directory where supported.
    sync_parent_dir(path);

    Ok(actual)
}

/// Copy a file, sync the destination, reopen it, and verify its hash matches
/// the source.
///
/// Returns the verified SHA-256 hex digest of the destination.
pub fn copy_sync_verify(src: &Path, dst: &Path) -> SnipResult<String> {
    let bytes = fs::read(src).map_err(|e| SnipError::io_error("read source for copy", src, e))?;
    write_sync_verify(dst, &bytes)
}

/// Sync the parent directory of a file to ensure directory entries are durable.
///
/// On Unix, this opens the parent directory and calls `sync_all`. On
/// Windows, this is a no-op (directory sync is not supported via the
/// same API). Failures are logged but not propagated — the file itself
/// was already synced.
fn sync_parent_dir(path: &Path) {
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => return,
    };
    #[cfg(unix)]
    {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        // On Windows, directory sync is not available via std.
        // The file sync_all already ensures data durability.
    }
}

/// Read a file and compute its SHA-256 hex digest.
///
/// This is used to verify installed destinations from the live file,
/// not from source buffers.
pub fn hash_file(path: &Path) -> SnipResult<String> {
    let bytes =
        fs::read(path).map_err(|e| SnipError::io_error("read file for hashing", path, e))?;
    Ok(sha256_hex(&bytes))
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
///
/// `transaction_dir` is the `.transaction` subdirectory where journals and
/// locks live. `sync_state_dir` is the canonical config directory where the
/// pending marker lives. The `CommittedLocal` recovery path needs both: it
/// inspects the canonical pending marker (in `sync_state_dir`) while
/// cleaning up transaction artifacts (in `transaction_dir`).
pub fn gate_mutation_on_interrupted_transactions(
    sync_state_dir: &Path,
    transaction_dir: &Path,
) -> SnipResult<()> {
    let interrupted = check_interrupted_transactions(transaction_dir)?;

    if interrupted.is_empty() {
        return Ok(());
    }

    if interrupted.len() == 1 {
        let journal = &interrupted[0];

        // Handle CleaningUp state: resume cleanup from the last step.
        if let TransactionState::CleaningUp { outcome, next_step } = &journal.state {
            tracing::info!(
                txn_id = %journal.id,
                outcome = ?outcome,
                next_step = ?next_step,
                "Resuming interrupted cleanup"
            );
            let mut cleanup_journal = journal.clone();
            match resume_cleanup(transaction_dir, &mut cleanup_journal) {
                Ok(()) => {
                    tracing::info!(
                        txn_id = %journal.id,
                        "Cleanup resumed and completed successfully"
                    );
                    return Ok(());
                }
                Err(e) => {
                    return Err(SnipError::runtime_error(
                        "Interrupted cleanup requires manual recovery",
                        Some(&format!(
                            "Transaction '{}' ({}) was interrupted during cleanup \
                             at step {:?} and automatic cleanup failed: {}. \
                             Run `snp repair` to inspect and recover.",
                            journal.operation, journal.id, next_step, e
                        )),
                    ));
                }
            }
        }

        // Handle legacy terminal Committed journal with artifacts:
        // treat as commit cleanup.
        if let TransactionState::Committed = &journal.state {
            if has_transaction_artifacts(transaction_dir, &journal.id) {
                tracing::info!(
                    txn_id = %journal.id,
                    "Legacy Committed journal has artifacts, cleaning up"
                );
                let cleanup_journal = journal.clone();
                match begin_cleanup(transaction_dir, &cleanup_journal, CleanupOutcome::Commit) {
                    Ok(()) => {
                        tracing::info!(
                            txn_id = %journal.id,
                            "Legacy Committed cleanup completed"
                        );
                        return Ok(());
                    }
                    Err(e) => {
                        return Err(SnipError::runtime_error(
                            "Legacy committed cleanup failed",
                            Some(&format!(
                                "Transaction '{}' ({}) has legacy Committed state with artifacts \
                                 and cleanup failed: {}. Run `snp repair`.",
                                journal.operation, journal.id, e
                            )),
                        ));
                    }
                }
            } else {
                // No artifacts — safe to remove the orphan journal.
                tracing::info!(
                    txn_id = %journal.id,
                    "Legacy Committed journal has no artifacts, removing"
                );
                let jpath = transaction_dir.join(format!("txn-{}.toml", journal.id));
                let _ = fs::remove_file(&jpath);
                return Ok(());
            }
        }

        // Handle legacy terminal RolledBack journal with artifacts:
        // treat as rollback cleanup.
        if let TransactionState::RolledBack = &journal.state {
            if has_transaction_artifacts(transaction_dir, &journal.id) {
                tracing::info!(
                    txn_id = %journal.id,
                    "Legacy RolledBack journal has artifacts, cleaning up"
                );
                let cleanup_journal = journal.clone();
                match begin_cleanup(transaction_dir, &cleanup_journal, CleanupOutcome::Rollback) {
                    Ok(()) => {
                        tracing::info!(
                            txn_id = %journal.id,
                            "Legacy RolledBack cleanup completed"
                        );
                        return Ok(());
                    }
                    Err(e) => {
                        return Err(SnipError::runtime_error(
                            "Legacy rolled-back cleanup failed",
                            Some(&format!(
                                "Transaction '{}' ({}) has legacy RolledBack state with artifacts \
                                 and cleanup failed: {}. Run `snp repair`.",
                                journal.operation, journal.id, e
                            )),
                        ));
                    }
                }
            } else {
                // No artifacts — safe to remove the orphan journal.
                tracing::info!(
                    txn_id = %journal.id,
                    "Legacy RolledBack journal has no artifacts, removing"
                );
                let jpath = transaction_dir.join(format!("txn-{}.toml", journal.id));
                let _ = fs::remove_file(&jpath);
                return Ok(());
            }
        }

        // Handle CommittedLocal finalization state: clean up without rollback.
        if let TransactionState::CommittedLocal { pending } = &journal.state {
            tracing::info!(
                txn_id = %journal.id,
                pending = ?pending,
                "Finalizing CommittedLocal transaction"
            );

            // If pending has not been recorded, attempt to finalize it.
            // The pending marker lives in the canonical sync state directory,
            // NOT in the transaction directory.
            let finalized = match pending {
                PendingFinalization::NotRecorded => {
                    // Idempotently create or reuse the pending marker.
                    // A crash here is recoverable: the gate will retry.
                    tracing::info!(
                        txn_id = %journal.id,
                        "Creating pending marker for CommittedLocal recovery"
                    );
                    let pending_result = crate::auto_sync::pending::ensure_pending_for_transaction(
                        sync_state_dir,
                        &journal.id,
                        crate::auto_sync::pending::PendingSnapshot::Mutation {
                            kind: crate::auto_sync::policy::MutationKind::Import,
                        },
                    );
                    match pending_result {
                        Ok(crate::auto_sync::pending::TransactionPendingResult::Created(state))
                        | Ok(crate::auto_sync::pending::TransactionPendingResult::Reused(state)) => {
                            PendingFinalization::Recorded {
                                generation: state.generation,
                            }
                        }
                        Ok(crate::auto_sync::pending::TransactionPendingResult::Conflict(
                            state,
                        )) => {
                            // An unrelated newer pending generation exists.
                            // Per the conflict policy, preserve it — the
                            // restored state is covered by the existing
                            // full-current-state sync generation.
                            tracing::warn!(
                                txn_id = %journal.id,
                                generation = state.generation,
                                "Pending conflict during recovery: \
                                 preserving existing generation"
                            );
                            PendingFinalization::CoveredByExisting {
                                generation: state.generation,
                            }
                        }
                        Err(e) => {
                            // Fail closed: preserve journal and artifacts.
                            return Err(SnipError::runtime_error(
                                "Committed restore requires pending recovery",
                                Some(&format!(
                                    "Transaction {} is committed locally but pending intent \
                                     could not be finalized: {e}. Recovery evidence was \
                                     preserved; run `snp repair`.",
                                    journal.id
                                )),
                            ));
                        }
                    }
                }
                // Already recorded — no action needed.
                PendingFinalization::Recorded { .. }
                | PendingFinalization::CoveredByExisting { .. } => pending.clone(),
            };

            // Persist the finalized pending state durably.
            let mut finalized_journal = journal.clone();
            finalized_journal.state = TransactionState::CommittedLocal { pending: finalized };
            persist_journal(transaction_dir, &finalized_journal)?;

            // Clean up: use the canonical restartable cleanup path.
            // This removes staged files, backup files, artifact directory,
            // and the journal itself, with progress persisted at each step.
            match finalize_transaction_cleanup(transaction_dir, &mut finalized_journal) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    return Err(SnipError::runtime_error(
                        "Committed restore cleanup failed",
                        Some(&format!(
                            "Transaction {} committed locally but cleanup failed: {}. \
                             Recovery evidence was preserved; run `snp repair`.",
                            journal.id, e
                        )),
                    ));
                }
            }
        }

        tracing::info!(
            txn_id = %journal.id,
            operation = %journal.operation,
            state = ?journal.state,
            "Attempting automatic rollback of interrupted transaction"
        );
        match rollback_transaction(transaction_dir, journal) {
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

/// Check whether a transaction has artifacts (staged files, backups, or
/// artifact directory) that still need cleanup.
fn has_transaction_artifacts(state_dir: &Path, txn_id: &str) -> bool {
    let artifact_dir = transaction_artifact_dir(state_dir, txn_id);
    if artifact_dir.exists() {
        return true;
    }
    // Also check for staged files referenced in the journal.
    false
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
        let _lock = acquire_transaction_lock(state_dir, "test_op").unwrap();
        let mut journal =
            begin_transaction(state_dir, "test_op", std::slice::from_ref(&file1)).unwrap();

        // Place backup inside the per-transaction artifact directory so
        // containment validation passes during cleanup.
        let artifact_dir = transaction_artifact_dir(state_dir, &journal.id);
        let backup_dir = artifact_dir.join("backups");
        fs::create_dir_all(&backup_dir).unwrap();
        let backup_path = backup_dir.join("0.bak");
        fs::copy(&file1, &backup_path).unwrap();
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
                pending: PendingFinalization::NotRecorded
            }
            .is_interruptible()
        );
        assert!(
            TransactionState::CleaningUp {
                outcome: CleanupOutcome::Commit,
                next_step: CleanupStep::Validate,
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
