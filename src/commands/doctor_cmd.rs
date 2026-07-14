use crate::commands::shell_cmd::{self, ShellType};
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
                    code: "I-FIELD-UNKNOWN".to_string(),
                    entry_index: Some(i),
                    field: Some(key.clone()),
                    severity: DiagnosticSeverity::Info,
                    message: format!("Unknown field '{}' will be ignored", key),
                    suggestion: None,
                    span: None,
                });
            }
        }

        if !table.contains_key("description")
            && !table.contains_key("Description")
            && !table.contains_key("name")
        {
            diagnostics.push(CompatibilityDiagnostic {
                code: "W-DESC-MISSING".to_string(),
                entry_index: Some(i),
                field: Some("description".to_string()),
                severity: DiagnosticSeverity::Warning,
                message: "Entry missing 'description' field (will be empty)".to_string(),
                suggestion: None,
                span: None,
            });
        }

        if !table.contains_key("command")
            && !table.contains_key("Command")
            && !table.contains_key("cmd")
        {
            diagnostics.push(CompatibilityDiagnostic {
                code: "W-CMD-MISSING".to_string(),
                entry_index: Some(i),
                field: Some("command".to_string()),
                severity: DiagnosticSeverity::Warning,
                message: "Entry missing 'command' field (will be empty)".to_string(),
                suggestion: None,
                span: None,
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
            code: "W-DESC-EMPTY".to_string(),
            entry_index: Some(index),
            field: Some("description".to_string()),
            severity: DiagnosticSeverity::Warning,
            message: "Entry has empty description".to_string(),
            suggestion: None,
            span: None,
        });
    }

    if pet.command.trim().is_empty() {
        diagnostics.push(CompatibilityDiagnostic {
            code: "E-CMD-EMPTY".to_string(),
            entry_index: Some(index),
            field: Some("command".to_string()),
            severity: DiagnosticSeverity::Error,
            message: "Entry has empty command".to_string(),
            suggestion: None,
            span: None,
        });
    }

    if !pet.output.is_empty() {
        diagnostics.push(CompatibilityDiagnostic {
            code: "I-OUTPUT-PRESENT".to_string(),
            entry_index: Some(index),
            field: Some("output".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "Entry has output field (preserved)".to_string(),
            suggestion: None,
            span: None,
        });
    }

    if pet.tags.is_empty() {
        diagnostics.push(CompatibilityDiagnostic {
            code: "I-TAGS-EMPTY".to_string(),
            entry_index: Some(index),
            field: Some("tag".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "Entry has no tags".to_string(),
            suggestion: None,
            span: None,
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
            code: "I-CHOICE-VARS".to_string(),
            entry_index: Some(index),
            field: Some("command".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "Entry contains choice variables".to_string(),
            suggestion: None,
            span: None,
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
                    code: "W-DUP-CMD".to_string(),
                    entry_index: Some(i),
                    field: Some("command".to_string()),
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Same command as entry {} ('{}') but different description",
                        j, pet_snippets.snippets[j].description
                    ),
                    suggestion: None,
                    span: None,
                });
            } else if same_description_different_command(
                &pet_snippets.snippets[i],
                &pet_snippets.snippets[j],
            ) {
                report.diagnostics.push(CompatibilityDiagnostic {
                    code: "W-DUP-DESC".to_string(),
                    entry_index: Some(i),
                    field: Some("description".to_string()),
                    severity: DiagnosticSeverity::Warning,
                    message: format!("Same description as entry {} but different command", j),
                    suggestion: None,
                    span: None,
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
        span: None,
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
                span: None,
            });
        } else {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: "compat.config_dir.readonly".to_string(),
                entry_index: None,
                field: Some("config_dir".to_string()),
                severity: DiagnosticSeverity::Warning,
                message: format!("Config directory is read-only: {}", config_dir.display()),
                suggestion: Some("Fix permissions so snp can write config files".to_string()),
                span: None,
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
            span: None,
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
            span: None,
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
            span: None,
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
                    span: None,
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
                    span: None,
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
                span: None,
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
                        span: None,
                    });
                } else {
                    report.diagnostics.push(CompatibilityDiagnostic {
                        code: "compat.sync.disabled".to_string(),
                        entry_index: None,
                        field: Some("sync".to_string()),
                        severity: DiagnosticSeverity::Info,
                        message: "Sync config present but disabled".to_string(),
                        suggestion: None,
                        span: None,
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
                    span: None,
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
            span: None,
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
                span: None,
            });
        } else {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: format!("compat.shell.{shell_name}.missing"),
                entry_index: None,
                field: Some(shell_name.to_string()),
                severity: DiagnosticSeverity::Info,
                message: format!("{shell_name} not found on PATH"),
                suggestion: None,
                span: None,
            });
        }
    }

    // Release 1: Check snp select availability
    let select_available = std::process::Command::new("snp")
        .args(["select", "--help"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if select_available {
        report.diagnostics.push(CompatibilityDiagnostic {
            code: "compat.select.ok".to_string(),
            entry_index: None,
            field: Some("select".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "snp select command is available".to_string(),
            suggestion: None,
            span: None,
        });
    } else {
        report.diagnostics.push(CompatibilityDiagnostic {
            code: "compat.select.missing".to_string(),
            entry_index: None,
            field: Some("select".to_string()),
            severity: DiagnosticSeverity::Warning,
            message: "snp select command is not available".to_string(),
            suggestion: Some("Ensure snp is installed and on PATH".to_string()),
            span: None,
        });
    }

    // Release 2: Check acquisition flags in snp new --help
    let new_help = std::process::Command::new("snp")
        .args(["new", "--help"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    let has_command_stdin = new_help.contains("--command-stdin");
    let has_from_file = new_help.contains("--from-file");
    let has_editor_flag = new_help.contains("--editor");

    report.diagnostics.push(CompatibilityDiagnostic {
        code: "compat.acquire.command_stdin".to_string(),
        entry_index: None,
        field: Some("acquire".to_string()),
        severity: if has_command_stdin {
            DiagnosticSeverity::Info
        } else {
            DiagnosticSeverity::Warning
        },
        message: if has_command_stdin {
            "snp new supports --command-stdin".to_string()
        } else {
            "snp new missing --command-stdin flag".to_string()
        },
        suggestion: if has_command_stdin {
            None
        } else {
            Some("Upgrade snp to a version with --command-stdin support".to_string())
        },
        span: None,
    });

    report.diagnostics.push(CompatibilityDiagnostic {
        code: "compat.acquire.from_file".to_string(),
        entry_index: None,
        field: Some("acquire".to_string()),
        severity: if has_from_file {
            DiagnosticSeverity::Info
        } else {
            DiagnosticSeverity::Warning
        },
        message: if has_from_file {
            "snp new supports --from-file".to_string()
        } else {
            "snp new missing --from-file flag".to_string()
        },
        suggestion: if has_from_file {
            None
        } else {
            Some("Upgrade snp to a version with --from-file support".to_string())
        },
        span: None,
    });

    report.diagnostics.push(CompatibilityDiagnostic {
        code: "compat.acquire.editor".to_string(),
        entry_index: None,
        field: Some("acquire".to_string()),
        severity: if has_editor_flag {
            DiagnosticSeverity::Info
        } else {
            DiagnosticSeverity::Warning
        },
        message: if has_editor_flag {
            "snp new supports --editor".to_string()
        } else {
            "snp new missing --editor flag".to_string()
        },
        suggestion: if has_editor_flag {
            None
        } else {
            Some("Upgrade snp to a version with --editor support".to_string())
        },
        span: None,
    });

    // Release 3: Verify choice-variable parser
    let choice_test =
        crate::utils::variables::parse_variables("echo <color=|_red_||_green_||_blue_||>");
    let has_choice_support = choice_test.iter().any(|v| {
        matches!(
            v.kind,
            crate::utils::variables::VariableKind::Choices { .. }
        )
    });

    if has_choice_support {
        report.diagnostics.push(CompatibilityDiagnostic {
            code: "compat.choice_parser.ok".to_string(),
            entry_index: None,
            field: Some("choice_parser".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "Choice variable parser is functional".to_string(),
            suggestion: None,
            span: None,
        });
    } else {
        report.diagnostics.push(CompatibilityDiagnostic {
            code: "compat.choice_parser.missing".to_string(),
            entry_index: None,
            field: Some("choice_parser".to_string()),
            severity: DiagnosticSeverity::Warning,
            message: "Choice variable parser did not produce expected result".to_string(),
            suggestion: Some("Report this issue; choice variables may not work".to_string()),
            span: None,
        });
    }

    // Shell syntax validation: generate snp shell init and check syntax
    for shell_name in &["bash", "zsh", "fish"] {
        let shell_found = std::process::Command::new("which")
            .arg(shell_name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !shell_found {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: format!("compat.shell_syntax.{shell_name}.unavailable"),
                entry_index: None,
                field: Some(format!("shell_syntax_{shell_name}")),
                severity: DiagnosticSeverity::Info,
                message: format!("{shell_name} not found; skipping syntax check"),
                suggestion: None,
                span: None,
            });
            continue;
        }

        let init_output = std::process::Command::new("snp")
            .args(["shell", "init", shell_name])
            .output();

        match init_output {
            Ok(output) if output.status.success() => {
                let check_arg = match *shell_name {
                    "bash" | "zsh" => "-n",
                    "fish" => "--no-execute",
                    _ => unreachable!(),
                };

                let syntax_check = std::process::Command::new(shell_name)
                    .arg(check_arg)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .and_then(|mut child| {
                        use std::io::Write;
                        if let Some(ref mut stdin) = child.stdin {
                            stdin.write_all(&output.stdout)?;
                        }
                        child.wait()
                    });

                match syntax_check {
                    Ok(status) if status.success() => {
                        report.diagnostics.push(CompatibilityDiagnostic {
                            code: format!("compat.shell_syntax.{shell_name}.ok"),
                            entry_index: None,
                            field: Some(format!("shell_syntax_{shell_name}")),
                            severity: DiagnosticSeverity::Info,
                            message: format!("snp shell init {shell_name} passes syntax check"),
                            suggestion: None,
                            span: None,
                        });
                    }
                    _ => {
                        report.diagnostics.push(CompatibilityDiagnostic {
                            code: format!("compat.shell_syntax.{shell_name}.invalid"),
                            entry_index: None,
                            field: Some(format!("shell_syntax_{shell_name}")),
                            severity: DiagnosticSeverity::Warning,
                            message: format!("snp shell init {shell_name} produced invalid syntax"),
                            suggestion: Some(format!(
                                "Run 'snp shell init {shell_name}' and inspect the output"
                            )),
                            span: None,
                        });
                    }
                }
            }
            _ => {
                report.diagnostics.push(CompatibilityDiagnostic {
                    code: format!("compat.shell_syntax.{shell_name}.init_failed"),
                    entry_index: None,
                    field: Some(format!("shell_syntax_{shell_name}")),
                    severity: DiagnosticSeverity::Warning,
                    message: format!("snp shell init {shell_name} failed to generate output"),
                    suggestion: Some(format!(
                        "Run 'snp shell init {shell_name}' manually to diagnose"
                    )),
                    span: None,
                });
            }
        }
    }

    // Editor configuration
    let editor = std::env::var("EDITOR").ok();
    let visual = std::env::var("VISUAL").ok();

    if editor.is_some() || visual.is_some() {
        let editor_display = match (&visual, &editor) {
            (Some(v), Some(e)) => format!("VISUAL={v}, EDITOR={e}"),
            (Some(v), None) => format!("VISUAL={v}"),
            (None, Some(e)) => format!("EDITOR={e}"),
            (None, None) => unreachable!(),
        };
        report.diagnostics.push(CompatibilityDiagnostic {
            code: "compat.editor.ok".to_string(),
            entry_index: None,
            field: Some("editor".to_string()),
            severity: DiagnosticSeverity::Info,
            message: format!("Editor configured ({editor_display})"),
            suggestion: None,
            span: None,
        });
    } else {
        report.diagnostics.push(CompatibilityDiagnostic {
            code: "compat.editor.unset".to_string(),
            entry_index: None,
            field: Some("editor".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "Neither $EDITOR nor $VISUAL is set (snp new --editor will use vim)"
                .to_string(),
            suggestion: None,
            span: None,
        });
    }

    // Known legacy paths
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        let legacy_single_file = PathBuf::from(&home).join(".config/snippets.toml");
        if legacy_single_file.exists() {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: "compat.legacy.single_file".to_string(),
                entry_index: None,
                field: Some("legacy".to_string()),
                severity: DiagnosticSeverity::Info,
                message: format!(
                    "Legacy single-file config found: {}",
                    legacy_single_file.display()
                ),
                suggestion: Some(
                    "Consider migrating to the library-based layout with 'snp library create'"
                        .to_string(),
                ),
                span: None,
            });
        }

        let legacy_config_dir = PathBuf::from(&home).join(".snip-it");
        if legacy_config_dir.exists() {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: "compat.legacy.config_dir".to_string(),
                entry_index: None,
                field: Some("legacy".to_string()),
                severity: DiagnosticSeverity::Info,
                message: format!(
                    "Legacy config directory found: {}",
                    legacy_config_dir.display()
                ),
                suggestion: Some(
                    "Consider migrating to ~/.config/snp/ and removing the old directory"
                        .to_string(),
                ),
                span: None,
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

/// Resolve a library name to a TOML file path.
///
/// Checks `~/.config/snp/libraries/<name>.toml` first, then treats as a literal path.
fn resolve_library_path(name: &str) -> SnipResult<PathBuf> {
    let as_literal = PathBuf::from(name);
    if as_literal.is_file() {
        return Ok(as_literal);
    }

    let config_dir = crate::utils::config::get_config_dir();
    let libraries_dir = config_dir.join("libraries");
    let by_name = libraries_dir.join(format!("{name}.toml"));
    if by_name.is_file() {
        return Ok(by_name);
    }

    Err(SnipError::runtime_error(
        "Library not found",
        Some(&format!(
            "'{name}' is not a file and no library named '{name}' exists in {}",
            libraries_dir.display()
        )),
    ))
}

/// Validate shell init output by piping through the shell's syntax checker.
fn check_shell_init(shell_name: &str, report: &mut DoctorReport, strict: bool) {
    let shell_type = match shell_name {
        "bash" => ShellType::Bash,
        "zsh" => ShellType::Zsh,
        "fish" => ShellType::Fish,
        _ => {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: format!("compat.shell_init.{shell_name}.unknown"),
                entry_index: None,
                field: Some(shell_name.to_string()),
                severity: DiagnosticSeverity::Warning,
                message: format!("Unknown shell type: {shell_name}"),
                suggestion: Some("Use bash, zsh, or fish".to_string()),
                span: None,
            });
            return;
        }
    };

    // Generate the init code
    let code = match shell_type {
        ShellType::Bash => shell_cmd::generate_bash(),
        ShellType::Zsh => shell_cmd::generate_zsh(),
        ShellType::Fish => shell_cmd::generate_fish(),
    };

    // Check if shell is available
    let shell_found = std::process::Command::new("which")
        .arg(shell_name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !shell_found {
        let severity = if strict {
            DiagnosticSeverity::Error
        } else {
            DiagnosticSeverity::Warning
        };
        report.diagnostics.push(CompatibilityDiagnostic {
            code: format!("compat.shell_init.{shell_name}.unavailable"),
            entry_index: None,
            field: Some(shell_name.to_string()),
            severity,
            message: format!("{shell_name} not found on PATH; cannot validate init output"),
            suggestion: Some(format!(
                "Install {shell_name} to enable shell init validation"
            )),
            span: None,
        });
        return;
    }

    // Write init code to a temp file for syntax checking
    let tmp = match tempfile::NamedTempFile::new() {
        Ok(t) => t,
        Err(e) => {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: format!("compat.shell_init.{shell_name}.error"),
                entry_index: None,
                field: Some(shell_name.to_string()),
                severity: DiagnosticSeverity::Warning,
                message: format!("Failed to create temp file for shell check: {e}"),
                suggestion: None,
                span: None,
            });
            return;
        }
    };
    if let Err(e) = std::fs::write(tmp.path(), &code) {
        report.diagnostics.push(CompatibilityDiagnostic {
            code: format!("compat.shell_init.{shell_name}.error"),
            entry_index: None,
            field: Some(shell_name.to_string()),
            severity: DiagnosticSeverity::Warning,
            message: format!("Failed to write temp file for shell check: {e}"),
            suggestion: None,
            span: None,
        });
        return;
    }

    // Run syntax check
    let output = match shell_type {
        ShellType::Bash => std::process::Command::new("bash")
            .args(["-n", tmp.path().to_str().unwrap()])
            .output(),
        ShellType::Zsh => std::process::Command::new("zsh")
            .args(["-n", tmp.path().to_str().unwrap()])
            .output(),
        ShellType::Fish => std::process::Command::new("fish")
            .args(["--no-execute", tmp.path().to_str().unwrap()])
            .output(),
    };

    match output {
        Ok(o) if o.status.success() => {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: format!("compat.shell_init.{shell_name}.ok"),
                entry_index: None,
                field: Some(shell_name.to_string()),
                severity: DiagnosticSeverity::Info,
                message: format!("{shell_name} init syntax valid ({} bytes)", code.len()),
                suggestion: None,
                span: None,
            });
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let severity = if strict {
                DiagnosticSeverity::Error
            } else {
                DiagnosticSeverity::Warning
            };
            report.diagnostics.push(CompatibilityDiagnostic {
                code: format!("compat.shell_init.{shell_name}.invalid"),
                entry_index: None,
                field: Some(shell_name.to_string()),
                severity,
                message: format!("{shell_name} init syntax check failed: {stderr}"),
                suggestion: Some("Run 'snp shell init <shell>' and review the output".to_string()),
                span: None,
            });
        }
        Err(e) => {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: format!("compat.shell_init.{shell_name}.error"),
                entry_index: None,
                field: Some(shell_name.to_string()),
                severity: DiagnosticSeverity::Warning,
                message: format!("Failed to run {shell_name} syntax check: {e}"),
                suggestion: None,
                span: None,
            });
        }
    }
}

/// Execute the doctor command.
pub fn run(
    pet_file: Option<PathBuf>,
    compatibility: bool,
    check_shell: Option<String>,
    library: Option<String>,
    strict: bool,
    report_format: DiagnosticReportFormat,
) -> SnipResult<()> {
    let has_file_mode = pet_file.is_some() || library.is_some();
    let has_mode = has_file_mode || compatibility || check_shell.is_some();

    if !has_mode {
        return Err(SnipError::runtime_error(
            "No mode selected",
            Some(
                "Specify --pet-file <path>, --library <name>, --compatibility, or --check-shell <shell>",
            ),
        ));
    }

    let mut report = if let Some(ref path) = pet_file {
        let content = read_source_file(path)?;

        if content.trim().is_empty() {
            return Err(SnipError::runtime_error(
                "Empty source file",
                Some("Source file contains no data"),
            ));
        }

        build_pet_report(path, &content, strict)?
    } else if let Some(ref lib_name) = library {
        let path = resolve_library_path(lib_name)?;
        let content = read_source_file(&path)?;

        if content.trim().is_empty() {
            return Err(SnipError::runtime_error(
                "Empty library file",
                Some("Library file contains no data"),
            ));
        }

        build_pet_report(&path, &content, strict)?
    } else if compatibility {
        build_compatibility_report(strict)?
    } else {
        DoctorReport::new(strict)
    };

    // Run shell init checks if requested
    if let Some(ref shell_name) = check_shell {
        check_shell_init(shell_name, &mut report, strict);
    }

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
