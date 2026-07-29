//! **Layer: Application**
//!
//! `snp repair` command — conservative, backed-up, idempotent repair.
//!
//! Validates configuration and library files, identifies safe repair
//! candidates, and applies fixes only when explicitly requested.
//! Always creates a backup before any mutations.

use crate::error::{SnipError, SnipResult};
use crate::library::LibraryManager;
use std::fs;
use std::path::{Path, PathBuf};

/// Typed repair action categories for safe, structured repair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairAction {
    /// Prune orphaned usage entries (usage index entries for snippets that no longer exist).
    PruneOrphanedUsage,
    /// Roll back an interrupted transaction (Prepared, BackupsDurable, Committing, RollingBack).
    RollbackTransaction {
        /// Transaction ID to roll back.
        transaction_id: String,
    },
    /// Resume cleanup for a CleaningUp transaction.
    ResumeCleanup {
        /// Transaction ID to resume cleanup for.
        transaction_id: String,
    },
    /// Finalize a CommittedLocal transaction (complete pending, then cleanup).
    FinalizeCommittedLocal {
        /// Transaction ID to finalize.
        transaction_id: String,
    },
    /// Clean up a legacy Committed journal with artifacts.
    CleanupLegacyCommitted {
        /// Transaction ID.
        transaction_id: String,
    },
    /// Clean up a legacy RolledBack journal with artifacts.
    CleanupLegacyRolledBack {
        /// Transaction ID.
        transaction_id: String,
    },
    /// Remove a terminal journal with no artifacts.
    RemoveTerminalJournal {
        /// Transaction ID.
        transaction_id: String,
    },
    /// Remove an orphaned transaction artifact directory (no matching journal).
    RemoveOrphanedArtifact,
    /// Repair library index (duplicate entries, missing primary, etc.).
    RepairLibraryIndex,
    /// Repair snippet IDs (duplicates, missing IDs).
    RepairSnippetIds,
    /// Repair timestamps (missing, invalid).
    RepairTimestamps,
}

impl RepairAction {
    /// Short category string for display.
    pub fn category(&self) -> &'static str {
        match self {
            RepairAction::PruneOrphanedUsage => "usage",
            RepairAction::RollbackTransaction { .. } => "transaction",
            RepairAction::ResumeCleanup { .. } => "transaction",
            RepairAction::FinalizeCommittedLocal { .. } => "transaction",
            RepairAction::CleanupLegacyCommitted { .. } => "transaction",
            RepairAction::CleanupLegacyRolledBack { .. } => "transaction",
            RepairAction::RemoveTerminalJournal { .. } => "transaction",
            RepairAction::RemoveOrphanedArtifact => "transaction",
            RepairAction::RepairLibraryIndex => "index",
            RepairAction::RepairSnippetIds => "ids",
            RepairAction::RepairTimestamps => "timestamps",
        }
    }

    /// Whether this action is safe to apply automatically.
    pub fn is_safe(&self) -> bool {
        matches!(
            self,
            RepairAction::PruneOrphanedUsage
                | RepairAction::RollbackTransaction { .. }
                | RepairAction::ResumeCleanup { .. }
                | RepairAction::FinalizeCommittedLocal { .. }
                | RepairAction::CleanupLegacyCommitted { .. }
                | RepairAction::CleanupLegacyRolledBack { .. }
                | RepairAction::RemoveTerminalJournal { .. }
                | RepairAction::RemoveOrphanedArtifact
        )
    }

    /// Get the transaction ID for this action, if applicable.
    pub fn transaction_id(&self) -> Option<&str> {
        match self {
            RepairAction::RollbackTransaction { transaction_id }
            | RepairAction::ResumeCleanup { transaction_id }
            | RepairAction::FinalizeCommittedLocal { transaction_id }
            | RepairAction::CleanupLegacyCommitted { transaction_id }
            | RepairAction::CleanupLegacyRolledBack { transaction_id }
            | RepairAction::RemoveTerminalJournal { transaction_id } => Some(transaction_id),
            _ => None,
        }
    }
}

/// A single repair action identified during validation.
#[derive(Debug, Clone)]
pub struct RepairItem {
    /// Typed action category.
    pub action: RepairAction,
    /// Short category string (for display).
    pub category: String,
    /// Description of the problem found.
    pub problem: String,
    /// Proposed fix.
    pub fix: String,
    /// Whether this fix is safe to apply automatically.
    pub safe: bool,
    /// Target path for the repair action, if applicable.
    /// This replaces fragile string parsing of the problem description.
    pub target_path: Option<PathBuf>,
}

/// Exit status for the repair command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepairExitStatus {
    /// No issues found — nothing to repair.
    #[default]
    Clean,
    /// Issues found and all safe repairs applied successfully.
    Repaired,
    /// Issues found but some repairs failed.
    PartialFailure,
    /// Issues found but no safe repairs to apply (all unsafe).
    UnsafeOnly,
    /// Dry run completed — no changes made.
    DryRun,
}

/// Report emitted after repair analysis or application.
#[derive(Debug, Default)]
pub struct RepairReport {
    pub items: Vec<RepairItem>,
    pub backups: Vec<PathBuf>,
    pub applied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub exit_status: RepairExitStatus,
}

/// Run the repair command.
///
/// # Modes
///
/// - `dry_run=true`: Analyse and print planned repairs without changes.
/// - `apply=true`: Create pre-repair backup, apply safe repairs, emit report.
/// - Neither: Print validation summary only.
pub fn run(
    dry_run: bool,
    apply: bool,
    library: Option<String>,
    json: bool,
) -> SnipResult<RepairExitStatus> {
    let mut report = RepairReport::default();

    // Step 1: Validate and collect repair candidates
    collect_repair_candidates(&mut report, library.as_deref())?;

    // Step 2: Handle interrupted transactions
    collect_transaction_repairs(&mut report)?;

    // Step 3: Output report
    if json {
        emit_json_report(&report)?;
    } else {
        emit_human_report(&report);
    }

    // Step 4: Apply if requested
    if apply && !report.items.is_empty() {
        let safe_items: Vec<RepairItem> = report.items.iter().filter(|i| i.safe).cloned().collect();

        if safe_items.is_empty() {
            eprintln!("\nNo safe repairs to apply.");
            report.exit_status = RepairExitStatus::UnsafeOnly;
            return Ok(report.exit_status);
        }

        // Create backup before applying
        let backup_dir = create_repair_backup()?;
        report.backups.push(backup_dir);

        for item in &safe_items {
            match apply_repair(item) {
                Ok(()) => {
                    report.applied += 1;
                    eprintln!("  Applied: {} — {}", item.category, item.fix);
                }
                Err(e) => {
                    report.failed += 1;
                    eprintln!("  Failed:  {} — {} ({e})", item.category, item.fix);
                }
            }
        }

        // Count skipped (unsafe) items
        report.skipped = report.items.len() - safe_items.len();

        if report.failed > 0 {
            report.exit_status = RepairExitStatus::PartialFailure;
        } else {
            report.exit_status = RepairExitStatus::Repaired;
        }
    } else if report.items.is_empty() {
        report.exit_status = RepairExitStatus::Clean;
    } else if dry_run {
        report.exit_status = RepairExitStatus::DryRun;
    } else {
        report.exit_status = RepairExitStatus::UnsafeOnly;
    }

    if dry_run {
        eprintln!("\n(dry run — no changes made)");
    }

    Ok(report.exit_status)
}

/// Collect repair candidates from library validation.
fn collect_repair_candidates(report: &mut RepairReport, library: Option<&str>) -> SnipResult<()> {
    let mgr = match LibraryManager::new() {
        Ok(m) => m,
        Err(e) => {
            report.items.push(RepairItem {
                action: RepairAction::RepairLibraryIndex,
                category: "config".to_string(),
                problem: format!("Failed to load library manager: {e}"),
                fix: "Check ~/.config/snp/libraries.toml for corruption".to_string(),
                safe: false,
                target_path: None,
            });
            return Ok(());
        }
    };

    let libraries_dir = mgr.get_libraries_dir().clone();

    // If a specific library was requested, only check that one
    let library_files: Vec<PathBuf> = if let Some(name) = library {
        let path = libraries_dir.join(format!("{name}.toml"));
        if path.exists() {
            vec![path]
        } else {
            return Err(SnipError::runtime_error(
                "Library not found",
                Some(&format!("No library named '{name}' exists")),
            ));
        }
    } else {
        // Check all libraries
        if !libraries_dir.exists() {
            return Ok(());
        }
        fs::read_dir(&libraries_dir)
            .map_err(|e| SnipError::io_error("read libraries directory", libraries_dir.clone(), e))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
            .collect()
    };

    let mut all_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for lib_path in &library_files {
        match crate::library::load_library(lib_path) {
            Ok(snippets) => {
                // Check for empty IDs
                for (i, snippet) in snippets.snippets.iter().enumerate() {
                    if snippet.id.is_empty() {
                        report.items.push(RepairItem {
                            action: RepairAction::RepairSnippetIds,
                            category: "ids".to_string(),
                            problem: format!(
                                "Snippet {} in '{}' has empty ID",
                                i,
                                lib_path.file_stem().unwrap_or_default().to_string_lossy()
                            ),
                            fix: "Generate UUID for snippet".to_string(),
                            safe: true,
                            target_path: None,
                        });
                    } else if !all_ids.insert(snippet.id.clone()) {
                        report.items.push(RepairItem {
                            action: RepairAction::RepairSnippetIds,
                            category: "ids".to_string(),
                            problem: format!(
                                "Duplicate ID '{}' in '{}'",
                                snippet.id,
                                lib_path.file_stem().unwrap_or_default().to_string_lossy()
                            ),
                            fix: "Regenerate duplicate ID".to_string(),
                            safe: true,
                            target_path: None,
                        });
                    }
                }

                // Check for missing timestamps
                for (i, snippet) in snippets.snippets.iter().enumerate() {
                    if snippet.created_at == 0 || snippet.updated_at == 0 {
                        report.items.push(RepairItem {
                            action: RepairAction::RepairTimestamps,
                            category: "timestamps".to_string(),
                            problem: format!(
                                "Snippet {} ('{}') in '{}' has zero timestamp",
                                i,
                                snippet.description,
                                lib_path.file_stem().unwrap_or_default().to_string_lossy()
                            ),
                            fix: "Set timestamps to current time".to_string(),
                            safe: true,
                            target_path: None,
                        });
                    }
                }
            }
            Err(e) => {
                report.items.push(RepairItem {
                    action: RepairAction::RepairLibraryIndex,
                    category: "config".to_string(),
                    problem: format!(
                        "Failed to load '{}': {e}",
                        lib_path.file_stem().unwrap_or_default().to_string_lossy()
                    ),
                    fix: "Check file for TOML syntax errors".to_string(),
                    safe: false,
                    target_path: None,
                });
            }
        }
    }

    // Check primary library selection
    match mgr.get_primary_library() {
        Some(primary) => {
            let primary_path = libraries_dir.join(format!("{}.toml", primary.filename));
            if !primary_path.exists() {
                report.items.push(RepairItem {
                    action: RepairAction::RepairLibraryIndex,
                    category: "primary".to_string(),
                    problem: format!(
                        "Primary library '{}' references missing file",
                        primary.filename
                    ),
                    fix: "Promote first available library to primary".to_string(),
                    safe: true,
                    target_path: None,
                });
            }
        }
        None => {
            // No primary set — check if we can auto-assign
            let libs = mgr.list_libraries();
            if libs.len() == 1 {
                report.items.push(RepairItem {
                    action: RepairAction::RepairLibraryIndex,
                    category: "primary".to_string(),
                    problem: "No primary library is set (only one library exists)".to_string(),
                    fix: format!("Set '{}' as primary", libs[0].filename),
                    safe: true,
                    target_path: None,
                });
            } else if !libs.is_empty() {
                report.items.push(RepairItem {
                    action: RepairAction::RepairLibraryIndex,
                    category: "primary".to_string(),
                    problem: "No primary library is set".to_string(),
                    fix: "Run 'snp library set-primary <name>' to choose one".to_string(),
                    safe: false,
                    target_path: None,
                });
            }
        }
    }

    // Check for orphaned usage entries
    let usage_index = crate::usage::UsageIndex::load();
    let mut active_ids: Vec<String> = Vec::new();
    for lib_path in &library_files {
        if let Ok(snippets) = crate::library::load_library(lib_path) {
            for snippet in &snippets.snippets {
                active_ids.push(snippet.id.clone());
            }
        }
    }

    let mut orphaned_count = 0;
    for entry in usage_index.entries() {
        if !active_ids.contains(&entry.id) {
            orphaned_count += 1;
        }
    }
    if orphaned_count > 0 {
        report.items.push(RepairItem {
            action: RepairAction::PruneOrphanedUsage,
            category: "usage".to_string(),
            problem: format!("{orphaned_count} orphaned usage entries (snippets no longer exist)"),
            fix: "Remove orphaned usage entries".to_string(),
            safe: true,
            target_path: None,
        });
    }

    Ok(())
}

/// Collect repair candidates from transaction journals using the complete
/// scanner and classifier. This discovers every transaction that needs
/// attention, including legacy terminal journals with artifacts.
fn collect_transaction_repairs(report: &mut RepairReport) -> SnipResult<()> {
    let state_dir = crate::auto_sync::notification::derive_state_dir().join(".transaction");
    let inventory = crate::transaction::scan_transaction_journals(&state_dir)?;

    // Fail closed on corrupt journals.
    for corrupt in &inventory.corrupt {
        report.items.push(RepairItem {
            action: RepairAction::RollbackTransaction {
                transaction_id: corrupt.path.display().to_string(),
            },
            category: "unsafe".to_string(),
            problem: format!(
                "Corrupt transaction journal '{}': {}",
                corrupt.path.display(),
                corrupt.error
            ),
            fix: "Requires manual quarantine or deletion".to_string(),
            safe: false,
            target_path: Some(corrupt.path.clone()),
        });
    }

    for journal in &inventory.journals {
        let recovery_class = match crate::transaction::classify_journal_recovery(&state_dir, journal) {
            Ok(class) => class,
            Err(e) => {
                // Unsafe artifact inspection failure — report as unsafe/manual.
                report.items.push(RepairItem {
                    action: RepairAction::RollbackTransaction {
                        transaction_id: journal.id.clone(),
                    },
                    category: "unsafe".to_string(),
                    problem: format!(
                        "Transaction '{}' (op: {}) has unsafe artifacts: {e}",
                        &journal.id[..8.min(journal.id.len())],
                        journal.operation,
                    ),
                    fix: "Requires manual investigation — preserve journal and artifacts"
                        .to_string(),
                    safe: false,
                    target_path: None,
                });
                continue;
            }
        };

        let action = match recovery_class {
            crate::transaction::RecoveryClass::Rollback => RepairAction::RollbackTransaction {
                transaction_id: journal.id.clone(),
            },
            crate::transaction::RecoveryClass::FinalizeCommittedLocal => {
                RepairAction::FinalizeCommittedLocal {
                    transaction_id: journal.id.clone(),
                }
            }
            crate::transaction::RecoveryClass::ResumeCleanup => RepairAction::ResumeCleanup {
                transaction_id: journal.id.clone(),
            },
            crate::transaction::RecoveryClass::CleanupLegacyCommitted => {
                RepairAction::CleanupLegacyCommitted {
                    transaction_id: journal.id.clone(),
                }
            }
            crate::transaction::RecoveryClass::CleanupLegacyRolledBack => {
                RepairAction::CleanupLegacyRolledBack {
                    transaction_id: journal.id.clone(),
                }
            }
            crate::transaction::RecoveryClass::RemoveTerminalJournal => {
                RepairAction::RemoveTerminalJournal {
                    transaction_id: journal.id.clone(),
                }
            }
            crate::transaction::RecoveryClass::UnsafeFailed => {
                report.items.push(RepairItem {
                    action: RepairAction::RollbackTransaction {
                        transaction_id: journal.id.clone(),
                    },
                    category: "unsafe".to_string(),
                    problem: format!(
                        "Transaction '{}' (op: {}) is in a Failed state",
                        &journal.id[..8],
                        journal.operation,
                    ),
                    fix: "Requires manual investigation — preserve journal and artifacts"
                        .to_string(),
                    safe: false,
                    target_path: None,
                });
                continue;
            }
        };

        let fix = match &action {
            RepairAction::FinalizeCommittedLocal { .. } => {
                "Finalize committed-local transaction (complete pending, then clean up)"
            }
            RepairAction::ResumeCleanup { .. } => "Resume interrupted cleanup",
            RepairAction::RollbackTransaction { .. } => "Roll back interrupted transaction",
            RepairAction::CleanupLegacyCommitted { .. } => "Clean up legacy committed journal",
            RepairAction::CleanupLegacyRolledBack { .. } => "Clean up legacy rolled-back journal",
            RepairAction::RemoveTerminalJournal { .. } => "Remove terminal journal (no artifacts)",
            _ => "Repair transaction",
        };

        report.items.push(RepairItem {
            action,
            category: "transaction".to_string(),
            problem: format!(
                "Transaction '{}' (op: {}, state: {:?})",
                &journal.id[..8],
                journal.operation,
                journal.state
            ),
            fix: fix.to_string(),
            safe: true,
            target_path: None,
        });
    }

    // Scan for orphaned artifact directories — directories under
    // `artifacts/` that have no corresponding journal file.
    // This catches stale artifacts left behind by crashes during cleanup.
    collect_orphan_artifact_repairs(report, &state_dir)?;

    Ok(())
}

/// Scan the transaction artifact root for orphaned directories that have
/// no corresponding journal file. These waste disk space and may contain
/// sensitive staged content.
fn collect_orphan_artifact_repairs(report: &mut RepairReport, state_dir: &Path) -> SnipResult<()> {
    let artifacts_root = state_dir.join("artifacts");
    if !artifacts_root.exists() {
        return Ok(());
    }

    // Collect journal IDs from existing journal files.
    let mut journal_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    if state_dir.exists() {
        for entry in fs::read_dir(state_dir).map_err(|e| {
            SnipError::io_error(
                "read transaction state directory",
                state_dir.to_path_buf(),
                e,
            )
        })? {
            let entry = entry.map_err(|e| {
                SnipError::io_error("read transaction state entry", state_dir.to_path_buf(), e)
            })?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml")
                && path
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy().starts_with("txn-"))
            {
                // Extract the UUID from `txn-<uuid>.toml`.
                if let Some(stem) = path.file_stem() {
                    let name = stem.to_string_lossy();
                    if let Some(uuid) = name.strip_prefix("txn-") {
                        journal_ids.insert(uuid.to_string());
                    }
                }
            }
        }
    }

    // Check each artifact directory for a matching journal.
    for entry in fs::read_dir(&artifacts_root)
        .map_err(|e| SnipError::io_error("read artifacts directory", artifacts_root.clone(), e))?
    {
        let entry = entry
            .map_err(|e| SnipError::io_error("read artifacts entry", artifacts_root.clone(), e))?;
        let path = entry.path();
        if path.is_dir()
            && let Some(dir_name) = path.file_stem()
        {
            let id = dir_name.to_string_lossy().to_string();
            if !journal_ids.contains(&id) {
                report.items.push(RepairItem {
                    action: RepairAction::RemoveOrphanedArtifact,
                    category: "transaction".to_string(),
                    problem: format!(
                        "Orphaned transaction artifact directory '{}'",
                        path.display()
                    ),
                    fix: "Remove orphaned artifact directory".to_string(),
                    safe: true,
                    target_path: Some(path.clone()),
                });
            }
        }
    }

    Ok(())
}

/// Apply a single safe repair.
fn apply_repair(item: &RepairItem) -> SnipResult<()> {
    match &item.action {
        RepairAction::PruneOrphanedUsage => {
            // Prune orphaned usage entries
            let mut usage_index = crate::usage::UsageIndex::load();
            let active_ids = collect_active_snippet_ids();
            usage_index.prune(&active_ids);
            usage_index.save()?;
        }
        RepairAction::RemoveOrphanedArtifact => {
            // Safe orphan deletion: use target_path (not string parsing),
            // validate containment, and reject symlinks.
            if let Some(ref path) = item.target_path {
                // Validate the path is within the transaction state directory.
                let state_dir =
                    crate::auto_sync::notification::derive_state_dir().join(".transaction");
                let artifacts_root = state_dir.join("artifacts");
                let canonical_root = artifacts_root
                    .canonicalize()
                    .unwrap_or(artifacts_root.clone());
                let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone());

                if !canonical_path.starts_with(&canonical_root) {
                    return Err(SnipError::runtime_error(
                        "Orphaned artifact path traversal",
                        Some(&format!(
                            "Artifact directory {} is outside the transaction artifacts root {}",
                            path.display(),
                            canonical_root.display()
                        )),
                    ));
                }

                // Reject symlinks.
                if path.is_symlink() {
                    return Err(SnipError::runtime_error(
                        "Orphaned artifact is a symlink",
                        Some(&format!(
                            "Refusing to remove symlinked artifact directory: {}",
                            path.display()
                        )),
                    ));
                }

                if path.exists() {
                    fs::remove_dir_all(path).map_err(|e| {
                        SnipError::io_error(
                            "remove orphaned artifact directory",
                            path.to_path_buf(),
                            e,
                        )
                    })?;
                }
            }
        }
        RepairAction::RollbackTransaction { transaction_id } => {
            let state_dir = crate::auto_sync::notification::derive_state_dir().join(".transaction");
            let sync_state_dir = crate::auto_sync::notification::derive_state_dir();
            crate::transaction::recover_transaction_by_id(
                &sync_state_dir,
                &state_dir,
                transaction_id,
                crate::transaction::RecoveryClass::Rollback,
            )
            .map_err(|e| {
                SnipError::runtime_error(
                    "rollback transaction",
                    Some(&format!(
                        "Failed to rollback transaction '{transaction_id}': {e}"
                    )),
                )
            })?;
        }
        RepairAction::ResumeCleanup { transaction_id } => {
            let state_dir = crate::auto_sync::notification::derive_state_dir().join(".transaction");
            let sync_state_dir = crate::auto_sync::notification::derive_state_dir();
            crate::transaction::recover_transaction_by_id(
                &sync_state_dir,
                &state_dir,
                transaction_id,
                crate::transaction::RecoveryClass::ResumeCleanup,
            )
            .map_err(|e| {
                SnipError::runtime_error(
                    "resume cleanup",
                    Some(&format!(
                        "Failed to resume cleanup for transaction '{transaction_id}': {e}"
                    )),
                )
            })?;
        }
        RepairAction::FinalizeCommittedLocal { transaction_id } => {
            let state_dir = crate::auto_sync::notification::derive_state_dir().join(".transaction");
            let sync_state_dir = crate::auto_sync::notification::derive_state_dir();
            crate::transaction::recover_transaction_by_id(
                &sync_state_dir,
                &state_dir,
                transaction_id,
                crate::transaction::RecoveryClass::FinalizeCommittedLocal,
            )
            .map_err(|e| {
                SnipError::runtime_error(
                    "finalize committed-local",
                    Some(&format!(
                        "Failed to finalize transaction '{transaction_id}': {e}"
                    )),
                )
            })?;
        }
        RepairAction::CleanupLegacyCommitted { transaction_id } => {
            let state_dir = crate::auto_sync::notification::derive_state_dir().join(".transaction");
            let sync_state_dir = crate::auto_sync::notification::derive_state_dir();
            crate::transaction::recover_transaction_by_id(
                &sync_state_dir,
                &state_dir,
                transaction_id,
                crate::transaction::RecoveryClass::CleanupLegacyCommitted,
            )
            .map_err(|e| {
                SnipError::runtime_error(
                    "cleanup legacy committed",
                    Some(&format!(
                        "Failed to cleanup legacy committed transaction '{transaction_id}': {e}"
                    )),
                )
            })?;
        }
        RepairAction::CleanupLegacyRolledBack { transaction_id } => {
            let state_dir = crate::auto_sync::notification::derive_state_dir().join(".transaction");
            let sync_state_dir = crate::auto_sync::notification::derive_state_dir();
            crate::transaction::recover_transaction_by_id(
                &sync_state_dir,
                &state_dir,
                transaction_id,
                crate::transaction::RecoveryClass::CleanupLegacyRolledBack,
            )
            .map_err(|e| {
                SnipError::runtime_error(
                    "cleanup legacy rolled-back",
                    Some(&format!(
                        "Failed to cleanup legacy rolled-back transaction '{transaction_id}': {e}"
                    )),
                )
            })?;
        }
        RepairAction::RemoveTerminalJournal { transaction_id } => {
            let state_dir = crate::auto_sync::notification::derive_state_dir().join(".transaction");
            let sync_state_dir = crate::auto_sync::notification::derive_state_dir();
            crate::transaction::recover_transaction_by_id(
                &sync_state_dir,
                &state_dir,
                transaction_id,
                crate::transaction::RecoveryClass::RemoveTerminalJournal,
            )
            .map_err(|e| {
                SnipError::runtime_error(
                    "remove terminal journal",
                    Some(&format!(
                        "Failed to remove terminal journal '{transaction_id}': {e}"
                    )),
                )
            })?;
        }
        RepairAction::RepairLibraryIndex
        | RepairAction::RepairSnippetIds
        | RepairAction::RepairTimestamps => {
            // These require library file mutations — not safe for auto-apply
            // without the full library context. Return a descriptive error.
            return Err(SnipError::runtime_error(
                "Auto-repair not implemented for this category",
                Some(&format!(
                    "Action {:?} requires manual intervention or full library context",
                    item.action
                )),
            ));
        }
    }
    Ok(())
}

/// Collect all active snippet IDs across all libraries.
fn collect_active_snippet_ids() -> Vec<String> {
    let mut ids = Vec::new();
    let mgr = match LibraryManager::new() {
        Ok(m) => m,
        Err(_) => return ids,
    };
    let libraries_dir = mgr.get_libraries_dir().clone();
    if !libraries_dir.exists() {
        return ids;
    }
    if let Ok(entries) = fs::read_dir(&libraries_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml")
                && let Ok(snippets) = crate::library::load_library(&path)
            {
                for snippet in &snippets.snippets {
                    ids.push(snippet.id.clone());
                }
            }
        }
    }
    ids
}

/// Create a timestamped backup of the entire config directory for repair.
fn create_repair_backup() -> SnipResult<PathBuf> {
    let config_dir = crate::utils::config::get_config_dir();
    let backup_root = config_dir.join("backups");
    fs::create_dir_all(&backup_root)
        .map_err(|e| SnipError::io_error("create backup directory", backup_root.clone(), e))?;

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let backup_dir = backup_root.join(format!("repair-{timestamp}-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&backup_dir).map_err(|e| {
        SnipError::io_error("create repair backup directory", backup_dir.clone(), e)
    })?;

    // Copy libraries directory
    let libraries_dir = config_dir.join("libraries");
    if libraries_dir.exists() {
        let dest = backup_dir.join("libraries");
        copy_dir_recursive(&libraries_dir, &dest)?;
    }

    // Copy libraries.toml
    let config_file = config_dir.join("libraries.toml");
    if config_file.exists() {
        let _ = fs::copy(&config_file, backup_dir.join("libraries.toml"));
    }

    // Copy usage.toml
    let usage_file = config_dir.join("usage.toml");
    if usage_file.exists() {
        let _ = fs::copy(&usage_file, backup_dir.join("usage.toml"));
    }

    Ok(backup_dir)
}

/// Recursively copy a directory.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> SnipResult<()> {
    fs::create_dir_all(dst)
        .map_err(|e| SnipError::io_error("create backup subdirectory", dst.to_path_buf(), e))?;
    for entry in fs::read_dir(src)
        .map_err(|e| SnipError::io_error("read source directory", src.to_path_buf(), e))?
    {
        let entry =
            entry.map_err(|e| SnipError::io_error("read directory entry", src.to_path_buf(), e))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)
                .map_err(|e| SnipError::io_error("copy file for backup", src_path.clone(), e))?;
        }
    }
    Ok(())
}

/// Emit the repair report in human-readable format.
fn emit_human_report(report: &RepairReport) {
    eprintln!();
    eprintln!("Repair Report");
    eprintln!("=============");

    if report.items.is_empty() {
        eprintln!("No issues found. All good!");
        return;
    }

    let safe_count = report.items.iter().filter(|i| i.safe).count();
    let unsafe_count = report.items.len() - safe_count;

    eprintln!(
        "\nFound {} issue(s) ({} safe, {} require manual review):",
        report.items.len(),
        safe_count,
        unsafe_count
    );

    for (i, item) in report.items.iter().enumerate() {
        let marker = if item.safe { "auto" } else { "manual" };
        eprintln!(
            "\n  {}. [{}] {} — {}",
            i + 1,
            marker,
            item.category,
            item.problem
        );
        eprintln!("     Fix: {}", item.fix);
    }

    if !report.backups.is_empty() {
        eprintln!("\nBackups:");
        for backup in &report.backups {
            eprintln!("  {}", backup.display());
        }
    }

    if report.applied > 0 || report.skipped > 0 || report.failed > 0 {
        eprintln!("\nResults:");
        if report.applied > 0 {
            eprintln!("  Applied:  {}", report.applied);
        }
        if report.skipped > 0 {
            eprintln!(
                "  Skipped:  {} (unsafe, requires manual fix)",
                report.skipped
            );
        }
        if report.failed > 0 {
            eprintln!("  Failed:   {}", report.failed);
        }
    }
}

/// Emit the repair report in JSON format.
fn emit_json_report(report: &RepairReport) -> SnipResult<()> {
    #[derive(serde::Serialize)]
    struct JsonRepairItem {
        action: String,
        category: String,
        transaction_id: Option<String>,
        problem: String,
        fix: String,
        safe: bool,
    }

    #[derive(serde::Serialize)]
    struct JsonReport {
        items: Vec<JsonRepairItem>,
        backups: Vec<String>,
        applied: usize,
        skipped: usize,
        failed: usize,
        exit_classification: String,
    }

    let exit_classification = match report.exit_status {
        RepairExitStatus::Clean => "clean",
        RepairExitStatus::Repaired => "repaired",
        RepairExitStatus::PartialFailure => "partial_failure",
        RepairExitStatus::UnsafeOnly => "unsafe_only",
        RepairExitStatus::DryRun => "dry_run",
    };

    let json = JsonReport {
        items: report
            .items
            .iter()
            .map(|i| JsonRepairItem {
                action: format!("{:?}", i.action),
                category: i.category.clone(),
                transaction_id: i.action.transaction_id().map(String::from),
                problem: i.problem.clone(),
                fix: i.fix.clone(),
                safe: i.safe,
            })
            .collect(),
        backups: report
            .backups
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
        applied: report.applied,
        skipped: report.skipped,
        failed: report.failed,
        exit_classification: exit_classification.to_string(),
    };

    let output = serde_json::to_string_pretty(&json)
        .map_err(|e| SnipError::runtime_error("serialize repair report", Some(&e.to_string())))?;
    println!("{output}");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_repair_report_default() {
        let report = RepairReport::default();
        assert!(report.items.is_empty());
        assert_eq!(report.applied, 0);
    }

    #[test]
    fn test_repair_item_creation() {
        let item = RepairItem {
            action: RepairAction::PruneOrphanedUsage,
            category: "usage".to_string(),
            problem: "orphaned entries".to_string(),
            fix: "prune".to_string(),
            safe: true,
            target_path: None,
        };
        assert!(item.safe);
        assert_eq!(item.category, "usage");
        assert_eq!(item.action, RepairAction::PruneOrphanedUsage);
    }

    #[test]
    fn test_copy_dir_recursive() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        // Create source structure
        fs::write(src.path().join("file.txt"), "hello").unwrap();
        fs::create_dir(src.path().join("sub")).unwrap();
        fs::write(src.path().join("sub").join("nested.txt"), "world").unwrap();

        let dest = dst.path().join("copy");
        copy_dir_recursive(src.path(), &dest).unwrap();

        assert!(dest.join("file.txt").exists());
        assert!(dest.join("sub").join("nested.txt").exists());
        assert_eq!(fs::read_to_string(dest.join("file.txt")).unwrap(), "hello");
    }
}
