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
use std::path::PathBuf;

/// A single repair action identified during validation.
#[derive(Debug, Clone)]
pub struct RepairItem {
    /// Short category (e.g. "index", "primary", "usage", "ids", "transaction").
    pub category: String,
    /// Description of the problem found.
    pub problem: String,
    /// Proposed fix.
    pub fix: String,
    /// Whether this fix is safe to apply automatically.
    pub safe: bool,
}

/// Report emitted after repair analysis or application.
#[derive(Debug, Default)]
pub struct RepairReport {
    pub items: Vec<RepairItem>,
    pub backups: Vec<PathBuf>,
    pub applied: usize,
    pub skipped: usize,
    pub failed: usize,
}

/// Run the repair command.
///
/// # Modes
///
/// - `dry_run=true`: Analyse and print planned repairs without changes.
/// - `apply=true`: Create pre-repair backup, apply safe repairs, emit report.
/// - Neither: Print validation summary only.
pub fn run(dry_run: bool, apply: bool, library: Option<String>, json: bool) -> SnipResult<()> {
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
            return Ok(());
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
    }

    if dry_run {
        eprintln!("\n(dry run — no changes made)");
    }

    Ok(())
}

/// Collect repair candidates from library validation.
fn collect_repair_candidates(report: &mut RepairReport, library: Option<&str>) -> SnipResult<()> {
    let mgr = match LibraryManager::new() {
        Ok(m) => m,
        Err(e) => {
            report.items.push(RepairItem {
                category: "config".to_string(),
                problem: format!("Failed to load library manager: {e}"),
                fix: "Check ~/.config/snp/libraries.toml for corruption".to_string(),
                safe: false,
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
                            category: "ids".to_string(),
                            problem: format!(
                                "Snippet {} in '{}' has empty ID",
                                i,
                                lib_path.file_stem().unwrap_or_default().to_string_lossy()
                            ),
                            fix: "Generate UUID for snippet".to_string(),
                            safe: true,
                        });
                    } else if !all_ids.insert(snippet.id.clone()) {
                        report.items.push(RepairItem {
                            category: "ids".to_string(),
                            problem: format!(
                                "Duplicate ID '{}' in '{}'",
                                snippet.id,
                                lib_path.file_stem().unwrap_or_default().to_string_lossy()
                            ),
                            fix: "Regenerate duplicate ID".to_string(),
                            safe: true,
                        });
                    }
                }

                // Check for missing timestamps
                for (i, snippet) in snippets.snippets.iter().enumerate() {
                    if snippet.created_at == 0 || snippet.updated_at == 0 {
                        report.items.push(RepairItem {
                            category: "timestamps".to_string(),
                            problem: format!(
                                "Snippet {} ('{}') in '{}' has zero timestamp",
                                i,
                                snippet.description,
                                lib_path.file_stem().unwrap_or_default().to_string_lossy()
                            ),
                            fix: "Set timestamps to current time".to_string(),
                            safe: true,
                        });
                    }
                }
            }
            Err(e) => {
                report.items.push(RepairItem {
                    category: "config".to_string(),
                    problem: format!(
                        "Failed to load '{}': {e}",
                        lib_path.file_stem().unwrap_or_default().to_string_lossy()
                    ),
                    fix: "Check file for TOML syntax errors".to_string(),
                    safe: false,
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
                    category: "primary".to_string(),
                    problem: format!(
                        "Primary library '{}' references missing file",
                        primary.filename
                    ),
                    fix: "Promote first available library to primary".to_string(),
                    safe: true,
                });
            }
        }
        None => {
            // No primary set — check if we can auto-assign
            let libs = mgr.list_libraries();
            if libs.len() == 1 {
                report.items.push(RepairItem {
                    category: "primary".to_string(),
                    problem: "No primary library is set (only one library exists)".to_string(),
                    fix: format!("Set '{}' as primary", libs[0].filename),
                    safe: true,
                });
            } else if !libs.is_empty() {
                report.items.push(RepairItem {
                    category: "primary".to_string(),
                    problem: "No primary library is set".to_string(),
                    fix: "Run 'snp library set-primary <name>' to choose one".to_string(),
                    safe: false,
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
            category: "usage".to_string(),
            problem: format!("{orphaned_count} orphaned usage entries (snippets no longer exist)"),
            fix: "Remove orphaned usage entries".to_string(),
            safe: true,
        });
    }

    Ok(())
}

/// Collect repair candidates from interrupted transactions.
fn collect_transaction_repairs(report: &mut RepairReport) -> SnipResult<()> {
    let state_dir = crate::auto_sync::notification::derive_state_dir();
    let interrupted = crate::transaction::check_interrupted_transactions(&state_dir)?;

    for journal in &interrupted {
        report.items.push(RepairItem {
            category: "transaction".to_string(),
            problem: format!(
                "Interrupted transaction '{}' (op: {})",
                &journal.id[..8],
                journal.operation
            ),
            fix: "Roll back interrupted transaction".to_string(),
            safe: true,
        });
    }

    Ok(())
}

/// Apply a single safe repair.
fn apply_repair(item: &RepairItem) -> SnipResult<()> {
    match item.category.as_str() {
        "usage" => {
            // Prune orphaned usage entries
            let mut usage_index = crate::usage::UsageIndex::load();
            let active_ids = collect_active_snippet_ids();
            usage_index.prune(&active_ids);
            usage_index.save()?;
        }
        "transaction" => {
            // Roll back interrupted transactions
            let state_dir = crate::auto_sync::notification::derive_state_dir().join(".transaction");
            let interrupted = crate::transaction::check_interrupted_transactions(&state_dir)?;
            for journal in &interrupted {
                if let Err(e) = crate::transaction::rollback_transaction(&state_dir, journal) {
                    tracing::warn!(
                        txn_id = %journal.id,
                        error = %e,
                        "Failed to rollback interrupted transaction"
                    );
                }
            }
        }
        "ids" | "timestamps" | "primary" => {
            // These require library file mutations — not safe for auto-apply
            // without the full library context. Return a descriptive error.
            return Err(SnipError::runtime_error(
                "Auto-repair not implemented for this category",
                Some(&format!(
                    "Category '{}' requires manual intervention or full library context",
                    item.category
                )),
            ));
        }
        _ => {
            return Err(SnipError::runtime_error(
                "Unknown repair category",
                Some(&item.category),
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
    let backup_dir = backup_root.join(format!("repair-{timestamp}"));
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
    struct JsonRepairItem<'a> {
        category: &'a str,
        problem: &'a str,
        fix: &'a str,
        safe: bool,
    }

    #[derive(serde::Serialize)]
    struct JsonReport<'a> {
        items: Vec<JsonRepairItem<'a>>,
        backups: Vec<String>,
        applied: usize,
        skipped: usize,
        failed: usize,
    }

    let json = JsonReport {
        items: report
            .items
            .iter()
            .map(|i| JsonRepairItem {
                category: &i.category,
                problem: &i.problem,
                fix: &i.fix,
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
            category: "usage".to_string(),
            problem: "orphaned entries".to_string(),
            fix: "prune".to_string(),
            safe: true,
        };
        assert!(item.safe);
        assert_eq!(item.category, "usage");
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
