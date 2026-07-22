//! **Layer: Application**
//!
//! `snp validate` command — comprehensive read-only validation of snippet data.

use crate::error::{SnipError, SnipResult};
use crate::library::{LibraryManager, Snippet, Snippets};
use crate::usage::UsageIndex;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Severity level for validation diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// Repairability classification for a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Repairability {
    Auto,
    Manual,
    Unrepairable,
}

/// A single validation diagnostic.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationDiagnostic {
    pub code: String,
    pub severity: Severity,
    pub path: Option<PathBuf>,
    pub library: Option<String>,
    pub snippet_id: Option<String>,
    pub message: String,
    pub repairability: Repairability,
}

/// Complete validation report.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationReport {
    pub schema_version: String,
    pub tool_version: String,
    pub strict_mode: bool,
    pub dry_run: bool,
    pub total_libraries: usize,
    pub total_snippets: usize,
    pub diagnostics: Vec<ValidationDiagnostic>,
}

impl ValidationReport {
    pub fn new(strict_mode: bool) -> Self {
        Self {
            schema_version: "1.0.0".to_string(),
            tool_version: crate::diagnostics::version().to_string(),
            strict_mode,
            dry_run: true,
            total_libraries: 0,
            total_snippets: 0,
            diagnostics: Vec::new(),
        }
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    pub fn info_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Info)
            .count()
    }
}

fn diags(
    report: &mut ValidationReport,
    lib_name: &str,
    lib_path: Option<&PathBuf>,
    snippet_id: Option<&str>,
    code: &str,
    severity: Severity,
    repairability: Repairability,
    message: String,
) {
    report.diagnostics.push(ValidationDiagnostic {
        code: code.to_string(),
        severity,
        path: lib_path.cloned(),
        library: Some(lib_name.to_string()),
        snippet_id: snippet_id.map(String::from),
        message,
        repairability,
    });
}

/// Validate a single library, appending diagnostics to the report.
fn validate_library(
    report: &mut ValidationReport,
    lib_name: &str,
    lib_path: &PathBuf,
    strict: bool,
) {
    let lib_display = lib_name.to_string();

    // b. Try loading the TOML file via the standard load path (which creates
    //    backups of corrupt files internally). We re-read the raw content to
    //    also check for structural issues that `load_library` silently fixes.
    let raw_content = match std::fs::read_to_string(lib_path) {
        Ok(c) => c,
        Err(e) => {
            diags(
                report,
                &lib_display,
                Some(lib_path),
                None,
                "E-FILE-READ",
                Severity::Error,
                Repairability::Manual,
                format!("Failed to read library file: {e}"),
            );
            return;
        }
    };

    if raw_content.trim().is_empty() {
        diags(
            report,
            &lib_display,
            Some(lib_path),
            None,
            "I-FILE-EMPTY",
            Severity::Info,
            Repairability::Auto,
            "Library file is empty (no snippets)".to_string(),
        );
        return;
    }

    // Check for TOML parse errors via the raw parser
    let snippets: Snippets = match toml::from_str(&raw_content) {
        Ok(s) => s,
        Err(e) => {
            diags(
                report,
                &lib_display,
                Some(lib_path),
                None,
                "E-TOML-PARSE",
                Severity::Error,
                Repairability::Manual,
                format!("TOML parse error: {e}"),
            );
            return;
        }
    };

    let active_snippets: Vec<&Snippet> = snippets.snippets.iter().collect();
    report.total_snippets += active_snippets.len();

    // c. Duplicate snippet IDs
    let mut id_counts: HashMap<&str, usize> = HashMap::new();
    for s in &active_snippets {
        if !s.id.is_empty() {
            *id_counts.entry(&s.id).or_insert(0) += 1;
        }
    }
    for (id, count) in &id_counts {
        if *count > 1 {
            diags(
                report,
                &lib_display,
                Some(lib_path),
                Some(id),
                "E-DUP-ID",
                Severity::Error,
                Repairability::Auto,
                format!("Duplicate snippet ID '{id}' found {count} times"),
            );
        }
    }

    // d. Empty IDs (should have been assigned on load by load_library)
    for s in &active_snippets {
        if s.id.is_empty() {
            diags(
                report,
                &lib_display,
                Some(lib_path),
                None,
                "W-ID-EMPTY",
                Severity::Warning,
                Repairability::Auto,
                format!(
                    "Snippet '{}' has empty ID (load_library assigns IDs on read)",
                    truncate_desc(&s.description)
                ),
            );
        }
    }

    // e. Empty commands / descriptions
    for s in &active_snippets {
        if s.command.trim().is_empty() {
            diags(
                report,
                &lib_display,
                Some(lib_path),
                if s.id.is_empty() { None } else { Some(&s.id) },
                "E-CMD-EMPTY",
                Severity::Error,
                Repairability::Manual,
                format!(
                    "Snippet '{}' has empty command",
                    truncate_desc(&s.description)
                ),
            );
        }
        if s.description.trim().is_empty() {
            let sev = if strict {
                Severity::Error
            } else {
                Severity::Warning
            };
            diags(
                report,
                &lib_display,
                Some(lib_path),
                if s.id.is_empty() { None } else { Some(&s.id) },
                "W-DESC-EMPTY",
                sev,
                Repairability::Manual,
                format!(
                    "Snippet (id={}) has empty description",
                    if s.id.is_empty() {
                        "<unassigned>".to_string()
                    } else {
                        s.id.clone()
                    }
                ),
            );
        }
    }

    // f. Same-ID divergent content — IDs should be unique after load_library
    //    dedup, so this checks the raw TOML before dedup. We re-parse without
    //    dedup to detect duplicates.
    if let Ok(raw_snippets) = toml::from_str::<Snippets>(&raw_content) {
        let mut id_map: HashMap<&str, Vec<(String, String)>> = HashMap::new();
        for s in &raw_snippets.snippets {
            if !s.id.is_empty() {
                id_map
                    .entry(&s.id)
                    .or_default()
                    .push((s.description.clone(), s.command.clone()));
            }
        }
        for (id, entries) in &id_map {
            if entries.len() > 1 {
                let (desc0, cmd0) = &entries[0];
                let has_divergence = entries.iter().any(|(d, c)| d != desc0 || c != cmd0);
                if has_divergence {
                    diags(
                        report,
                        &lib_display,
                        Some(lib_path),
                        Some(id),
                        "W-SAME-ID-DIVERGENT",
                        Severity::Warning,
                        Repairability::Auto,
                        format!(
                            "Snippet ID '{id}' appears {} times with divergent content",
                            entries.len()
                        ),
                    );
                }
            }
        }
    }

    // g. Exact duplicate entries (same description + command)
    let mut seen_pairs: HashSet<(&str, &str)> = HashSet::new();
    for s in &active_snippets {
        let pair = (s.description.as_str(), s.command.as_str());
        if !seen_pairs.insert(pair) {
            diags(
                report,
                &lib_display,
                Some(lib_path),
                if s.id.is_empty() { None } else { Some(&s.id) },
                "W-EXACT-DUP",
                Severity::Warning,
                Repairability::Manual,
                format!(
                    "Exact duplicate snippet: '{}' / '{}'",
                    truncate_desc(&s.description),
                    truncate_cmd(&s.command)
                ),
            );
        }
    }

    // Check for backup file creation success (verify .toml.bak or .toml.corrupt.bak not present)
    let corrupt_bak = lib_path.with_extension("toml.corrupt.bak");
    if corrupt_bak.exists() {
        diags(
            report,
            &lib_display,
            Some(lib_path),
            None,
            "W-CORRUPT-BAK",
            Severity::Warning,
            Repairability::Auto,
            format!(
                "Corrupt backup exists at {} (indicates previous parse failure)",
                corrupt_bak.display()
            ),
        );
    }
}

/// Validate library index cross-references.
fn validate_index(report: &mut ValidationReport, mgr: &LibraryManager, _strict: bool) {
    let libraries_dir = mgr.get_libraries_dir().clone();

    // h. Check library index references to missing files
    for lib_meta in mgr.list_libraries() {
        let expected_path = libraries_dir.join(format!("{}.toml", lib_meta.filename));
        if !expected_path.exists() {
            diags(
                report,
                &lib_meta.filename,
                Some(&expected_path),
                None,
                "E-INDEX-MISSING-FILE",
                Severity::Error,
                Repairability::Manual,
                format!(
                    "Library '{}' is registered in index but file {} does not exist",
                    lib_meta.filename,
                    expected_path.display()
                ),
            );
        }
    }

    // i. Orphaned library files (files in libraries/ not in index)
    if libraries_dir.exists() {
        let indexed: HashSet<&str> = mgr
            .list_libraries()
            .iter()
            .map(|l| l.filename.as_str())
            .collect();

        if let Ok(entries) = std::fs::read_dir(&libraries_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "toml")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                    && !indexed.contains(stem)
                {
                    diags(
                        report,
                        stem,
                        Some(&path),
                        None,
                        "W-ORPHAN-FILE",
                        Severity::Warning,
                        Repairability::Auto,
                        format!(
                            "File {} is not registered in the library index",
                            path.display()
                        ),
                    );
                }
            }
        }
    }

    // j. Invalid primary library (missing or nonexistent)
    match mgr.get_primary_library() {
        Some(primary) => {
            let primary_path = libraries_dir.join(format!("{}.toml", primary.filename));
            if !primary_path.exists() {
                diags(
                    report,
                    &primary.filename,
                    Some(&primary_path),
                    None,
                    "E-PRIMARY-MISSING",
                    Severity::Error,
                    Repairability::Manual,
                    format!(
                        "Primary library '{}' file does not exist at {}",
                        primary.filename,
                        primary_path.display()
                    ),
                );
            }
        }
        None => {
            let libs = mgr.list_libraries();
            if !libs.is_empty() {
                diags(
                    report,
                    "<none>",
                    None,
                    None,
                    "W-NO-PRIMARY",
                    Severity::Warning,
                    Repairability::Auto,
                    "No primary library is set but libraries exist".to_string(),
                );
            }
        }
    }
}

/// Validate orphaned usage entries.
fn validate_usage(report: &mut ValidationReport, mgr: &LibraryManager) {
    // k. Orphaned usage entries
    let usage_index = UsageIndex::load();
    if usage_index.entries().is_empty() {
        return;
    }

    // Collect all active snippet IDs across all libraries
    let mut active_ids: HashSet<String> = HashSet::new();
    let libraries_dir = mgr.get_libraries_dir();
    for lib_meta in mgr.list_libraries() {
        let lib_path = libraries_dir.join(format!("{}.toml", lib_meta.filename));
        if let Ok(snippets) = crate::library::load_library(&lib_path) {
            for s in &snippets.snippets {
                active_ids.insert(s.id.clone());
            }
        }
    }

    for entry in usage_index.entries() {
        if !entry.id.is_empty() && !active_ids.contains(&entry.id) {
            diags(
                report,
                "<usage>",
                None,
                Some(&entry.id),
                "W-USAGE-ORPHAN",
                Severity::Warning,
                Repairability::Auto,
                format!(
                    "Usage entry for snippet '{}' references a snippet not found in any library",
                    entry.id
                ),
            );
        }
    }
}

/// Validate permissions on sensitive config files.
fn validate_permissions(report: &mut ValidationReport, _strict: bool) {
    // l. Insecure permissions on sensitive files
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let config_dir = crate::utils::config::get_config_dir();
        let sensitive_files: Vec<PathBuf> = vec![
            config_dir.join("snippets.toml"),
            config_dir.join("libraries.toml"),
            config_dir.join("sync.toml"),
            config_dir.join("usage.toml"),
        ];

        for path in &sensitive_files {
            if path.exists()
                && let Ok(meta) = std::fs::metadata(path)
            {
                let mode = meta.permissions().mode() & 0o777;
                // Files with world-readable or group-writable bits
                if mode & 0o077 != 0 {
                    diags(
                        report,
                        "<permissions>",
                        Some(path),
                        None,
                        "W-INSECURE-PERMS",
                        Severity::Warning,
                        Repairability::Auto,
                        format!(
                            "File {} has permissions {:o} (recommended: 0600)",
                            path.display(),
                            mode
                        ),
                    );
                }
            }
        }
    }
    let _ = report; // suppress unused warning on non-unix
}

/// Emit the report in human-readable format to stderr.
fn emit_human(report: &ValidationReport) {
    eprintln!();
    eprintln!("Validation Report");
    eprintln!("=================");
    eprintln!("Version: {}", report.tool_version);
    eprintln!("Strict mode: {}", report.strict_mode);
    eprintln!("Libraries: {}", report.total_libraries);
    eprintln!("Snippets: {}", report.total_snippets);

    let errors = report.error_count();
    let warnings = report.warning_count();
    let infos = report.info_count();

    if errors > 0 {
        eprintln!();
        eprintln!("Errors ({errors}):");
        for d in &report.diagnostics {
            if d.severity == Severity::Error {
                emit_diagnostic(d);
            }
        }
    }

    if warnings > 0 {
        eprintln!();
        eprintln!("Warnings ({warnings}):");
        for d in &report.diagnostics {
            if d.severity == Severity::Warning {
                emit_diagnostic(d);
            }
        }
    }

    if infos > 0 {
        eprintln!();
        eprintln!("Info ({infos}):");
        for d in &report.diagnostics {
            if d.severity == Severity::Info {
                emit_diagnostic(d);
            }
        }
    }

    if report.diagnostics.is_empty() {
        eprintln!();
        eprintln!("No issues found.");
    }

    eprintln!();
}

fn emit_diagnostic(d: &ValidationDiagnostic) {
    let lib = d.library.as_deref().unwrap_or("-");
    let snippet = d
        .snippet_id
        .as_deref()
        .map(|s| format!(" [snippet:{s}]"))
        .unwrap_or_default();
    let repair = match d.repairability {
        Repairability::Auto => " (auto-repairable)",
        Repairability::Manual => " (manual fix required)",
        Repairability::Unrepairable => " (unrepairable)",
    };
    eprintln!(
        "  [{code}] {lib}{snippet}: {msg}{repair}",
        code = d.code,
        msg = d.message
    );
}

/// Truncate a description for display.
fn truncate_desc(s: &str) -> String {
    if s.len() > 40 {
        format!("{}...", &s[..37])
    } else {
        s.to_string()
    }
}

/// Truncate a command for display.
fn truncate_cmd(s: &str) -> String {
    let one_line = s.lines().next().unwrap_or("");
    if one_line.len() > 50 {
        format!("{}...", &one_line[..47])
    } else {
        one_line.to_string()
    }
}

/// Run validation.
pub fn run(library: Option<String>, strict: bool, json: bool) -> SnipResult<()> {
    let mgr = crate::commands::init_library_manager()?;

    let mut report = ValidationReport::new(strict);

    // Determine which libraries to validate
    let lib_names: Vec<String> = match &library {
        Some(name) => {
            let lib = mgr.get_library_by_filename(name).ok_or_else(|| {
                SnipError::runtime_error(
                    "Library not found",
                    Some(&format!(
                        "Library '{name}' does not exist. Use 'snp library list' to see available libraries."
                    )),
                )
            })?;
            vec![lib.filename.clone()]
        }
        None => mgr
            .list_libraries()
            .iter()
            .map(|l| l.filename.clone())
            .collect(),
    };

    report.total_libraries = lib_names.len();
    let libraries_dir = mgr.get_libraries_dir().clone();

    // a. Validate each library
    for lib_name in &lib_names {
        let lib_path = libraries_dir.join(format!("{lib_name}.toml"));
        validate_library(&mut report, lib_name, &lib_path, strict);
    }

    // h, i, j. Validate index cross-references
    validate_index(&mut report, &mgr, strict);

    // k. Validate orphaned usage entries
    validate_usage(&mut report, &mgr);

    // l. Validate file permissions
    validate_permissions(&mut report, strict);

    // Strict mode: elevate warnings to errors for designated codes
    if strict {
        const STRICT_CODES: &[&str] = &[
            "W-ID-EMPTY",
            "W-DESC-EMPTY",
            "W-SAME-ID-DIVERGENT",
            "W-EXACT-DUP",
        ];
        for d in &mut report.diagnostics {
            if d.severity == Severity::Warning && STRICT_CODES.contains(&d.code.as_str()) {
                d.severity = Severity::Error;
            }
        }
    }

    // Output report
    if json {
        let json_str = serde_json::to_string_pretty(&report).map_err(|e| {
            SnipError::runtime_error("Failed to serialize report", Some(&e.to_string()))
        })?;
        println!("{json_str}");
    } else {
        emit_human(&report);
    }

    // Exit code: 2 if errors, 0 otherwise
    if report.has_errors() {
        std::process::exit(2);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{LibraryConfig, LibraryMeta, Snippet, Snippets};
    use tempfile::TempDir;

    fn make_snippet(id: &str, desc: &str, cmd: &str) -> Snippet {
        Snippet {
            id: id.to_string(),
            description: desc.to_string(),
            command: cmd.to_string(),
            ..Default::default()
        }
    }

    fn write_library(path: &std::path::Path, snippets: &Snippets) {
        let content = toml::to_string_pretty(snippets).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn write_libraries_toml(dir: &std::path::Path, libs: &[LibraryMeta]) {
        let config = LibraryConfig {
            libraries: libs.to_vec(),
        };
        let content = toml::to_string_pretty(&config).unwrap();
        std::fs::write(dir.join("libraries.toml"), content).unwrap();
    }

    #[test]
    fn test_report_new_defaults() {
        let report = ValidationReport::new(false);
        assert_eq!(report.schema_version, "1.0.0");
        assert!(!report.strict_mode);
        assert!(report.dry_run);
        assert_eq!(report.total_libraries, 0);
        assert_eq!(report.total_snippets, 0);
        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn test_report_counts() {
        let mut report = ValidationReport::new(false);
        report.diagnostics.push(ValidationDiagnostic {
            code: "E-TEST".to_string(),
            severity: Severity::Error,
            path: None,
            library: None,
            snippet_id: None,
            message: "err".to_string(),
            repairability: Repairability::Manual,
        });
        report.diagnostics.push(ValidationDiagnostic {
            code: "W-TEST".to_string(),
            severity: Severity::Warning,
            path: None,
            library: None,
            snippet_id: None,
            message: "warn".to_string(),
            repairability: Repairability::Auto,
        });
        report.diagnostics.push(ValidationDiagnostic {
            code: "I-TEST".to_string(),
            severity: Severity::Info,
            path: None,
            library: None,
            snippet_id: None,
            message: "info".to_string(),
            repairability: Repairability::Unrepairable,
        });

        assert!(report.has_errors());
        assert_eq!(report.error_count(), 1);
        assert_eq!(report.warning_count(), 1);
        assert_eq!(report.info_count(), 1);
    }

    #[test]
    fn test_report_no_errors() {
        let report = ValidationReport::new(false);
        assert!(!report.has_errors());
        assert_eq!(report.error_count(), 0);
    }

    #[test]
    fn test_validate_library_empty_file() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("test.toml");
        std::fs::write(&lib_path, "").unwrap();

        let mut report = ValidationReport::new(false);
        validate_library(&mut report, "test", &lib_path, false);

        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].code, "I-FILE-EMPTY");
        assert_eq!(report.diagnostics[0].severity, Severity::Info);
    }

    #[test]
    fn test_validate_library_valid() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("valid.toml");
        let snippets = Snippets {
            snippets: vec![
                make_snippet("id1", "desc1", "cmd1"),
                make_snippet("id2", "desc2", "cmd2"),
            ],
            ..Default::default()
        };
        write_library(&lib_path, &snippets);

        let mut report = ValidationReport::new(false);
        validate_library(&mut report, "valid", &lib_path, false);

        assert!(report.diagnostics.is_empty());
        assert_eq!(report.total_snippets, 2);
    }

    #[test]
    fn test_validate_library_duplicate_ids() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("dup.toml");
        let snippets = Snippets {
            snippets: vec![
                make_snippet("same-id", "desc1", "cmd1"),
                make_snippet("same-id", "desc2", "cmd2"),
            ],
            ..Default::default()
        };
        write_library(&lib_path, &snippets);

        let mut report = ValidationReport::new(false);
        validate_library(&mut report, "dup", &lib_path, false);

        let dup_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "E-DUP-ID")
            .collect();
        assert_eq!(dup_diags.len(), 1);
        assert_eq!(dup_diags[0].snippet_id.as_deref(), Some("same-id"));
    }

    #[test]
    fn test_validate_library_empty_description() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("empty_desc.toml");
        let snippets = Snippets {
            snippets: vec![make_snippet("id1", "", "cmd1")],
            ..Default::default()
        };
        write_library(&lib_path, &snippets);

        let mut report = ValidationReport::new(false);
        validate_library(&mut report, "empty_desc", &lib_path, false);

        let desc_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "W-DESC-EMPTY")
            .collect();
        assert_eq!(desc_diags.len(), 1);
        assert_eq!(desc_diags[0].severity, Severity::Warning);
    }

    #[test]
    fn test_validate_library_empty_description_strict() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("empty_desc_strict.toml");
        let snippets = Snippets {
            snippets: vec![make_snippet("id1", "", "cmd1")],
            ..Default::default()
        };
        write_library(&lib_path, &snippets);

        let mut report = ValidationReport::new(true);
        validate_library(&mut report, "empty_desc_strict", &lib_path, true);

        let desc_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "W-DESC-EMPTY")
            .collect();
        assert_eq!(desc_diags.len(), 1);
        assert_eq!(desc_diags[0].severity, Severity::Error);
    }

    #[test]
    fn test_validate_library_empty_command() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("empty_cmd.toml");
        let snippets = Snippets {
            snippets: vec![make_snippet("id1", "desc1", "")],
            ..Default::default()
        };
        write_library(&lib_path, &snippets);

        let mut report = ValidationReport::new(false);
        validate_library(&mut report, "empty_cmd", &lib_path, false);

        let cmd_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "E-CMD-EMPTY")
            .collect();
        assert_eq!(cmd_diags.len(), 1);
        assert_eq!(cmd_diags[0].severity, Severity::Error);
    }

    #[test]
    fn test_validate_library_exact_duplicates() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("exact_dup.toml");
        let snippets = Snippets {
            snippets: vec![
                make_snippet("id1", "same desc", "same cmd"),
                make_snippet("id2", "same desc", "same cmd"),
            ],
            ..Default::default()
        };
        write_library(&lib_path, &snippets);

        let mut report = ValidationReport::new(false);
        validate_library(&mut report, "exact_dup", &lib_path, false);

        let dup_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "W-EXACT-DUP")
            .collect();
        assert_eq!(dup_diags.len(), 1);
    }

    #[test]
    fn test_validate_library_same_id_divergent() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("divergent.toml");
        let snippets = Snippets {
            snippets: vec![
                make_snippet("same-id", "desc1", "cmd1"),
                make_snippet("same-id", "desc2", "cmd2"),
            ],
            ..Default::default()
        };
        write_library(&lib_path, &snippets);

        let mut report = ValidationReport::new(false);
        validate_library(&mut report, "divergent", &lib_path, false);

        let div_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "W-SAME-ID-DIVERGENT")
            .collect();
        assert_eq!(div_diags.len(), 1);
        assert_eq!(div_diags[0].repairability, Repairability::Auto);
    }

    #[test]
    fn test_validate_library_toml_parse_error() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("bad.toml");
        std::fs::write(&lib_path, "invalid = [toml").unwrap();

        let mut report = ValidationReport::new(false);
        validate_library(&mut report, "bad", &lib_path, false);

        let parse_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "E-TOML-PARSE")
            .collect();
        assert_eq!(parse_diags.len(), 1);
        assert_eq!(parse_diags[0].repairability, Repairability::Manual);
    }

    #[test]
    fn test_validate_index_missing_file() {
        let tmp = TempDir::new().unwrap();
        let mut meta = LibraryMeta::new("missing");
        meta.is_primary = true;
        write_libraries_toml(tmp.path(), &[meta.clone()]);

        let config_dir = tmp.path().join("xfg/snp");
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::copy(
            tmp.path().join("libraries.toml"),
            config_dir.join("libraries.toml"),
        )
        .unwrap();

        let mgr = LibraryManager::with_config_dir(config_dir).unwrap();

        let mut report = ValidationReport::new(false);
        validate_index(&mut report, &mgr, false);

        let missing_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "E-INDEX-MISSING-FILE")
            .collect();
        assert_eq!(missing_diags.len(), 1);
    }

    #[test]
    fn test_validate_index_orphan_file() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().join("xfg/snp");
        let libs_dir = config_dir.join("libraries");
        std::fs::create_dir_all(&libs_dir).unwrap();
        std::fs::write(libs_dir.join("orphan.toml"), "snippets = []\n").unwrap();
        write_libraries_toml(&config_dir, &[]);

        let mgr = LibraryManager::with_config_dir(config_dir).unwrap();

        let mut report = ValidationReport::new(false);
        validate_index(&mut report, &mgr, false);

        let orphan_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "W-ORPHAN-FILE")
            .collect();
        assert_eq!(orphan_diags.len(), 1);
        assert_eq!(orphan_diags[0].repairability, Repairability::Auto);
    }

    #[test]
    fn test_validate_no_primary() {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().join("xfg/snp");
        let libs_dir = config_dir.join("libraries");
        std::fs::create_dir_all(&libs_dir).unwrap();
        write_libraries_toml(&config_dir, &[LibraryMeta::new("lib1")]);

        let mgr = LibraryManager::with_config_dir(config_dir).unwrap();

        let mut report = ValidationReport::new(false);
        validate_index(&mut report, &mgr, false);

        let primary_diags: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "W-NO-PRIMARY")
            .collect();
        assert_eq!(primary_diags.len(), 1);
    }

    #[test]
    fn test_strict_mode_elevates_warnings() {
        let mut report = ValidationReport::new(true);
        report.diagnostics.push(ValidationDiagnostic {
            code: "W-ID-EMPTY".to_string(),
            severity: Severity::Warning,
            path: None,
            library: None,
            snippet_id: None,
            message: "test".to_string(),
            repairability: Repairability::Auto,
        });
        report.diagnostics.push(ValidationDiagnostic {
            code: "W-OTHER".to_string(),
            severity: Severity::Warning,
            path: None,
            library: None,
            snippet_id: None,
            message: "other".to_string(),
            repairability: Repairability::Auto,
        });

        // Simulate strict elevation
        const STRICT_CODES: &[&str] = &[
            "W-ID-EMPTY",
            "W-DESC-EMPTY",
            "W-SAME-ID-DIVERGENT",
            "W-EXACT-DUP",
        ];
        for d in &mut report.diagnostics {
            if d.severity == Severity::Warning && STRICT_CODES.contains(&d.code.as_str()) {
                d.severity = Severity::Error;
            }
        }

        assert_eq!(report.diagnostics[0].severity, Severity::Error);
        assert_eq!(report.diagnostics[1].severity, Severity::Warning);
    }

    #[test]
    fn test_truncate_desc() {
        assert_eq!(truncate_desc("short"), "short");
        assert_eq!(
            truncate_desc(&"a".repeat(50)),
            format!("{}...", "a".repeat(37))
        );
    }

    #[test]
    fn test_truncate_cmd() {
        assert_eq!(truncate_cmd("echo hi"), "echo hi");
        assert_eq!(
            truncate_cmd(&format!("echo {}", "x".repeat(60))),
            format!("echo {}...", "x".repeat(42))
        );
    }

    #[test]
    fn test_severity_serialization_roundtrip() {
        for sev in [Severity::Info, Severity::Warning, Severity::Error] {
            let json = serde_json::to_string(&sev).unwrap();
            let back: Severity = serde_json::from_str(&json).unwrap();
            assert_eq!(sev, back);
        }
    }

    #[test]
    fn test_repairability_serialization_roundtrip() {
        for rep in [
            Repairability::Auto,
            Repairability::Manual,
            Repairability::Unrepairable,
        ] {
            let json = serde_json::to_string(&rep).unwrap();
            let back: Repairability = serde_json::from_str(&json).unwrap();
            assert_eq!(rep, back);
        }
    }

    #[test]
    fn test_validation_diagnostic_serialization() {
        let diag = ValidationDiagnostic {
            code: "E-TEST".to_string(),
            severity: Severity::Error,
            path: Some(PathBuf::from("/tmp/test.toml")),
            library: Some("mylib".to_string()),
            snippet_id: Some("abc-123".to_string()),
            message: "test message".to_string(),
            repairability: Repairability::Auto,
        };
        let json = serde_json::to_string(&diag).unwrap();
        let back: ValidationDiagnostic = serde_json::from_str(&json).unwrap();
        assert_eq!(back.code, "E-TEST");
        assert_eq!(back.severity, Severity::Error);
        assert_eq!(back.library, Some("mylib".to_string()));
    }

    #[test]
    fn test_validation_report_full_roundtrip() {
        let mut report = ValidationReport::new(false);
        report.total_libraries = 2;
        report.total_snippets = 10;
        report.diagnostics.push(ValidationDiagnostic {
            code: "W-TEST".to_string(),
            severity: Severity::Warning,
            path: None,
            library: Some("lib".to_string()),
            snippet_id: None,
            message: "warn".to_string(),
            repairability: Repairability::Manual,
        });

        let json = serde_json::to_string_pretty(&report).unwrap();
        let back: ValidationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.total_libraries, 2);
        assert_eq!(back.total_snippets, 10);
        assert_eq!(back.diagnostics.len(), 1);
        assert!(!back.strict_mode);
        assert!(back.dry_run);
    }

    #[cfg(unix)]
    #[test]
    fn test_validate_permissions_does_not_panic() {
        let mut report = ValidationReport::new(false);

        // Just verify the function doesn't panic when run against the real config dir
        validate_permissions(&mut report, false);
    }
}
