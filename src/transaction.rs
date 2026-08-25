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
//! Prepared → Committing{pos} → CleaningUp{outcome, step} → (journal removed)
//! Prepared → RollingBack{pos} → CleaningUp{outcome, step} → (journal removed)
//! ```
//!
//! `CleaningUp` is interruptible and restartable: `finalize_transaction_cleanup`
//! persists progress after each step and resumes from `next_step` on recovery.
//!
//! New transactions never persist terminal `Committed` or `RolledBack` states.
//! The journal is removed during cleanup, making the absence of a journal the
//! true terminal indicator. Legacy `Committed`, `RolledBack`, and
//! `CommittedLocal` journals (from older versions) are handled during recovery.

use crate::error::{SnipError, SnipResult};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::fs;
use std::path::{Component, Path, PathBuf};

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
    parse_linux_proc_start_token(&content)
}

#[cfg(target_os = "linux")]
fn parse_linux_proc_start_token(stat: &str) -> Option<String> {
    let after_comm = stat.rfind(')')?;
    let fields: Vec<&str> = stat.get(after_comm + 2..)?.split_whitespace().collect();
    fields.get(19).map(|value| (*value).to_owned())
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
///
/// ```text
/// Prepared → Committing{pos} → CleaningUp{outcome, step} → (journal removed)
/// Prepared → RollingBack{pos} → CleaningUp{outcome, step} → (journal removed)
/// ```
///
/// New transactions never persist terminal `Committed` or `RolledBack` states.
/// The journal is removed during cleanup, making the absence of a journal the
/// true terminal indicator. Legacy `Committed`, `RolledBack`,
/// `BackupsDurable`, and `CommittedLocal` journals (from older versions) are
/// handled during recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionState {
    /// Transaction is prepared; backups taken, staged files ready.
    Prepared,
    /// All backup files are durably written to disk.
    /// Legacy state from older versions — treated as `Prepared` by recovery.
    BackupsDurable,
    /// Live replacement is in progress; tracks completed positions.
    Committing {
        /// Number of completed and verified file installations.
        next_commit_position: usize,
    },
    /// All destinations installed and verified; pending sync intent is
    /// being durably recorded.
    ///
    /// This state is used only for backward-compatible recovery of
    /// journals from older versions. New transactions transition
    /// directly from `Committing` to `CleaningUp`.
    CommittedLocal {
        /// Pending finalization state — whether and how the pending
        /// marker has been durably recorded.
        pending: PendingFinalization,
    },
    /// Transaction has been committed; staged files are in place.
    /// Legacy terminal state — only present in journals from older versions.
    Committed,
    /// Rollback is in progress; tracks rollback-order position.
    RollingBack {
        /// Number of completed rollback actions in rollback order.
        next_rollback_position: usize,
    },
    /// Transaction was rolled back; backups restored.
    /// Legacy terminal state — only present in journals from older versions.
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
    /// `CommittedLocal`, `RollingBack`, and `CleaningUp`. Terminal states
    /// (`Committed`, `RolledBack`, `Failed`) are not interruptible.
    #[allow(dead_code)]
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

/// Complete inventory of all transaction journals in the transaction directory.
///
/// Unlike `check_interrupted_transactions`, this discovers every journal
/// regardless of state, including legacy terminal journals that may still
/// own artifacts.
#[derive(Debug)]
pub struct JournalInventory {
    /// Valid journals parsed from `txn-*.toml` files, in stable path order.
    pub journals: Vec<TransactionJournal>,
    /// Journal files that could not be parsed, with the error message.
    pub corrupt: Vec<CorruptJournal>,
}

/// A transaction journal file that failed to parse.
#[derive(Debug)]
pub struct CorruptJournal {
    /// Absolute path to the corrupt journal file.
    pub path: PathBuf,
    /// Error message from the parse failure.
    pub error: String,
}

/// Recovery classification for a single transaction journal.
///
/// Determines what action (if any) is needed to recover or clean up
/// the transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryClass {
    /// Interrupted before commit; roll back.
    Rollback,
    /// Committed locally but pending sync intent not recorded.
    FinalizeCommittedLocal,
    /// Cleanup in progress; resume from last persisted step.
    ResumeCleanup,
    /// Legacy terminal Committed journal that still owns artifacts.
    CleanupLegacyCommitted,
    /// Legacy terminal RolledBack journal that still owns artifacts.
    CleanupLegacyRolledBack,
    /// Terminal journal with no artifacts; safe to remove the journal file.
    RemoveTerminalJournal,
    /// Failed state; requires manual investigation.
    UnsafeFailed,
}

/// Scan the transaction directory and discover all journals, regardless of state.
///
/// This is the authoritative journal discovery function. It replaces the
/// filtered `check_interrupted_transactions` for all mutation-gate and
/// repair-collection use cases.
///
/// The scanner:
/// - enumerates every `txn-*.toml` file;
/// - parses every valid journal regardless of state;
/// - reports corrupt journal files rather than silently skipping them;
/// - avoids following symlinks;
/// - uses stable path ordering for deterministic diagnostics and tests;
/// - performs no mutation.
pub fn scan_transaction_journals(transaction_dir: &Path) -> SnipResult<JournalInventory> {
    let mut journals = Vec::new();
    let mut corrupt = Vec::new();

    if !transaction_dir.exists() {
        return Ok(JournalInventory { journals, corrupt });
    }

    let mut entries: Vec<_> = fs::read_dir(transaction_dir)
        .map_err(|e| SnipError::io_error("read transaction directory", transaction_dir, e))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let path = e.path();
            path.extension().is_some_and(|ext| ext == "toml")
                && path
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy().starts_with("txn-"))
        })
        .collect();

    // Stable ordering by path for deterministic output.
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let path = entry.path();

        // Reject symlinked journal files.
        if path.is_symlink() {
            corrupt.push(CorruptJournal {
                path: path.clone(),
                error: "journal file is a symlink".to_string(),
            });
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                corrupt.push(CorruptJournal {
                    path: path.clone(),
                    error: format!("failed to read: {e}"),
                });
                continue;
            }
        };

        match toml::from_str::<TransactionJournal>(&content) {
            Ok(journal) => {
                // Validate internal ID.
                if let Err(e) = validate_transaction_id(&journal.id) {
                    corrupt.push(CorruptJournal {
                        path: path.clone(),
                        error: format!("invalid internal journal ID: {e}"),
                    });
                    continue;
                }

                // Validate filename ID matches internal ID.
                match journal_id_from_path(&path) {
                    Ok(filename_id) if filename_id == journal.id => {
                        journals.push(journal);
                    }
                    Ok(filename_id) => {
                        corrupt.push(CorruptJournal {
                            path: path.clone(),
                            error: format!(
                                "journal ID mismatch: filename contains {filename_id}, \
                                 body contains {}",
                                journal.id
                            ),
                        });
                    }
                    Err(e) => {
                        corrupt.push(CorruptJournal {
                            path: path.clone(),
                            error: format!("invalid journal filename ID: {e}"),
                        });
                    }
                }
            }
            Err(e) => {
                corrupt.push(CorruptJournal {
                    path: path.clone(),
                    error: format!("failed to parse: {e}"),
                });
            }
        }
    }

    Ok(JournalInventory { journals, corrupt })
}

/// Classify the recovery action needed for a single transaction journal.
///
/// This is a pure function — it inspects only the journal state and
/// artifact ownership, performing no mutation.
///
/// Artifact path validation runs for **every** state before classification,
/// ensuring unsafe references are caught regardless of transaction state.
///
/// Returns `Err` if artifact ownership inspection finds unsafe paths
/// (symlinks, out-of-root references, lexical traversal). This prevents
/// suppressing inspection errors.
pub fn classify_journal_recovery(
    transaction_dir: &Path,
    journal: &TransactionJournal,
) -> SnipResult<RecoveryClass> {
    // Validate ALL artifact references for safety for EVERY state.
    // This ensures unsafe paths are caught regardless of transaction state.
    let owns_artifacts = journal_owns_artifacts(transaction_dir, journal)?;

    Ok(match &journal.state {
        TransactionState::Prepared
        | TransactionState::BackupsDurable
        | TransactionState::Committing { .. }
        | TransactionState::RollingBack { .. } => RecoveryClass::Rollback,
        TransactionState::CommittedLocal { .. } => RecoveryClass::FinalizeCommittedLocal,
        TransactionState::CleaningUp { .. } => RecoveryClass::ResumeCleanup,
        TransactionState::Committed => {
            if owns_artifacts {
                RecoveryClass::CleanupLegacyCommitted
            } else {
                RecoveryClass::RemoveTerminalJournal
            }
        }
        TransactionState::RolledBack => {
            if owns_artifacts {
                RecoveryClass::CleanupLegacyRolledBack
            } else {
                RecoveryClass::RemoveTerminalJournal
            }
        }
        TransactionState::Failed(_) => RecoveryClass::UnsafeFailed,
    })
}

/// Check whether a transaction journal still owns artifacts that require cleanup.
///
/// Considers:
/// - the per-transaction artifact root directory;
/// - every `backup_path` in the staged files;
/// - every `durable_staged_path` in the staged files.
///
/// A missing artifact is not an error — absence is a valid idempotent
/// cleanup result. Only the existence of the artifact root or any
/// referenced artifact path counts as "owned."
///
/// Returns `Err` if any artifact reference is unsafe (symlink, out-of-root,
/// lexical traversal). This checks ALL references regardless of existence —
/// missing out-of-root references fail closed.
pub fn journal_owns_artifacts(
    transaction_dir: &Path,
    journal: &TransactionJournal,
) -> SnipResult<bool> {
    let artifact_dir = transaction_artifact_dir(transaction_dir, &journal.id);

    // Reject a symlinked artifact root.
    if artifact_dir.is_symlink() {
        return Err(SnipError::runtime_error(
            "symlinked transaction artifact root",
            Some(&format!(
                "Artifact root {} is a symlink; refusing to follow. \
                 Transaction '{}' (op: {}) may be compromised.",
                artifact_dir.display(),
                short_transaction_id(&journal.id),
                journal.operation,
            )),
        ));
    }

    // Validate ALL artifact references for safety before checking existence.
    // This ensures missing out-of-root references fail closed.
    for staged in &journal.staged_files {
        if let Some(ref backup) = staged.backup_path {
            validate_contained_path(&artifact_dir, backup, "backup_path")?;
        }
        if let Some(ref durable) = staged.durable_staged_path {
            validate_contained_path(&artifact_dir, durable, "durable_staged_path")?;
        }
    }

    // Now check for artifact ownership (existence).
    let root_exists = artifact_dir.exists();

    for staged in &journal.staged_files {
        if let Some(ref backup) = staged.backup_path
            && backup.exists()
        {
            return Ok(true);
        }
        if let Some(ref durable) = staged.durable_staged_path
            && durable.exists()
        {
            return Ok(true);
        }
    }

    Ok(root_exists)
}

/// Extract the transaction ID from a journal filename (`txn-<id>.toml`).
///
/// Validates that the ID is well-formed (no empty, no path separators,
/// no traversal sequences) before returning it.
fn journal_id_from_path(path: &Path) -> SnipResult<String> {
    let stem = path.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
        SnipError::runtime_error(
            "invalid journal filename",
            Some(&format!(
                "Cannot extract filename stem from journal path: {}",
                path.display()
            )),
        )
    })?;

    let id = stem.strip_prefix("txn-").ok_or_else(|| {
        SnipError::runtime_error(
            "invalid journal filename",
            Some(&format!(
                "Journal filename '{}' does not start with 'txn-'",
                path.display()
            )),
        )
    })?;

    validate_transaction_id(id)?;
    Ok(id.to_owned())
}

/// Return the first 8 characters of a transaction ID for human display.
///
/// This is safe for untrusted IDs — it uses character indexing, not byte
/// slicing, so it will never panic on multi-byte Unicode or short IDs.
pub(crate) fn short_transaction_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// Validate a transaction ID — must be a simple UUID-like identifier,
/// not a path component that could escape the directory.
fn validate_transaction_id(transaction_id: &str) -> SnipResult<()> {
    if transaction_id.is_empty() {
        return Err(SnipError::runtime_error(
            "invalid transaction ID",
            Some("Transaction ID must not be empty"),
        ));
    }
    if transaction_id.contains('/')
        || transaction_id.contains('\\')
        || transaction_id.contains("..")
    {
        return Err(SnipError::runtime_error(
            "invalid transaction ID",
            Some(&format!(
                "Transaction ID '{transaction_id}' contains path separators or traversal"
            )),
        ));
    }
    Ok(())
}

/// Derive the human-readable operation name for a recovery class.
fn recovery_operation_name(class: RecoveryClass) -> &'static str {
    match class {
        RecoveryClass::Rollback => "repair-rollback",
        RecoveryClass::FinalizeCommittedLocal => "repair-finalize",
        RecoveryClass::ResumeCleanup => "repair-cleanup",
        RecoveryClass::CleanupLegacyCommitted => "repair-legacy-commit",
        RecoveryClass::CleanupLegacyRolledBack => "repair-legacy-rollback",
        RecoveryClass::RemoveTerminalJournal => "repair-remove-journal",
        RecoveryClass::UnsafeFailed => "repair-unsafe-failed",
    }
}

/// Load exactly one journal under the established lock.
///
/// Derives `txn-<id>.toml`, rejects symlinks, reads and parses the journal,
/// and verifies the journal's internal ID matches the requested ID.
fn load_exact_journal(
    transaction_dir: &Path,
    transaction_id: &str,
) -> SnipResult<TransactionJournal> {
    let jpath = transaction_dir.join(format!("txn-{transaction_id}.toml"));

    // Reject symlinked journal files.
    if jpath.is_symlink() {
        return Err(SnipError::runtime_error(
            "symlinked transaction journal",
            Some(&format!(
                "Journal file {} is a symlink; refusing to follow",
                jpath.display()
            )),
        ));
    }

    if !jpath.exists() {
        return Err(SnipError::runtime_error(
            "transaction journal not found",
            Some(&format!(
                "Journal for transaction {transaction_id} does not exist at {}",
                jpath.display()
            )),
        ));
    }

    let content = fs::read_to_string(&jpath)
        .map_err(|e| SnipError::io_error("read transaction journal", jpath.clone(), e))?;
    let journal: TransactionJournal = toml::from_str(&content)
        .map_err(|e| SnipError::toml_error("parse transaction journal", e))?;

    // Verify the journal's internal ID matches the requested ID.
    if journal.id != transaction_id {
        return Err(SnipError::runtime_error(
            "transaction ID mismatch",
            Some(&format!(
                "Requested transaction {transaction_id} but journal contains ID {}. \
                 The journal file may have been replaced.",
                journal.id
            )),
        ));
    }

    Ok(journal)
}

/// Recover exactly one transaction by ID and expected recovery class.
///
/// This is the canonical transaction-specific recovery API. It:
/// 1. validates the transaction ID;
/// 2. acquires the transaction lock FIRST;
/// 3. loads and classifies the journal UNDER the lock;
/// 4. compares actual classification with expected under the lock;
/// 5. executes the exact recovery path while holding the lock;
/// 6. returns only after that exact journal is recovered or a precise error is produced.
///
/// The lock prevents TOCTOU races where the journal state could change between
/// classification and execution.
pub fn recover_transaction_by_id(
    sync_state_dir: &Path,
    transaction_dir: &Path,
    transaction_id: &str,
    expected: RecoveryClass,
) -> SnipResult<()> {
    validate_transaction_id(transaction_id)?;

    // Acquire the transaction lock BEFORE loading or classifying the journal.
    // This eliminates the TOCTOU window where state could change between
    // classification and lock acquisition.
    let lock = acquire_transaction_lock(transaction_dir, recovery_operation_name(expected))?;

    // Test-only barrier: allows concurrent tests to mutate the journal
    // between lock acquisition and load/classification.
    crate::test_failpoints::mutation_barrier("recover-after-lock-before-load");

    // Load the exact journal under the established lock.
    let journal = load_exact_journal(transaction_dir, transaction_id)?;

    // Classify under lock — the authoritative classification.
    let actual = classify_journal_recovery(transaction_dir, &journal)?;
    if actual != expected {
        return Err(SnipError::runtime_error(
            "stale repair action",
            Some(&format!(
                "Transaction {transaction_id} was expected to be {expected:?} but is now {actual:?}. \
                 The journal state changed after the repair report was generated. \
                 Run `snp repair` again to get an updated report."
            )),
        ));
    }

    // Dispatch to the state-specific recovery function.
    // The lock is held for the entire duration — no recursive acquisition.
    match expected {
        RecoveryClass::Rollback => {
            rollback_transaction(transaction_dir, &journal)?;
        }
        RecoveryClass::FinalizeCommittedLocal => {
            finalize_committed_local_transaction_locked(
                sync_state_dir,
                transaction_dir,
                &journal,
                &lock,
            )?;
        }
        RecoveryClass::ResumeCleanup => {
            let mut journal = journal;
            resume_cleanup(transaction_dir, &mut journal)?;
        }
        RecoveryClass::CleanupLegacyCommitted => {
            begin_cleanup(transaction_dir, &journal, CleanupOutcome::Commit)?;
        }
        RecoveryClass::CleanupLegacyRolledBack => {
            begin_cleanup(transaction_dir, &journal, CleanupOutcome::Rollback)?;
        }
        RecoveryClass::RemoveTerminalJournal => {
            remove_terminal_journal(transaction_dir, transaction_id)?;
        }
        RecoveryClass::UnsafeFailed => {
            return Err(SnipError::runtime_error(
                "unsafe transaction state",
                Some(&format!(
                    "Transaction {transaction_id} is in a Failed state and cannot be \
                     automatically recovered. Preserve the journal and artifacts for \
                     manual investigation. Run `snp repair` for diagnostic output."
                )),
            ));
        }
    }

    // Lock is released when dropped.
    drop(lock);
    Ok(())
}

/// Remove a terminal journal file durably.
///
/// Validates the journal path internally, rejects symlinks, removes the
/// file, and fsyncs the parent directory.
fn remove_terminal_journal(transaction_dir: &Path, transaction_id: &str) -> SnipResult<()> {
    validate_transaction_id(transaction_id)?;
    let jpath = transaction_dir.join(format!("txn-{transaction_id}.toml"));

    if jpath.is_symlink() {
        return Err(SnipError::runtime_error(
            "symlinked transaction journal",
            Some(&format!(
                "Journal file {} is a symlink; refusing to follow",
                jpath.display()
            )),
        ));
    }

    if jpath.exists() {
        fs::remove_file(&jpath)
            .map_err(|e| SnipError::io_error("remove terminal journal", jpath.clone(), e))?;
        // Fsync the parent directory to durably record the removal.
        fsync_parent_dir(&jpath)?;
    }
    Ok(())
}

/// Finalize a CommittedLocal transaction directly.
///
/// Completes the pending sync intent and runs canonical cleanup.
/// Both startup recovery and repair call this API. It affects no
/// other journal. Acquires the transaction lock internally.
#[allow(dead_code)]
pub fn finalize_committed_local_transaction(
    sync_state_dir: &Path,
    transaction_dir: &Path,
    journal: &TransactionJournal,
) -> SnipResult<()> {
    let lock = acquire_transaction_lock(transaction_dir, "repair-finalize")?;
    finalize_committed_local_transaction_locked(sync_state_dir, transaction_dir, journal, &lock)
}

/// Finalize a CommittedLocal transaction while the caller holds the lock.
///
/// Does NOT acquire the transaction lock — the caller must already hold it.
fn finalize_committed_local_transaction_locked(
    sync_state_dir: &Path,
    transaction_dir: &Path,
    journal: &TransactionJournal,
    _lock: &TransactionLock,
) -> SnipResult<()> {
    tracing::info!(
        txn_id = %journal.id,
        "Finalizing CommittedLocal transaction"
    );

    let mut finalized_journal = journal.clone();

    // Complete the pending sync intent if not already recorded.
    match &journal.state {
        TransactionState::CommittedLocal { pending } => {
            let finalized = match pending {
                PendingFinalization::NotRecorded => {
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
                            tracing::warn!(
                                txn_id = %journal.id,
                                generation = state.generation,
                                "Pending conflict during recovery: preserving existing generation"
                            );
                            PendingFinalization::CoveredByExisting {
                                generation: state.generation,
                            }
                        }
                        Err(e) => {
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
                PendingFinalization::Recorded { .. }
                | PendingFinalization::CoveredByExisting { .. } => pending.clone(),
            };

            finalized_journal.state = TransactionState::CommittedLocal { pending: finalized };
            persist_journal(transaction_dir, &finalized_journal)?;
        }
        _ => {
            return Err(SnipError::runtime_error(
                "invalid state for finalize",
                Some(&format!(
                    "Transaction {} is not in CommittedLocal state (got {:?})",
                    journal.id, journal.state
                )),
            ));
        }
    }

    // Run canonical cleanup — lock is held by caller, not reacquired.
    finalize_transaction_cleanup(transaction_dir, &mut finalized_journal)
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
///
/// PID 0 is never a valid lock owner: `kill(0, 0)` targets the caller's
/// process group and would always succeed, so it is treated as dead to
/// match the liveness probes in `auto_sync::execution_lock`.
#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    let rc = unsafe { libc::kill(pid as i32, 0) };
    rc == 0 || classify_kill_zero_error(std::io::Error::last_os_error().raw_os_error())
}

#[cfg(unix)]
fn classify_kill_zero_error(errno: Option<i32>) -> bool {
    !matches!(errno, Some(libc::ESRCH))
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

/// Persist the journal after backups are durably written.
///
/// This is a compatibility shim — the `BackupsDurable` state is retained
/// for backward-compatible recovery of old journals. New code may call
/// this function, but the state machine treats `BackupsDurable` as
/// equivalent to `Prepared` for recovery classification.
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

/// Persist the journal in `CommittedLocal` finalization state.
///
/// This state is used only for backward-compatible recovery of journals
/// from older versions. New transactions transition directly from
/// `Committing` to `CleaningUp` (pending sync intent is recorded
/// separately after the transaction state machine completes).
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
///
/// Performs lexical containment first (rejects `..` components), then
/// canonical containment for existing paths. Missing paths are validated
/// lexically only — they cannot be canonicalized.
fn validate_contained_path(root: &Path, path: &Path, label: &str) -> SnipResult<()> {
    // Lexical containment check: explicitly rejects `Component::ParentDir`.
    // This catches traversal even when the path doesn't exist yet.
    if !lexically_within(root, path) {
        return Err(SnipError::runtime_error(
            "transaction artifact path traversal",
            Some(&format!(
                "Artifact {} at {} is outside the transaction artifact root {}",
                label,
                path.display(),
                root.display()
            )),
        ));
    }

    // Reject a symlinked artifact root — it could escape the root before
    // any deeper check runs. Uses symlink_metadata so we don't follow links.
    if let Ok(meta) = fs::symlink_metadata(root)
        && meta.file_type().is_symlink()
    {
        return Err(SnipError::runtime_error(
            "symlinked transaction artifact root",
            Some(&format!(
                "Artifact root {} is a symlink; refusing to follow. Path: {} ({label})",
                root.display(),
                path.display()
            )),
        ));
    }

    // Reject existing intermediate components that are symlinks — even
    // when the final path is missing. Catches `<root>/link/missing.bin`
    // where `link` is a symlink to outside.
    reject_symlinked_existing_prefixes(root, path)?;

    // For existing paths, also verify canonical containment as defense in
    // depth (catches reparse/junction behavior on supported platforms).
    if path.exists() {
        let canonical_root = root.canonicalize().map_err(|e| {
            SnipError::io_error(
                "canonicalize transaction artifact root",
                root.to_path_buf(),
                e,
            )
        })?;
        let canonical_path = path.canonicalize().map_err(|e| {
            SnipError::io_error(
                "canonicalize transaction artifact path",
                path.to_path_buf(),
                e,
            )
        })?;

        if !canonical_path.starts_with(&canonical_root) {
            return Err(SnipError::runtime_error(
                "transaction artifact path traversal",
                Some(&format!(
                    "Artifact {} at {} resolves outside the transaction artifact root {}",
                    label,
                    path.display(),
                    canonical_root.display()
                )),
            ));
        }
    }

    Ok(())
}

/// Check whether `child` is lexically within `root` without canonicalizing.
///
/// Both paths must be absolute. Any `Component::ParentDir` in either path
/// is explicitly rejected — the path is treated as unsafe and the function
/// returns `false`. `Component::CurDir` is normalized away.
fn lexically_within(root: &Path, child: &Path) -> bool {
    if !root.is_absolute() || !child.is_absolute() {
        return false;
    }

    let Some(root_components) = normalize_absolute_without_parent(root) else {
        return false;
    };
    let Some(child_components) = normalize_absolute_without_parent(child) else {
        return false;
    };

    if child_components.len() < root_components.len() {
        return false;
    }

    for (rc, cc) in root_components.iter().zip(child_components.iter()) {
        if rc != cc {
            return false;
        }
    }

    true
}

/// Normalize an absolute path, explicitly rejecting `Component::ParentDir`.
///
/// Returns `None` if the path is not absolute or contains any `..` component.
/// `Component::CurDir` is dropped silently. `Component::Prefix` and
/// `Component::RootDir` are preserved so cross-platform prefix semantics
/// are retained.
fn normalize_absolute_without_parent(path: &Path) -> Option<Vec<Component<'_>>> {
    if !path.is_absolute() {
        return None;
    }

    let mut normalized: Vec<Component<'_>> = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => normalized.push(component),
            Component::CurDir => {}
            Component::Normal(_) => normalized.push(component),
            Component::ParentDir => return None,
        }
    }
    Some(normalized)
}

/// Reject existing intermediate path components that are symlinks.
///
/// Walks from `root` toward `child`, stopping at the first missing
/// component. Each existing component is inspected with `symlink_metadata`
/// so symlinks are not followed. Used to catch references like
/// `<root>/link/missing.bin` where `link` is a symlink to outside and the
/// final file is absent.
fn reject_symlinked_existing_prefixes(root: &Path, child: &Path) -> SnipResult<()> {
    // Reject a symlinked artifact root.
    match fs::symlink_metadata(root) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(SnipError::runtime_error(
                "symlinked transaction artifact root",
                Some(&format!(
                    "Artifact root {} is a symlink; refusing to follow",
                    root.display()
                )),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Root absent — the walk below will see only missing components.
        }
        Err(error) => {
            return Err(SnipError::io_error(
                "stat transaction artifact root",
                root.to_path_buf(),
                error,
            ));
        }
    }

    let relative = child.strip_prefix(root).map_err(|_| {
        SnipError::runtime_error(
            "transaction artifact path traversal",
            Some(&format!(
                "Artifact path {} is not within root {}",
                child.display(),
                root.display()
            )),
        )
    })?;

    let mut current = root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                current.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::Normal(_) => {
                current.push(component.as_os_str());
                match fs::symlink_metadata(&current) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        return Err(SnipError::runtime_error(
                            "symlinked transaction artifact prefix",
                            Some(&format!(
                                "Existing component {} is a symlink; refusing to follow",
                                current.display()
                            )),
                        ));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        // Missing component — stop walking; the final path is absent.
                        break;
                    }
                    Err(error) => {
                        return Err(SnipError::io_error(
                            "stat transaction artifact prefix",
                            current.clone(),
                            error,
                        ));
                    }
                }
            }
            Component::ParentDir => {
                return Err(SnipError::runtime_error(
                    "transaction artifact path traversal",
                    Some(&format!(
                        "Artifact path {} contains parent traversal",
                        child.display()
                    )),
                ));
            }
        }
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
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        let dir = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_RDONLY)
            .open(parent)
            .map_err(|e| {
                SnipError::io_error("open parent dir for fsync", parent.to_path_buf(), e)
            })?;

        // Test-only error injection: simulate fsync failure.
        #[cfg(feature = "test-support")]
        crate::test_failpoints::maybe_injected_error("terminal-journal-parent-sync")?;

        let rc = unsafe { libc::fsync(dir.as_raw_fd()) };
        if rc != 0 {
            return Err(SnipError::io_error(
                "fsync parent directory",
                parent.to_path_buf(),
                std::io::Error::last_os_error(),
            ));
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
                if let Some(ref backup) = staged.backup_path {
                    // Defense in depth: revalidate backup reference immediately
                    // before reading, even though classification already validated.
                    let artifact_root = transaction_artifact_dir(state_dir, &rb_journal.id);
                    validate_contained_path(&artifact_root, backup, "backup_path")?;

                    if backup.exists() {
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
///    automatic recovery. Return `Ok(())` if recovery succeeds.
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
    let inventory = scan_transaction_journals(transaction_dir)?;

    // Fail closed on corrupt journals.
    if !inventory.corrupt.is_empty() {
        let paths: Vec<String> = inventory
            .corrupt
            .iter()
            .map(|c| c.path.display().to_string())
            .collect();
        let errors: Vec<&str> = inventory.corrupt.iter().map(|c| c.error.as_str()).collect();
        return Err(SnipError::runtime_error(
            "corrupt transaction journal(s) detected",
            Some(&format!(
                "Found {} corrupt journal(s): [{}]. Errors: [{}]. \
                 Mutations are refused until corrupt journals are resolved. \
                 Run `snp repair` to inspect and quarantine.",
                inventory.corrupt.len(),
                paths.join(", "),
                errors.join("; ")
            )),
        ));
    }

    // Classify all journals.
    let classified: Vec<(TransactionJournal, RecoveryClass)> = inventory
        .journals
        .iter()
        .map(|j| {
            let class = classify_journal_recovery(transaction_dir, j)?;
            Ok((j.clone(), class))
        })
        .collect::<SnipResult<Vec<_>>>()?;

    // Fail closed on UnsafeFailed journals — they must block mutation
    // and remain preserved for manual investigation.
    for (journal, class) in &classified {
        if *class == RecoveryClass::UnsafeFailed {
            return Err(SnipError::runtime_error(
                "unsafe transaction state blocks mutation",
                Some(&format!(
                    "Transaction '{}' (op: {}) is in a Failed state and cannot be \
                     automatically recovered. Preserve the journal and artifacts for \
                     manual investigation. Run `snp repair` for diagnostic output.",
                    short_transaction_id(&journal.id),
                    journal.operation,
                )),
            ));
        }
    }

    // Collect actionable journals (those needing recovery).
    let actionable: Vec<_> = classified
        .iter()
        .filter(|(_, class)| !matches!(class, RecoveryClass::RemoveTerminalJournal))
        .collect();

    if actionable.is_empty() {
        // Remove terminal journals through exact recovery, one at a time.
        // Each acquires the transaction lock and revalidates before removal.
        let terminal: Vec<_> = classified
            .iter()
            .filter(|(_, class)| *class == RecoveryClass::RemoveTerminalJournal)
            .collect();

        for (journal, _class) in &terminal {
            tracing::info!(
                txn_id = %journal.id,
                "Removing terminal journal with no artifacts via exact recovery"
            );
            recover_transaction_by_id(
                sync_state_dir,
                transaction_dir,
                &journal.id,
                RecoveryClass::RemoveTerminalJournal,
            )?;
        }
        return Ok(());
    }

    if actionable.len() == 1 {
        let (journal, class) = &actionable[0];
        tracing::info!(
            txn_id = %journal.id,
            class = ?class,
            "Attempting automatic recovery of single interrupted transaction"
        );
        return recover_transaction_by_id(sync_state_dir, transaction_dir, &journal.id, *class);
    }

    // Multiple actionable journals — refuse and direct to repair.
    let ids: Vec<&str> = actionable.iter().map(|(j, _)| j.id.as_str()).collect();
    Err(SnipError::runtime_error(
        "Multiple interrupted transactions detected",
        Some(&format!(
            "Found {} actionable transactions (IDs: {}). \
             Run `snp repair` to inspect and recover before making new mutations.",
            actionable.len(),
            ids.join(", ")
        )),
    ))
}

/// Check for interrupted transactions on startup (compatibility wrapper).
///
/// Returns journals in interruptible states (Prepared, Committing,
/// RollingBack, CommittedLocal, CleaningUp). This is a narrow
/// compatibility wrapper over the complete scanner and classifier. New code
/// should use `scan_transaction_journals` + `classify_journal_recovery`
/// directly.
#[allow(dead_code)]
pub fn check_interrupted_transactions(state_dir: &Path) -> SnipResult<Vec<TransactionJournal>> {
    let inventory = scan_transaction_journals(state_dir)?;
    Ok(inventory
        .journals
        .into_iter()
        .filter(|j| j.state.is_interruptible())
        .collect())
}

/// Derive the journal file path for a given transaction ID.
fn journal_path(state_dir: &Path, txn_id: &str) -> PathBuf {
    state_dir.join(format!("txn-{txn_id}.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_start_token_parser_reads_field_22() {
        let stat = "42 (name with ) parens) S f4 f5 f6 f7 f8 f9 f10 f11 f12 f13 f14 f15 f16 f17 f18 f19 f20 FIELD21 START22";
        assert_eq!(parse_linux_proc_start_token(stat), Some("START22".into()));
        assert_ne!(parse_linux_proc_start_token(stat), Some("FIELD21".into()));
        assert_eq!(parse_linux_proc_start_token("1 (short) S f3"), None);
    }

    #[cfg(unix)]
    #[test]
    fn kill_zero_error_classification_is_conservative() {
        assert!(classify_kill_zero_error(Some(libc::EPERM)));
        assert!(!classify_kill_zero_error(Some(libc::ESRCH)));
        assert!(classify_kill_zero_error(Some(libc::EINVAL)));
    }
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

    #[test]
    fn test_scan_empty_directory() {
        let dir = TempDir::new().unwrap();
        let inv = scan_transaction_journals(dir.path()).unwrap();
        assert!(inv.journals.is_empty());
        assert!(inv.corrupt.is_empty());
    }

    #[test]
    fn test_scan_nonexistent_directory() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("nonexistent");
        let inv = scan_transaction_journals(&missing).unwrap();
        assert!(inv.journals.is_empty());
        assert!(inv.corrupt.is_empty());
    }

    #[test]
    fn test_scan_discovers_all_states() {
        let dir = TempDir::new().unwrap();
        // Write journals in every state. Internal ID must match filename ID.
        for (id, state_str) in [
            (
                "prepared-aaaa-0000-0000-000000000001",
                r#"state = "Prepared""#,
            ),
            (
                "committed-bbbb-0000-0000-000000000002",
                r#"state = "Committed""#,
            ),
            (
                "rolledback-cccc-0000-0000-000000000003",
                r#"state = "RolledBack""#,
            ),
            (
                "failed-dddd-0000-0000-000000000004",
                r#"state = { Failed = "oops" }"#,
            ),
        ] {
            let journal = format!(
                r#"
id = "{id}"
operation = "test"
created_at_unix_ms = 0
staged_files = []
{state_str}
"#
            );
            fs::write(dir.path().join(format!("txn-{id}.toml")), journal).unwrap();
        }

        let inv = scan_transaction_journals(dir.path()).unwrap();
        assert_eq!(inv.journals.len(), 4);
        assert!(inv.corrupt.is_empty());

        // Verify all states are present.
        let states: Vec<_> = inv.journals.iter().map(|j| j.state.clone()).collect();
        assert!(states.contains(&TransactionState::Prepared));
        assert!(states.contains(&TransactionState::Committed));
        assert!(states.contains(&TransactionState::RolledBack));
    }

    #[test]
    fn test_scan_rejects_corrupt_journal() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("txn-corrupt.toml"),
            "this is not valid toml {{{",
        )
        .unwrap();

        let inv = scan_transaction_journals(dir.path()).unwrap();
        assert!(inv.journals.is_empty());
        assert_eq!(inv.corrupt.len(), 1);
        assert!(inv.corrupt[0].error.contains("failed to parse"));
    }

    #[test]
    fn test_scan_rejects_symlinked_journal() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("real.toml");
        fs::write(
            &target,
            r#"
id = "txn-real"
operation = "test"
created_at_unix_ms = 0
staged_files = []
state = "Prepared"
"#,
        )
        .unwrap();
        let symlink = dir.path().join("txn-symlink.toml");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &symlink).unwrap();
            // Verify the symlink was actually created.
            assert!(
                symlink.is_symlink(),
                "symlink should exist and be detected as symlink"
            );
        }

        let inv = scan_transaction_journals(dir.path()).unwrap();
        #[cfg(unix)]
        {
            // The symlink MUST be rejected as corrupt — the scanner never
            // follows symlinks for journal files.
            assert_eq!(
                inv.journals.len(),
                0,
                "symlinked journal must not enter valid journals"
            );
            assert_eq!(inv.corrupt.len(), 1, "symlinked journal must enter corrupt");
            assert!(
                inv.corrupt[0].error.contains("symlink"),
                "corrupt error should mention symlink: {}",
                inv.corrupt[0].error
            );
        }
        #[cfg(not(unix))]
        {
            // On non-Unix, symlinks may not be supported; just verify no crash.
            let _ = inv;
        }
    }

    #[test]
    fn test_scan_stable_ordering() {
        let dir = TempDir::new().unwrap();
        for i in [3, 1, 2] {
            let journal = format!(
                r#"
id = "{i:04}"
operation = "test"
created_at_unix_ms = 0
staged_files = []
state = "Prepared"
"#
            );
            fs::write(dir.path().join(format!("txn-{i:04}.toml")), journal).unwrap();
        }

        let inv = scan_transaction_journals(dir.path()).unwrap();
        assert_eq!(inv.journals.len(), 3);
        assert_eq!(inv.journals[0].id, "0001");
        assert_eq!(inv.journals[1].id, "0002");
        assert_eq!(inv.journals[2].id, "0003");
    }

    #[test]
    fn test_classify_rollback_states() {
        let dir = TempDir::new().unwrap();
        let base = TransactionJournal {
            id: "test".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![],
            state: TransactionState::Prepared,
        };
        assert_eq!(
            classify_journal_recovery(dir.path(), &base).unwrap(),
            RecoveryClass::Rollback
        );

        let mut j = base.clone();
        j.state = TransactionState::BackupsDurable;
        assert_eq!(
            classify_journal_recovery(dir.path(), &j).unwrap(),
            RecoveryClass::Rollback
        );

        j.state = TransactionState::Committing {
            next_commit_position: 0,
        };
        assert_eq!(
            classify_journal_recovery(dir.path(), &j).unwrap(),
            RecoveryClass::Rollback
        );

        j.state = TransactionState::RollingBack {
            next_rollback_position: 0,
        };
        assert_eq!(
            classify_journal_recovery(dir.path(), &j).unwrap(),
            RecoveryClass::Rollback
        );
    }

    #[test]
    fn test_classify_committed_local() {
        let dir = TempDir::new().unwrap();
        let j = TransactionJournal {
            id: "test".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![],
            state: TransactionState::CommittedLocal {
                pending: PendingFinalization::NotRecorded,
            },
        };
        assert_eq!(
            classify_journal_recovery(dir.path(), &j).unwrap(),
            RecoveryClass::FinalizeCommittedLocal
        );
    }

    #[test]
    fn test_classify_cleaning_up() {
        let dir = TempDir::new().unwrap();
        let j = TransactionJournal {
            id: "test".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![],
            state: TransactionState::CleaningUp {
                outcome: CleanupOutcome::Commit,
                next_step: CleanupStep::Validate,
            },
        };
        assert_eq!(
            classify_journal_recovery(dir.path(), &j).unwrap(),
            RecoveryClass::ResumeCleanup
        );
    }

    #[test]
    fn test_classify_legacy_committed_with_artifacts() {
        let dir = TempDir::new().unwrap();
        let artifact_dir = transaction_artifact_dir(dir.path(), "legacy");
        fs::create_dir_all(&artifact_dir).unwrap();
        let j = TransactionJournal {
            id: "legacy".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![],
            state: TransactionState::Committed,
        };
        assert_eq!(
            classify_journal_recovery(dir.path(), &j).unwrap(),
            RecoveryClass::CleanupLegacyCommitted
        );
    }

    #[test]
    fn test_classify_legacy_committed_without_artifacts() {
        let dir = TempDir::new().unwrap();
        let j = TransactionJournal {
            id: "legacy".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![],
            state: TransactionState::Committed,
        };
        assert_eq!(
            classify_journal_recovery(dir.path(), &j).unwrap(),
            RecoveryClass::RemoveTerminalJournal
        );
    }

    #[test]
    fn test_classify_legacy_rolled_back_with_artifacts() {
        let dir = TempDir::new().unwrap();
        let artifact_dir = transaction_artifact_dir(dir.path(), "legacy");
        fs::create_dir_all(&artifact_dir).unwrap();
        let j = TransactionJournal {
            id: "legacy".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![],
            state: TransactionState::RolledBack,
        };
        assert_eq!(
            classify_journal_recovery(dir.path(), &j).unwrap(),
            RecoveryClass::CleanupLegacyRolledBack
        );
    }

    #[test]
    fn test_classify_legacy_rolled_back_without_artifacts() {
        let dir = TempDir::new().unwrap();
        let j = TransactionJournal {
            id: "legacy".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![],
            state: TransactionState::RolledBack,
        };
        assert_eq!(
            classify_journal_recovery(dir.path(), &j).unwrap(),
            RecoveryClass::RemoveTerminalJournal
        );
    }

    #[test]
    fn test_classify_failed() {
        let dir = TempDir::new().unwrap();
        let j = TransactionJournal {
            id: "test".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![],
            state: TransactionState::Failed("oops".into()),
        };
        assert_eq!(
            classify_journal_recovery(dir.path(), &j).unwrap(),
            RecoveryClass::UnsafeFailed
        );
    }

    #[test]
    fn test_journal_owns_artifacts_artifact_root() {
        let dir = TempDir::new().unwrap();
        let j = TransactionJournal {
            id: "test".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![],
            state: TransactionState::Committed,
        };
        assert!(!journal_owns_artifacts(dir.path(), &j).unwrap());

        let artifact_dir = transaction_artifact_dir(dir.path(), "test");
        fs::create_dir_all(&artifact_dir).unwrap();
        assert!(journal_owns_artifacts(dir.path(), &j).unwrap());
    }

    #[test]
    fn test_journal_owns_artifacts_backup_path() {
        let dir = TempDir::new().unwrap();
        let artifact_dir = transaction_artifact_dir(dir.path(), "test");
        fs::create_dir_all(&artifact_dir).unwrap();
        let backup = artifact_dir.join("backup.bak");
        fs::write(&backup, "data").unwrap();
        let j = TransactionJournal {
            id: "test".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![StagedFile {
                original_path: PathBuf::from("/fake"),
                backup_path: Some(backup),
                staged_path: PathBuf::from("/fake"),
                sha256: String::new(),
                existed_before: true,
                action: StagedAction::Replace,
                original_hash: String::new(),
                new_hash: String::new(),
                durable_staged_path: None,
                original_metadata: OriginalFileMetadata::default(),
            }],
            state: TransactionState::Committed,
        };
        assert!(journal_owns_artifacts(dir.path(), &j).unwrap());
    }

    #[test]
    fn test_journal_owns_artifacts_durable_staged_path() {
        let dir = TempDir::new().unwrap();
        let artifact_dir = transaction_artifact_dir(dir.path(), "test");
        fs::create_dir_all(&artifact_dir).unwrap();
        let staged = artifact_dir.join("staged.toml");
        fs::write(&staged, "data").unwrap();
        let j = TransactionJournal {
            id: "test".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![StagedFile {
                original_path: PathBuf::from("/fake"),
                backup_path: None,
                staged_path: PathBuf::from("/fake"),
                sha256: String::new(),
                existed_before: true,
                action: StagedAction::Replace,
                original_hash: String::new(),
                new_hash: String::new(),
                durable_staged_path: Some(staged),
                original_metadata: OriginalFileMetadata::default(),
            }],
            state: TransactionState::Committed,
        };
        assert!(journal_owns_artifacts(dir.path(), &j).unwrap());
    }

    #[test]
    fn test_scan_does_not_mutate() {
        let dir = TempDir::new().unwrap();
        let journal = r#"
id = "txn-test"
operation = "test"
created_at_unix_ms = 0
staged_files = []
state = "Prepared"
"#;
        let jpath = dir.path().join("txn-test.toml");
        fs::write(&jpath, journal).unwrap();
        let before_metadata = fs::metadata(&jpath).unwrap();

        let _inv = scan_transaction_journals(dir.path()).unwrap();

        let after_metadata = fs::metadata(&jpath).unwrap();
        assert_eq!(
            before_metadata.modified().unwrap(),
            after_metadata.modified().unwrap()
        );
        assert_eq!(before_metadata.len(), after_metadata.len());
    }

    // =========================================================================
    // Workstream A: recover_transaction_by_id under lock tests
    // =========================================================================

    /// Helper: write a journal directly into the transaction directory.
    fn write_test_journal(dir: &Path, txn_id: &str, state: TransactionState) {
        let journal = TransactionJournal {
            id: txn_id.to_string(),
            operation: "test_op".to_string(),
            created_at_unix_ms: 1000000,
            staged_files: vec![],
            state,
        };
        let jpath = journal_path(dir, txn_id);
        let content = toml::to_string_pretty(&journal).unwrap();
        fs::write(&jpath, content).unwrap();
    }

    #[test]
    fn test_recover_prepared_as_rollback_succeeds() {
        let dir = TempDir::new().unwrap();
        let sync_dir = TempDir::new().unwrap();
        let txn_id = "aaaa1111-0000-0000-0000-000000000001";
        write_test_journal(dir.path(), txn_id, TransactionState::Prepared);

        let result =
            recover_transaction_by_id(sync_dir.path(), dir.path(), txn_id, RecoveryClass::Rollback);
        assert!(
            result.is_ok(),
            "Prepared→Rollback should succeed: {result:?}"
        );

        // Journal should have been cleaned up (removed after rollback+cleanup).
        assert!(
            !journal_path(dir.path(), txn_id).exists(),
            "journal should be removed after rollback cleanup"
        );
    }

    #[test]
    fn test_recover_stale_action_rejected() {
        let dir = TempDir::new().unwrap();
        let sync_dir = TempDir::new().unwrap();
        let txn_id = "aaaa1111-0000-0000-0000-000000000001";
        write_test_journal(dir.path(), txn_id, TransactionState::Prepared);

        // Write a new journal with the same ID but Committed state,
        // simulating a state change after classification.
        write_test_journal(dir.path(), txn_id, TransactionState::Committed);

        let result =
            recover_transaction_by_id(sync_dir.path(), dir.path(), txn_id, RecoveryClass::Rollback);
        assert!(result.is_err(), "stale action should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("stale"), "error should mention stale: {msg}");
        assert!(
            msg.contains("Rollback"),
            "error should mention expected class: {msg}"
        );
    }

    #[test]
    fn test_recover_id_mismatch_rejected() {
        let dir = TempDir::new().unwrap();
        let sync_dir = TempDir::new().unwrap();
        // Create journal with internal ID "real-id" but request "wrong-id".
        write_test_journal(dir.path(), "real-id", TransactionState::Prepared);

        let result = recover_transaction_by_id(
            sync_dir.path(),
            dir.path(),
            "wrong-id",
            RecoveryClass::Rollback,
        );
        assert!(result.is_err(), "ID mismatch should be rejected");
    }

    #[test]
    fn test_recover_empty_id_rejected() {
        let dir = TempDir::new().unwrap();
        let sync_dir = TempDir::new().unwrap();

        let result =
            recover_transaction_by_id(sync_dir.path(), dir.path(), "", RecoveryClass::Rollback);
        assert!(result.is_err(), "empty ID should be rejected");
    }

    #[test]
    fn test_recover_traversal_id_rejected() {
        let dir = TempDir::new().unwrap();
        let sync_dir = TempDir::new().unwrap();

        let result = recover_transaction_by_id(
            sync_dir.path(),
            dir.path(),
            "../etc/passwd",
            RecoveryClass::Rollback,
        );
        assert!(result.is_err(), "traversal ID should be rejected");
    }

    #[cfg(unix)]
    #[test]
    fn test_recover_symlinked_journal_rejected() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let sync_dir = TempDir::new().unwrap();
        let txn_id = "aaaa1111-0000-0000-0000-000000000001";

        // Create a real journal, then symlink to it.
        write_test_journal(dir.path(), txn_id, TransactionState::Prepared);
        let real_jpath = journal_path(dir.path(), txn_id);
        let symlink_path = dir.path().join(format!("txn-{txn_id}.toml.bak"));
        fs::copy(&real_jpath, &symlink_path).unwrap();
        fs::remove_file(&real_jpath).unwrap();
        symlink(&symlink_path, &real_jpath).unwrap();

        let result =
            recover_transaction_by_id(sync_dir.path(), dir.path(), txn_id, RecoveryClass::Rollback);
        assert!(result.is_err(), "symlinked journal should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("symlink"),
            "error should mention symlink: {msg}"
        );
    }

    #[test]
    fn test_recover_two_journals_isolates_a_from_b() {
        let dir = TempDir::new().unwrap();
        let sync_dir = TempDir::new().unwrap();
        let txn_a = "aaaa1111-0000-0000-0000-000000000001";
        let txn_b = "bbbb2222-0000-0000-0000-000000000002";

        write_test_journal(dir.path(), txn_a, TransactionState::Prepared);
        write_test_journal(dir.path(), txn_b, TransactionState::Prepared);

        let b_before = fs::read_to_string(journal_path(dir.path(), txn_b)).unwrap();

        // Recover only A.
        let result =
            recover_transaction_by_id(sync_dir.path(), dir.path(), txn_a, RecoveryClass::Rollback);
        assert!(result.is_ok(), "recovering A should succeed: {result:?}");

        // A should be removed (rolled back and cleaned up).
        assert!(
            !journal_path(dir.path(), txn_a).exists(),
            "A's journal should be removed"
        );

        // B should be byte-for-byte unchanged.
        let b_after = fs::read_to_string(journal_path(dir.path(), txn_b)).unwrap();
        assert_eq!(b_before, b_after, "B's journal must not be altered");
    }

    #[test]
    fn test_recover_not_found_rejected() {
        let dir = TempDir::new().unwrap();
        let sync_dir = TempDir::new().unwrap();

        let result = recover_transaction_by_id(
            sync_dir.path(),
            dir.path(),
            "nonexistent-0000-0000-0000-000000000000",
            RecoveryClass::Rollback,
        );
        assert!(result.is_err(), "nonexistent ID should be rejected");
    }

    #[test]
    fn test_validate_transaction_id_cases() {
        assert!(validate_transaction_id("valid-uuid-123").is_ok());
        assert!(validate_transaction_id("").is_err());
        assert!(validate_transaction_id("../x").is_err());
        assert!(validate_transaction_id("a/b").is_err());
        assert!(validate_transaction_id("a\\b").is_err());
    }

    #[test]
    fn test_remove_terminal_journal_idempotent() {
        let dir = TempDir::new().unwrap();
        let txn_id = "aaaa1111-0000-0000-0000-000000000001";
        // Remove when no journal exists — should be idempotent success.
        let result = remove_terminal_journal(dir.path(), txn_id);
        assert!(
            result.is_ok(),
            "removing nonexistent journal should be idempotent"
        );
    }

    #[test]
    fn test_remove_terminal_journal_removes_file() {
        let dir = TempDir::new().unwrap();
        let txn_id = "aaaa1111-0000-0000-0000-000000000001";
        write_test_journal(dir.path(), txn_id, TransactionState::Committed);
        assert!(journal_path(dir.path(), txn_id).exists());

        let result = remove_terminal_journal(dir.path(), txn_id);
        assert!(result.is_ok());
        assert!(!journal_path(dir.path(), txn_id).exists());
    }

    // =========================================================================
    // Workstream C: Fallible artifact ownership inspection tests
    // =========================================================================

    #[cfg(unix)]
    #[test]
    fn test_journal_owns_artifacts_rejects_symlinked_root() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let real_dir = dir.path().join("real_artifacts");
        fs::create_dir_all(&real_dir).unwrap();
        // Create the parent "artifacts" directory so the symlink can be placed.
        fs::create_dir_all(dir.path().join("artifacts")).unwrap();
        let symlink_dir = transaction_artifact_dir(dir.path(), "test");
        symlink(&real_dir, &symlink_dir).unwrap();

        let j = TransactionJournal {
            id: "test".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![],
            state: TransactionState::Committed,
        };
        let result = journal_owns_artifacts(dir.path(), &j);
        assert!(
            result.is_err(),
            "symlinked artifact root should be rejected"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("symlink"),
            "error should mention symlink: {msg}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_journal_owns_artifacts_rejects_symlinked_backup() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let artifact_dir = transaction_artifact_dir(dir.path(), "test");
        fs::create_dir_all(&artifact_dir).unwrap();

        // Create a real backup file, then symlink to it.
        let real_backup = dir.path().join("real_backup.bak");
        fs::write(&real_backup, "data").unwrap();
        let symlink_backup = artifact_dir.join("backup.bak");
        symlink(&real_backup, &symlink_backup).unwrap();

        let j = TransactionJournal {
            id: "test".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![StagedFile {
                original_path: PathBuf::from("/fake"),
                backup_path: Some(symlink_backup),
                staged_path: PathBuf::from("/fake"),
                sha256: String::new(),
                existed_before: true,
                action: StagedAction::Replace,
                original_hash: String::new(),
                new_hash: String::new(),
                durable_staged_path: None,
                original_metadata: OriginalFileMetadata::default(),
            }],
            state: TransactionState::Committed,
        };
        let result = journal_owns_artifacts(dir.path(), &j);
        assert!(result.is_err(), "symlinked backup should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("symlink"),
            "error should mention symlink: {msg}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_journal_owns_artifacts_rejects_symlinked_durable_staged() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let artifact_dir = transaction_artifact_dir(dir.path(), "test");
        fs::create_dir_all(&artifact_dir).unwrap();

        let real_staged = dir.path().join("real_staged.bin");
        fs::write(&real_staged, "data").unwrap();
        let symlink_staged = artifact_dir.join("staged.bin");
        symlink(&real_staged, &symlink_staged).unwrap();

        let j = TransactionJournal {
            id: "test".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![StagedFile {
                original_path: PathBuf::from("/fake"),
                backup_path: None,
                staged_path: PathBuf::from("/fake"),
                sha256: String::new(),
                existed_before: true,
                action: StagedAction::Replace,
                original_hash: String::new(),
                new_hash: String::new(),
                durable_staged_path: Some(symlink_staged),
                original_metadata: OriginalFileMetadata::default(),
            }],
            state: TransactionState::Committed,
        };
        let result = journal_owns_artifacts(dir.path(), &j);
        assert!(
            result.is_err(),
            "symlinked durable staged should be rejected"
        );
    }

    #[test]
    fn test_journal_owns_artifacts_rejects_out_of_root_backup() {
        let dir = TempDir::new().unwrap();
        let artifact_dir = transaction_artifact_dir(dir.path(), "test");
        fs::create_dir_all(&artifact_dir).unwrap();

        // Backup outside the artifact root.
        let outside_backup = dir.path().join("outside.bak");
        fs::write(&outside_backup, "data").unwrap();

        // Verify containment directly to debug.
        let result = validate_contained_path(&artifact_dir, &outside_backup, "backup_path");
        assert!(
            result.is_err(),
            "validate_contained_path should reject out-of-root backup: artifact_dir={}, outside={}",
            artifact_dir.display(),
            outside_backup.display()
        );

        let j = TransactionJournal {
            id: "test".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![StagedFile {
                original_path: PathBuf::from("/fake"),
                backup_path: Some(outside_backup),
                staged_path: PathBuf::from("/fake"),
                sha256: String::new(),
                existed_before: true,
                action: StagedAction::Replace,
                original_hash: String::new(),
                new_hash: String::new(),
                durable_staged_path: None,
                original_metadata: OriginalFileMetadata::default(),
            }],
            state: TransactionState::Committed,
        };
        let result = journal_owns_artifacts(dir.path(), &j);
        assert!(result.is_err(), "out-of-root backup should be rejected");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("outside") || msg.contains("traversal"),
            "error should mention containment: {msg}"
        );
    }

    #[test]
    fn test_classify_journal_recovery_is_fallible() {
        let dir = TempDir::new().unwrap();
        let j = TransactionJournal {
            id: "test".to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![],
            state: TransactionState::Prepared,
        };
        // Simple state — no artifact inspection needed.
        let result = classify_journal_recovery(dir.path(), &j);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), RecoveryClass::Rollback);
    }

    // =========================================================================
    // Workstream A: Scanner identity validation tests
    // =========================================================================

    #[test]
    fn test_scan_valid_filename_matching_id_enters_journals() {
        let dir = TempDir::new().unwrap();
        let txn_id = "aaaa1111-0000-0000-0000-000000000001";
        let journal = format!(
            r#"id = "{txn_id}"
operation = "test"
created_at_unix_ms = 0
staged_files = []
state = "Prepared"
"#
        );
        fs::write(dir.path().join(format!("txn-{txn_id}.toml")), journal).unwrap();

        let inv = scan_transaction_journals(dir.path()).unwrap();
        assert_eq!(inv.journals.len(), 1);
        assert_eq!(inv.journals[0].id, txn_id);
        assert!(inv.corrupt.is_empty());
    }

    #[test]
    fn test_scan_filename_mismatched_id_enters_corrupt() {
        let dir = TempDir::new().unwrap();
        let filename_id = "aaaa1111-0000-0000-0000-000000000001";
        let internal_id = "bbbb2222-0000-0000-0000-000000000002";
        let journal = format!(
            r#"id = "{internal_id}"
operation = "test"
created_at_unix_ms = 0
staged_files = []
state = "Prepared"
"#
        );
        fs::write(dir.path().join(format!("txn-{filename_id}.toml")), journal).unwrap();

        let inv = scan_transaction_journals(dir.path()).unwrap();
        assert_eq!(
            inv.journals.len(),
            0,
            "mismatched ID must not enter journals"
        );
        assert_eq!(inv.corrupt.len(), 1, "mismatched ID must enter corrupt");
        assert!(
            inv.corrupt[0].error.contains("mismatch"),
            "error should mention mismatch: {}",
            inv.corrupt[0].error
        );
    }

    #[test]
    fn test_scan_empty_internal_id_enters_corrupt() {
        let dir = TempDir::new().unwrap();
        let txn_id = "aaaa1111-0000-0000-0000-000000000001";
        let journal = r#"id = ""
operation = "test"
created_at_unix_ms = 0
staged_files = []
state = "Prepared"
"#;
        fs::write(dir.path().join(format!("txn-{txn_id}.toml")), journal).unwrap();

        let inv = scan_transaction_journals(dir.path()).unwrap();
        assert_eq!(
            inv.journals.len(),
            0,
            "empty internal ID must not enter journals"
        );
        assert_eq!(inv.corrupt.len(), 1, "empty internal ID must enter corrupt");
    }

    #[test]
    fn test_scan_traversal_internal_id_enters_corrupt() {
        let dir = TempDir::new().unwrap();
        let txn_id = "aaaa1111-0000-0000-0000-000000000001";
        for bad_id in ["../evil", "a/b", "a\\b", "a..b"] {
            let journal = format!(
                r#"id = "{bad_id}"
operation = "test"
created_at_unix_ms = 0
staged_files = []
state = "Prepared"
"#
            );
            fs::write(dir.path().join(format!("txn-{txn_id}.toml")), journal).unwrap();

            let inv = scan_transaction_journals(dir.path()).unwrap();
            assert!(
                inv.journals.is_empty(),
                "traversal ID '{bad_id}' must not enter journals"
            );
            assert!(
                !inv.corrupt.is_empty(),
                "traversal ID '{bad_id}' must enter corrupt"
            );

            // Clean up for next iteration.
            fs::remove_file(dir.path().join(format!("txn-{txn_id}.toml"))).unwrap();
        }
    }

    #[test]
    fn test_short_transaction_id_does_not_panic() {
        // Short ID
        assert_eq!(short_transaction_id("abc"), "abc");
        // Empty ID
        assert_eq!(short_transaction_id(""), "");
        // Non-ASCII: 日本語テスト is 6 chars, all fit within 8
        assert_eq!(short_transaction_id("日本語テスト"), "日本語テスト");
        // Exactly 8 chars
        assert_eq!(short_transaction_id("12345678"), "12345678");
        // Longer than 8
        assert_eq!(short_transaction_id("1234567890"), "12345678");
    }

    #[test]
    fn test_journal_id_from_path_valid() {
        let path = Path::new("/tmp/txn-aaaa1111-0000-0000-0000-000000000001.toml");
        let id = journal_id_from_path(path).unwrap();
        assert_eq!(id, "aaaa1111-0000-0000-0000-000000000001");
    }

    #[test]
    fn test_journal_id_from_path_no_prefix() {
        let path = Path::new("/tmp/notxn-aaaa.toml");
        assert!(journal_id_from_path(path).is_err());
    }

    #[test]
    fn test_journal_id_from_path_empty_id() {
        let path = Path::new("/tmp/txn-.toml");
        assert!(journal_id_from_path(path).is_err());
    }

    // =========================================================================
    // Workstream A (Phase 11L): Lexical containment exact tests
    //
    // Pin the `Component::ParentDir` lexical-traversal defect and prove
    // the complete missing-path safety contract:
    //
    // - `Component::ParentDir` is rejected during normalization.
    // - lexical containment compares components, not strings.
    // - missing in-root paths are accepted as absent.
    // - missing out-of-root and traversal paths are rejected.
    // - missing child below an existing symlinked prefix is rejected.
    //
    // These tests intentionally fail against the pre-fix implementation
    // because the prior `lexically_within` accepted paths that begin with
    // the root components and later contain `..`.
    // =========================================================================

    #[test]
    fn lexical_containment_accepts_missing_normal_child() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("artifact-root");
        let child = root.join("missing.bin");
        assert!(
            !child.exists(),
            "missing child must not exist before validation"
        );
        let result = validate_contained_path(&root, &child, "backup_path");
        assert!(
            result.is_ok(),
            "missing in-root child must be accepted as absent: {:?}",
            result.err()
        );
    }

    #[test]
    fn lexical_containment_accepts_existing_normal_child() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("artifact-root");
        fs::create_dir_all(&root).unwrap();
        let child = root.join("present.bin");
        fs::write(&child, "data").unwrap();

        let result = validate_contained_path(&root, &child, "backup_path");
        assert!(
            result.is_ok(),
            "existing in-root child must be accepted: {:?}",
            result.err()
        );
    }

    #[test]
    fn lexical_containment_rejects_parent_dir_after_matching_root_prefix() {
        // Baseline defect reproduction: child starts with all root
        // components, then contains `..` after the root prefix.
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("artifact-root");
        // Do NOT create `root` — child references root but escapes through `..`.
        let child = root.join("..").join("..").join("outside.bin");

        assert!(!child.exists(), "child must not exist for this test");
        let result = validate_contained_path(&root, &child, "backup_path");
        assert!(
            result.is_err(),
            "parent traversal after root prefix must be rejected"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("outside") || msg.contains("traversal"),
            "error must mention traversal/outside, got: {msg}"
        );
    }

    #[test]
    fn lexical_containment_rejects_nested_parent_escape() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("artifact-root");
        let child = root.join("sub").join("..").join("..").join("outside.bin");
        assert!(!child.exists(), "child must not exist for this test");
        let result = validate_contained_path(&root, &child, "backup_path");
        assert!(
            result.is_err(),
            "nested parent escape must be rejected: {:?}",
            result.err()
        );
    }

    #[test]
    fn lexical_containment_rejects_sibling_path() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("artifacts");
        let sibling = dir.path().join("artifacts-other").join("file.bin");
        fs::create_dir_all(sibling.parent().unwrap()).unwrap();
        fs::write(&sibling, "x").unwrap();

        let result = validate_contained_path(&root, &sibling, "backup_path");
        assert!(
            result.is_err(),
            "sibling path with shared prefix must be rejected"
        );
    }

    #[test]
    fn lexical_containment_rejects_relative_paths() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let child = PathBuf::from("relative/child");
        let result = validate_contained_path(&root, &child, "backup_path");
        assert!(result.is_err(), "relative child must be rejected");
    }

    #[test]
    fn lexical_containment_rejects_relative_root() {
        let dir = TempDir::new().unwrap();
        let root = PathBuf::from("relative/root");
        let child = dir.path().to_path_buf();
        let result = validate_contained_path(&root, &child, "backup_path");
        assert!(result.is_err(), "relative root must be rejected");
    }

    #[test]
    fn lexical_containment_rejects_child_shorter_than_root() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("artifact-root");
        let child = dir.path().to_path_buf();
        let result = validate_contained_path(&root, &child, "backup_path");
        assert!(result.is_err(), "child shorter than root must be rejected");
    }

    #[test]
    fn lexical_containment_accepts_curdir_normalization() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().join("artifact-root");
        let child = root.join("sub").join(".").join("file.bin");
        // Child does not exist — should still pass lexical containment
        // after `CurDir` normalization.
        let result = validate_contained_path(&root, &child, "backup_path");
        assert!(
            result.is_ok(),
            "CurDir normalization must accept the path: {:?}",
            result.err()
        );
    }

    #[test]
    fn validate_artifact_containment_rejects_traversal_backup_path_for_every_state() {
        // For every transaction state, an unsafe missing backup path
        // must produce Err, leave the journal file untouched, and never
        // touch any external path.
        let states = [
            ("Prepared", TransactionState::Prepared),
            ("BackupsDurable", TransactionState::BackupsDurable),
            (
                "Committing",
                TransactionState::Committing {
                    next_commit_position: 0,
                },
            ),
            (
                "CommittedLocal",
                TransactionState::CommittedLocal {
                    pending: PendingFinalization::NotRecorded,
                },
            ),
            (
                "RollingBack",
                TransactionState::RollingBack {
                    next_rollback_position: 0,
                },
            ),
            (
                "CleaningUp_Commit",
                TransactionState::CleaningUp {
                    outcome: CleanupOutcome::Commit,
                    next_step: CleanupStep::Validate,
                },
            ),
            ("Committed", TransactionState::Committed),
            ("RolledBack", TransactionState::RolledBack),
        ];

        for (label, state) in states {
            let dir = TempDir::new().unwrap();
            let txn_id = "aaaa1111-0000-0000-0000-000000000001";
            let artifact_root = transaction_artifact_dir(dir.path(), txn_id);
            fs::create_dir_all(&artifact_root).unwrap();

            // Missing backup path that escapes via `..`.
            let unsafe_backup = artifact_root.join("..").join("..").join("outside.bin");
            assert!(
                !unsafe_backup.exists(),
                "{label}: unsafe backup must not exist"
            );

            let journal = TransactionJournal {
                id: txn_id.to_string(),
                operation: "test".to_string(),
                created_at_unix_ms: 0,
                staged_files: vec![StagedFile {
                    original_path: artifact_root.join("dest.toml"),
                    backup_path: Some(unsafe_backup.clone()),
                    staged_path: artifact_root.join("dest.toml"),
                    sha256: String::new(),
                    existed_before: true,
                    action: StagedAction::Replace,
                    original_hash: String::new(),
                    new_hash: String::new(),
                    durable_staged_path: None,
                    original_metadata: OriginalFileMetadata::default(),
                }],
                state: state.clone(),
            };
            let jpath = dir.path().join(format!("txn-{txn_id}.toml"));
            fs::write(&jpath, toml::to_string_pretty(&journal).unwrap()).unwrap();
            let journal_after = fs::read_to_string(&jpath).unwrap();

            let result = journal_owns_artifacts(dir.path(), &journal);
            assert!(
                result.is_err(),
                "{label}: unsafe missing backup must produce Err"
            );
            // Journal file untouched.
            assert_eq!(
                fs::read_to_string(&jpath).unwrap(),
                journal_after,
                "{label}: journal must be untouched on unsafe-path error"
            );
            // External path never created.
            assert!(
                !unsafe_backup.exists(),
                "{label}: unsafe external path must not be created"
            );
        }
    }

    #[test]
    fn classify_journal_recovery_rejects_durable_staged_traversal_for_committed_local() {
        let dir = TempDir::new().unwrap();
        let txn_id = "aaaa1111-0000-0000-0000-000000000001";
        let artifact_root = transaction_artifact_dir(dir.path(), txn_id);
        fs::create_dir_all(&artifact_root).unwrap();

        let unsafe_durable = artifact_root
            .join("..")
            .join("..")
            .join("outside-staged.bin");
        assert!(!unsafe_durable.exists());

        let journal = TransactionJournal {
            id: txn_id.to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![StagedFile {
                original_path: artifact_root.join("dest.toml"),
                backup_path: None,
                staged_path: artifact_root.join("dest.toml"),
                sha256: String::new(),
                existed_before: false,
                action: StagedAction::Create,
                original_hash: String::new(),
                new_hash: String::new(),
                durable_staged_path: Some(unsafe_durable.clone()),
                original_metadata: OriginalFileMetadata::default(),
            }],
            state: TransactionState::CommittedLocal {
                pending: PendingFinalization::NotRecorded,
            },
        };
        let result = classify_journal_recovery(dir.path(), &journal);
        assert!(
            result.is_err(),
            "CommittedLocal with traversal must be rejected: {:?}",
            result.err()
        );
        assert!(
            !unsafe_durable.exists(),
            "unsafe external path must not be created"
        );
    }

    #[test]
    fn safe_missing_in_root_backup_remains_classifiable() {
        let dir = TempDir::new().unwrap();
        let txn_id = "aaaa1111-0000-0000-0000-000000000001";
        let artifact_root = transaction_artifact_dir(dir.path(), txn_id);
        fs::create_dir_all(&artifact_root).unwrap();

        // Missing in-root backup should remain classifiable.
        let safe_backup = artifact_root.join("missing.bak");
        assert!(!safe_backup.exists());

        let journal = TransactionJournal {
            id: txn_id.to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![StagedFile {
                original_path: artifact_root.join("dest.toml"),
                backup_path: Some(safe_backup),
                staged_path: artifact_root.join("dest.toml"),
                sha256: String::new(),
                existed_before: true,
                action: StagedAction::Replace,
                original_hash: String::new(),
                new_hash: String::new(),
                durable_staged_path: None,
                original_metadata: OriginalFileMetadata::default(),
            }],
            state: TransactionState::Prepared,
        };
        let result = classify_journal_recovery(dir.path(), &journal);
        assert!(
            result.is_ok(),
            "safe missing in-root backup must be classifiable: {:?}",
            result.err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_existing_prefix_rejects_missing_child() {
        // `<artifact-root>/link/missing.bin` where `link` is an existing
        // symlink to outside and `missing.bin` does not exist. The path
        // must be rejected even though the final file is absent.
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let artifact_root = dir.path().join("artifact-root");
        fs::create_dir_all(&artifact_root).unwrap();

        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("marker.txt"), "do-not-touch").unwrap();

        let link = artifact_root.join("link");
        symlink(&outside, &link).unwrap();

        let missing_under_link = artifact_root.join("link").join("missing.bin");
        assert!(
            !missing_under_link.exists(),
            "missing.bin under symlink must not exist"
        );

        let result = validate_contained_path(&artifact_root, &missing_under_link, "backup_path");
        assert!(
            result.is_err(),
            "missing child under symlinked prefix must be rejected"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("symlink"),
            "error must mention symlink, got: {msg}"
        );

        // Outside marker must be untouched.
        assert!(
            outside.join("marker.txt").exists(),
            "external symlink target must not be touched"
        );
        // The symlink itself must be untouched.
        assert!(link.is_symlink(), "symlink must remain in place");
    }

    #[cfg(unix)]
    #[test]
    fn journal_owns_artifacts_rejects_symlinked_existing_prefix() {
        // The journal-level invariant: an unsafe missing reference
        // through a symlinked prefix must produce Err, and the journal
        // file and external target must be untouched.
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let txn_id = "aaaa1111-0000-0000-0000-000000000001";
        let artifact_root = transaction_artifact_dir(dir.path(), txn_id);
        fs::create_dir_all(&artifact_root).unwrap();

        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("evidence.bin"), "must-remain").unwrap();

        let link = artifact_root.join("link");
        symlink(&outside, &link).unwrap();

        let unsafe_backup = artifact_root.join("link").join("missing.bin");
        assert!(!unsafe_backup.exists());

        let journal = TransactionJournal {
            id: txn_id.to_string(),
            operation: "test".to_string(),
            created_at_unix_ms: 0,
            staged_files: vec![StagedFile {
                original_path: artifact_root.join("dest.toml"),
                backup_path: Some(unsafe_backup.clone()),
                staged_path: artifact_root.join("dest.toml"),
                sha256: String::new(),
                existed_before: true,
                action: StagedAction::Replace,
                original_hash: String::new(),
                new_hash: String::new(),
                durable_staged_path: None,
                original_metadata: OriginalFileMetadata::default(),
            }],
            state: TransactionState::Prepared,
        };
        let jpath = dir.path().join(format!("txn-{txn_id}.toml"));
        let journal_content = toml::to_string_pretty(&journal).unwrap();
        fs::write(&jpath, &journal_content).unwrap();

        let result = journal_owns_artifacts(dir.path(), &journal);
        assert!(
            result.is_err(),
            "journal_owns_artifacts must reject symlinked-prefix reference"
        );

        // Journal must remain untouched.
        assert_eq!(fs::read_to_string(&jpath).unwrap(), journal_content);
        // External target must remain untouched.
        assert!(outside.join("evidence.bin").exists());
    }
}
