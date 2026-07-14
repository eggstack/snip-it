use crate::diagnostics::{
    CompatibilityDiagnostic, DiagnosticSeverity, DoctorReport, ImportDuplicate, diagnostic_counts,
    version,
};
use crate::error::{SnipError, SnipResult};
use crate::library::{LibraryManager, Snippet, Snippets};
use crate::utils::toml_helpers::fix_invalid_toml_escapes;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;

/// Output format for the doctor report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub enum DiagnosticReportFormat {
    #[default]
    Human,
    Json,
}

/// Known field names for pet snippet entries (canonical + aliases).
const KNOWN_SNIPPET_FIELDS: &[&str] = &[
    "id",
    "description",
    "command",
    "output",
    "tag",
    "tags",
    "folders",
    "favorite",
    "created_at",
    "updated_at",
    "device_id",
    "deleted",
    "name",
    "cmd",
    "Tag",
    "Tags",
    "Description",
    "Command",
    "Output",
    "Id",
    "ID",
];

/// Read and validate a source file using the same checks as import_cmd.
fn read_source_file(path: &Path) -> SnipResult<String> {
    let metadata = fs::metadata(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SnipError::runtime_error(
                "Source file not found",
                Some(&format!("'{}' does not exist", path.display())),
            )
        } else {
            SnipError::io_error("read source file metadata", path, e)
        }
    })?;

    if metadata.is_dir() {
        return Err(SnipError::runtime_error(
            "Path is a directory",
            Some(&format!("'{}' is a directory, not a file", path.display())),
        ));
    }

    if !metadata.is_file() {
        return Err(SnipError::runtime_error(
            "Unsupported file type",
            Some(&format!("'{}' is not a regular file", path.display())),
        ));
    }

    let mut bytes = Vec::new();
    let file =
        fs::File::open(path).map_err(|e| SnipError::io_error("open source file", path, e))?;
    std::io::BufReader::new(file)
        .take((MAX_FILE_BYTES as u64) + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| SnipError::io_error("read source file", path, e))?;

    if bytes.len() > MAX_FILE_BYTES {
        return Err(SnipError::runtime_error(
            "Source file too large",
            Some(&format!(
                "Files are limited to {} MiB",
                MAX_FILE_BYTES / (1024 * 1024)
            )),
        ));
    }

    let content = String::from_utf8(bytes).map_err(|_| {
        SnipError::runtime_error(
            "Invalid source file",
            Some("Source file must be valid UTF-8"),
        )
    })?;

    if content.contains('\0') {
        return Err(SnipError::runtime_error(
            "Invalid source file",
            Some("Source file cannot contain NUL bytes"),
        ));
    }

    Ok(content)
}

/// Parse raw TOML content into a `Snippets` collection.
fn parse_pet_toml(content: &str) -> SnipResult<Snippets> {
    let fixed = fix_invalid_toml_escapes(content);
    toml::from_str(&fixed).map_err(|e| SnipError::toml_error("parse pet TOML", e))
}

/// Detect unknown fields, missing required keys, and structural issues.
fn detect_unknown_fields(raw_toml: &str) -> Vec<CompatibilityDiagnostic> {
    let mut diagnostics = Vec::new();

    let value: toml::Value = match toml::from_str(raw_toml) {
        Ok(v) => v,
        Err(_) => return diagnostics,
    };

    let entries = match value.get("snippets").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return diagnostics,
    };

    for (i, entry) in entries.iter().enumerate() {
        let table = match entry.as_table() {
            Some(t) => t,
            None => continue,
        };

        for key in table.keys() {
            if !KNOWN_SNIPPET_FIELDS.contains(&key.as_str()) {
                diagnostics.push(CompatibilityDiagnostic {
                    code: format!("field.unknown.{key}"),
                    entry_index: Some(i),
                    field: Some(key.clone()),
                    severity: DiagnosticSeverity::Info,
                    message: format!("Unknown field '{}' will be ignored", key),
                    suggestion: None,
                });
            }
        }

        if !table.contains_key("description")
            && !table.contains_key("Description")
            && !table.contains_key("name")
        {
            diagnostics.push(CompatibilityDiagnostic {
                code: "field.missing.description".to_string(),
                entry_index: Some(i),
                field: Some("description".to_string()),
                severity: DiagnosticSeverity::Warning,
                message: "Entry missing 'description' field (will be empty)".to_string(),
                suggestion: None,
            });
        }

        if !table.contains_key("command")
            && !table.contains_key("Command")
            && !table.contains_key("cmd")
        {
            diagnostics.push(CompatibilityDiagnostic {
                code: "field.missing.command".to_string(),
                entry_index: Some(i),
                field: Some("command".to_string()),
                severity: DiagnosticSeverity::Warning,
                message: "Entry missing 'command' field (will be empty)".to_string(),
                suggestion: None,
            });
        }
    }

    diagnostics
}

/// Analyze a single pet entry and produce diagnostics.
fn analyze_entry(index: usize, pet: &Snippet) -> Vec<CompatibilityDiagnostic> {
    let mut diagnostics = Vec::new();

    if pet.description.trim().is_empty() {
        diagnostics.push(CompatibilityDiagnostic {
            code: "entry.empty_description".to_string(),
            entry_index: Some(index),
            field: Some("description".to_string()),
            severity: DiagnosticSeverity::Warning,
            message: "Entry has empty description".to_string(),
            suggestion: None,
        });
    }

    if pet.command.trim().is_empty() {
        diagnostics.push(CompatibilityDiagnostic {
            code: "entry.empty_command".to_string(),
            entry_index: Some(index),
            field: Some("command".to_string()),
            severity: DiagnosticSeverity::Error,
            message: "Entry has empty command".to_string(),
            suggestion: None,
        });
    }

    if !pet.output.is_empty() {
        diagnostics.push(CompatibilityDiagnostic {
            code: "entry.output_field".to_string(),
            entry_index: Some(index),
            field: Some("output".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "Entry has output field (preserved)".to_string(),
            suggestion: None,
        });
    }

    if pet.tags.is_empty() {
        diagnostics.push(CompatibilityDiagnostic {
            code: "entry.no_tags".to_string(),
            entry_index: Some(index),
            field: Some("tag".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "Entry has no tags".to_string(),
            suggestion: None,
        });
    }

    let vars = crate::utils::variables::parse_variables(&pet.command);
    if vars.iter().any(|v| {
        matches!(
            v.kind,
            crate::utils::variables::VariableKind::Choices { .. }
        )
    }) {
        diagnostics.push(CompatibilityDiagnostic {
            code: "entry.choice_variables".to_string(),
            entry_index: Some(index),
            field: Some("command".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "Entry contains choice variables".to_string(),
            suggestion: None,
        });
    }

    diagnostics
}

/// Check if two snippets are exact duplicates (same command and description).
fn is_exact_duplicate(a: &Snippet, b: &Snippet) -> bool {
    a.command == b.command && a.description == b.description
}

/// Check if two snippets have the same command but different descriptions.
fn same_command_different_description(a: &Snippet, b: &Snippet) -> bool {
    a.command == b.command && a.description != b.description
}

/// Check if two snippets have the same description but different commands.
fn same_description_different_command(a: &Snippet, b: &Snippet) -> bool {
    a.description == b.description && a.command != b.command
}

/// Build a DoctorReport from a pet file analysis.
fn build_pet_report(source_path: &Path, content: &str, strict: bool) -> SnipResult<DoctorReport> {
    let mut report = DoctorReport::new(strict);
    report.source = Some(source_path.display().to_string());

    // Raw TOML structural analysis
    let structural = detect_unknown_fields(content);
    report.diagnostics.extend(structural);

    // Parse into Snippets
    let pet_snippets = match parse_pet_toml(content) {
        Ok(s) => s,
        Err(e) => {
            report.has_toml_error = true;
            report.toml_error_detail = Some(e.to_string());
            return Ok(report);
        }
    };

    report.total_entries = pet_snippets.snippets.len();

    // Analyze each entry
    for (i, pet) in pet_snippets.snippets.iter().enumerate() {
        let entry_diags = analyze_entry(i, pet);
        report.diagnostics.extend(entry_diags);
    }

    // Detect duplicates
    for i in 0..pet_snippets.snippets.len() {
        for j in (i + 1)..pet_snippets.snippets.len() {
            if is_exact_duplicate(&pet_snippets.snippets[i], &pet_snippets.snippets[j]) {
                report.duplicates.push(ImportDuplicate {
                    source_index: i,
                    destination_index: j,
                    description: pet_snippets.snippets[i].description.clone(),
                    reason: "Exact duplicate (same command and description)".to_string(),
                });
            } else if same_command_different_description(
                &pet_snippets.snippets[i],
                &pet_snippets.snippets[j],
            ) {
                report.diagnostics.push(CompatibilityDiagnostic {
                    code: "duplicate.same_command".to_string(),
                    entry_index: Some(i),
                    field: Some("command".to_string()),
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Same command as entry {} ('{}') but different description",
                        j, pet_snippets.snippets[j].description
                    ),
                    suggestion: None,
                });
            } else if same_description_different_command(
                &pet_snippets.snippets[i],
                &pet_snippets.snippets[j],
            ) {
                report.diagnostics.push(CompatibilityDiagnostic {
                    code: "duplicate.same_description".to_string(),
                    entry_index: Some(i),
                    field: Some("description".to_string()),
                    severity: DiagnosticSeverity::Warning,
                    message: format!("Same description as entry {} but different command", j),
                    suggestion: None,
                });
            }
        }
    }

    // Detect capabilities
    report.detected_capabilities.push("toml_format".to_string());
    report
        .detected_capabilities
        .push(format!("snippet_count={}", pet_snippets.snippets.len()));

    let has_vars = pet_snippets
        .snippets
        .iter()
        .any(|s| !crate::utils::variables::parse_variables(&s.command).is_empty());
    if has_vars {
        report.detected_capabilities.push("variables".to_string());
    }

    let has_choices = pet_snippets.snippets.iter().any(|s| {
        crate::utils::variables::parse_variables(&s.command)
            .iter()
            .any(|v| {
                matches!(
                    v.kind,
                    crate::utils::variables::VariableKind::Choices { .. }
                )
            })
    });
    if has_choices {
        report
            .detected_capabilities
            .push("choice_variables".to_string());
    }

    let has_output = pet_snippets.snippets.iter().any(|s| !s.output.is_empty());
    if has_output {
        report
            .detected_capabilities
            .push("output_fields".to_string());
    }

    let has_tags = pet_snippets.snippets.iter().any(|s| !s.tags.is_empty());
    if has_tags {
        report.detected_capabilities.push("tags".to_string());
    }

    // Build recommended import command
    let lib_name = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported");
    let (info_count, warn_count, error_count) = diagnostic_counts(&report.diagnostics);
    let has_errors = error_count > 0;

    let import_cmd = if has_errors {
        format!(
            "# Resolve errors first, then run:\n# snp import pet {}",
            source_path.display()
        )
    } else {
        format!("snp import pet {}", source_path.display())
    };
    report.recommended_import_command = Some(import_cmd);

    // Add suggestion to error diagnostics
    for diag in &mut report.diagnostics {
        if diag.severity == DiagnosticSeverity::Error && diag.suggestion.is_none() {
            diag.suggestion = Some("Fix this entry before importing".to_string());
        }
    }

    let _ = (warn_count, info_count);
    let _ = lib_name;

    Ok(report)
}

/// Run the compatibility check mode.
fn build_compatibility_report(strict: bool) -> SnipResult<DoctorReport> {
    let mut report = DoctorReport::new(strict);
    report.source = None;

    // Check binary version
    let exe_path = std::env::current_exe().ok();
    let exe_display = exe_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<unknown>".to_string());
    report.diagnostics.push(CompatibilityDiagnostic {
        code: "compat.binary_version".to_string(),
        entry_index: None,
        field: Some("binary".to_string()),
        severity: DiagnosticSeverity::Info,
        message: format!("snp version {} ({})", version(), exe_display),
        suggestion: None,
    });

    // Check config directory
    let config_dir = crate::utils::config::get_config_dir();
    if config_dir.exists() {
        let writable = fs::metadata(&config_dir)
            .map(|m| !m.permissions().readonly())
            .unwrap_or(false);
        if writable {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: "compat.config_dir.ok".to_string(),
                entry_index: None,
                field: Some("config_dir".to_string()),
                severity: DiagnosticSeverity::Info,
                message: format!(
                    "Config directory exists and is writable: {}",
                    config_dir.display()
                ),
                suggestion: None,
            });
        } else {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: "compat.config_dir.readonly".to_string(),
                entry_index: None,
                field: Some("config_dir".to_string()),
                severity: DiagnosticSeverity::Warning,
                message: format!("Config directory is read-only: {}", config_dir.display()),
                suggestion: Some("Fix permissions so snp can write config files".to_string()),
            });
        }
    } else {
        report.diagnostics.push(CompatibilityDiagnostic {
            code: "compat.config_dir.missing".to_string(),
            entry_index: None,
            field: Some("config_dir".to_string()),
            severity: DiagnosticSeverity::Warning,
            message: format!("Config directory does not exist: {}", config_dir.display()),
            suggestion: Some("Run any snp command to create the config directory".to_string()),
        });
    }

    // Check library directory
    let libraries_dir = config_dir.join("libraries");
    if libraries_dir.exists() {
        report.diagnostics.push(CompatibilityDiagnostic {
            code: "compat.libraries_dir.ok".to_string(),
            entry_index: None,
            field: Some("libraries_dir".to_string()),
            severity: DiagnosticSeverity::Info,
            message: format!("Library directory exists: {}", libraries_dir.display()),
            suggestion: None,
        });
    } else {
        report.diagnostics.push(CompatibilityDiagnostic {
            code: "compat.libraries_dir.missing".to_string(),
            entry_index: None,
            field: Some("libraries_dir".to_string()),
            severity: DiagnosticSeverity::Info,
            message: format!(
                "Library directory does not exist (single-file mode): {}",
                libraries_dir.display()
            ),
            suggestion: None,
        });
    }

    // Check primary library resolution
    match LibraryManager::new() {
        Ok(mgr) => match mgr.get_primary_library() {
            Some(primary) => {
                let lib_path = mgr
                    .get_libraries_dir()
                    .join(format!("{}.toml", primary.filename));
                let exists = lib_path.exists();
                let snippet_count = if exists {
                    crate::library::load_library(&lib_path)
                        .map(|s| s.snippets.len())
                        .unwrap_or(0)
                } else {
                    0
                };
                report.diagnostics.push(CompatibilityDiagnostic {
                    code: "compat.primary_library.ok".to_string(),
                    entry_index: None,
                    field: Some("primary_library".to_string()),
                    severity: DiagnosticSeverity::Info,
                    message: format!(
                        "Primary library '{}' ({} snippets)",
                        primary.filename, snippet_count
                    ),
                    suggestion: None,
                });
            }
            None => {
                report.diagnostics.push(CompatibilityDiagnostic {
                    code: "compat.primary_library.missing".to_string(),
                    entry_index: None,
                    field: Some("primary_library".to_string()),
                    severity: DiagnosticSeverity::Warning,
                    message: "No primary library is set".to_string(),
                    suggestion: Some(
                        "Create a library with 'snp library create <name>'".to_string(),
                    ),
                });
            }
        },
        Err(e) => {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: "compat.library_manager.error".to_string(),
                entry_index: None,
                field: Some("library_manager".to_string()),
                severity: DiagnosticSeverity::Warning,
                message: format!("Failed to initialize library manager: {e}"),
                suggestion: None,
            });
        }
    }

    // Check sync config presence
    let sync_path = crate::utils::config::get_sync_config_path();
    if sync_path.exists() {
        match crate::config::load_sync_settings() {
            Ok(settings) => {
                if settings.enabled {
                    report.diagnostics.push(CompatibilityDiagnostic {
                        code: "compat.sync.enabled".to_string(),
                        entry_index: None,
                        field: Some("sync".to_string()),
                        severity: DiagnosticSeverity::Info,
                        message: format!("Sync enabled (server: {})", settings.server_url),
                        suggestion: None,
                    });
                } else {
                    report.diagnostics.push(CompatibilityDiagnostic {
                        code: "compat.sync.disabled".to_string(),
                        entry_index: None,
                        field: Some("sync".to_string()),
                        severity: DiagnosticSeverity::Info,
                        message: "Sync config present but disabled".to_string(),
                        suggestion: None,
                    });
                }
            }
            Err(e) => {
                report.diagnostics.push(CompatibilityDiagnostic {
                    code: "compat.sync.config_error".to_string(),
                    entry_index: None,
                    field: Some("sync".to_string()),
                    severity: DiagnosticSeverity::Warning,
                    message: format!("Failed to load sync config: {e}"),
                    suggestion: Some("Check ~/.config/snp/sync.toml for syntax errors".to_string()),
                });
            }
        }
    } else {
        report.diagnostics.push(CompatibilityDiagnostic {
            code: "compat.sync.absent".to_string(),
            entry_index: None,
            field: Some("sync".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "No sync configuration found".to_string(),
            suggestion: Some("Run 'snp register' to set up sync".to_string()),
        });
    }

    // Check shell availability
    for shell_name in &["bash", "zsh", "fish"] {
        let found = std::process::Command::new("which")
            .arg(shell_name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if found {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: format!("compat.shell.{shell_name}.ok"),
                entry_index: None,
                field: Some(shell_name.to_string()),
                severity: DiagnosticSeverity::Info,
                message: format!("{shell_name} found on PATH"),
                suggestion: None,
            });
        } else {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: format!("compat.shell.{shell_name}.missing"),
                entry_index: None,
                field: Some(shell_name.to_string()),
                severity: DiagnosticSeverity::Info,
                message: format!("{shell_name} not found on PATH"),
                suggestion: None,
            });
        }
    }

    report.total_entries = 0;
    Ok(report)
}

/// Emit the report in human-readable format to stderr.
fn emit_human_report(report: &DoctorReport) {
    eprintln!();
    eprintln!("Doctor Report");
    eprintln!("=============");
    if let Some(ref source) = report.source {
        eprintln!("Source: {}", source);
    }
    eprintln!("Version: {}", report.tool_version);
    eprintln!("Entries: {}", report.total_entries);

    let (info_count, warn_count, error_count) = diagnostic_counts(&report.diagnostics);

    if report.has_toml_error {
        eprintln!();
        eprintln!("TOML Error:");
        if let Some(ref detail) = report.toml_error_detail {
            eprintln!("  {detail}");
        }
    }

    if error_count > 0 {
        eprintln!();
        eprintln!("Errors ({error_count}):");
        for diag in &report.diagnostics {
            if diag.severity == DiagnosticSeverity::Error {
                eprintln!(
                    "  [e] [{}] {}: {}",
                    diag.entry_index.map_or("-".to_string(), |i| i.to_string()),
                    diag.field.as_deref().unwrap_or("-"),
                    diag.message
                );
                if let Some(ref suggestion) = diag.suggestion {
                    eprintln!("        suggestion: {suggestion}");
                }
            }
        }
    }

    if warn_count > 0 {
        eprintln!();
        eprintln!("Warnings ({warn_count}):");
        for diag in &report.diagnostics {
            if diag.severity == DiagnosticSeverity::Warning {
                eprintln!(
                    "  [w] [{}] {}: {}",
                    diag.entry_index.map_or("-".to_string(), |i| i.to_string()),
                    diag.field.as_deref().unwrap_or("-"),
                    diag.message
                );
                if let Some(ref suggestion) = diag.suggestion {
                    eprintln!("        suggestion: {suggestion}");
                }
            }
        }
    }

    if info_count > 0 {
        eprintln!();
        eprintln!("Info ({info_count}):");
        for diag in &report.diagnostics {
            if diag.severity == DiagnosticSeverity::Info {
                eprintln!(
                    "  [i] [{}] {}: {}",
                    diag.entry_index.map_or("-".to_string(), |i| i.to_string()),
                    diag.field.as_deref().unwrap_or("-"),
                    diag.message
                );
            }
        }
    }

    if !report.duplicates.is_empty() {
        eprintln!();
        eprintln!("Duplicates ({}):", report.duplicates.len());
        for dup in &report.duplicates {
            eprintln!(
                "  [{}] {} — {}",
                dup.source_index, dup.description, dup.reason
            );
        }
    }

    if !report.detected_capabilities.is_empty() {
        eprintln!();
        eprintln!("Supported features:");
        for cap in &report.detected_capabilities {
            eprintln!("  {cap}");
        }
    }

    if let Some(ref cmd) = report.recommended_import_command {
        eprintln!();
        eprintln!("Suggested next command:");
        eprintln!("  {cmd}");
    }
}

/// Execute the doctor command.
pub fn run(
    pet_file: Option<PathBuf>,
    compatibility: bool,
    strict: bool,
    report_format: DiagnosticReportFormat,
) -> SnipResult<()> {
    let report = if let Some(ref path) = pet_file {
        let content = read_source_file(path)?;

        if content.trim().is_empty() {
            return Err(SnipError::runtime_error(
                "Empty source file",
                Some("Source file contains no data"),
            ));
        }

        build_pet_report(path, &content, strict)?
    } else if compatibility {
        build_compatibility_report(strict)?
    } else {
        return Err(SnipError::runtime_error(
            "No mode selected",
            Some("Specify --pet-file <path> or --compatibility"),
        ));
    };

    // Emit report
    match report_format {
        DiagnosticReportFormat::Human => {
            emit_human_report(&report);
        }
        DiagnosticReportFormat::Json => {
            let json = serde_json::to_string_pretty(&report).map_err(|e| {
                SnipError::runtime_error("Failed to serialize report", Some(&e.to_string()))
            })?;
            println!("{json}");
        }
    }

    // Determine exit code
    let has_errors = report
        .diagnostics
        .iter()
        .any(|d| d.severity == DiagnosticSeverity::Error);

    if has_errors {
        std::process::exit(2);
    }

    Ok(())
}
