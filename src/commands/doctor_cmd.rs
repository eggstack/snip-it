use crate::commands::pet_analysis::{
    analyze_entry, detect_duplicates, detect_unknown_fields, parse_pet_toml, read_source_file,
};
use crate::commands::shell_cmd::{self, ShellType};
use crate::diagnostics::{
    CompatibilityDiagnostic, DiagnosticSeverity, DoctorReport, NormalizationRecord,
    diagnostic_counts, version,
};
use crate::error::{SnipError, SnipResult};
use crate::library::{LibraryManager, Snippets};
use std::fs;
use std::path::{Path, PathBuf};

/// Output format for the doctor report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub enum DiagnosticReportFormat {
    #[default]
    Human,
    Json,
}

/// Designated warnings that `--strict` elevates to errors in pet-file analysis.
const STRICT_WARNING_CODES: &[&str] = &[
    "W-MALFORMED-VAR",
    "W-DUP-CMD",
    "W-DUP-DESC",
    "W-DEST-CONFLICT",
    "W-DESC-MISSING",
    "W-CMD-MISSING",
    "W-DESC-EMPTY",
    "W-TYPE-MISMATCH",
    "W-TAG-EMPTY",
];

/// Elevate designated warning diagnostics to errors when `--strict` is active.
fn apply_strict_elevation(report: &mut DoctorReport) {
    if !report.strict_mode {
        return;
    }
    for diag in &mut report.diagnostics {
        if diag.severity == DiagnosticSeverity::Warning
            && STRICT_WARNING_CODES.contains(&diag.code.as_str())
        {
            diag.severity = DiagnosticSeverity::Error;
        }
    }
}

/// Build a DoctorReport from a pet file analysis.
fn build_pet_report(
    source_path: &Path,
    content: &str,
    strict: bool,
    existing_library_names: &[String],
) -> SnipResult<DoctorReport> {
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

    // Detect unsupported pet-specific concepts
    detect_unsupported_concepts(&pet_snippets, &mut report);

    // Detect destination naming conflicts
    let lib_name = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("imported");
    let sanitized = sanitize_library_name(lib_name);
    if existing_library_names.contains(&sanitized) {
        report.diagnostics.push(CompatibilityDiagnostic {
            code: "W-DEST-CONFLICT".to_string(),
            entry_index: None,
            field: Some("destination".to_string()),
            severity: DiagnosticSeverity::Warning,
            message: format!(
                "Library '{}' already exists; import will need --merge or --replace",
                sanitized
            ),
            suggestion: Some("Use --library <different-name> or --merge/--replace".to_string()),
            span: None,
        });
    }

    // Detect duplicates within the source file
    let (duplicates, dup_diags) = detect_duplicates(&pet_snippets.snippets);
    report.duplicates.extend(duplicates);
    report.diagnostics.extend(dup_diags);

    // Populate normalization preview
    for (i, pet) in pet_snippets.snippets.iter().enumerate() {
        // Timestamp normalization
        if pet.created_at == 0 || pet.updated_at == 0 {
            report.normalizations.push(NormalizationRecord {
                entry_index: i,
                field: "timestamps".to_string(),
                original: format!(
                    "created_at={}, updated_at={}",
                    pet.created_at, pet.updated_at
                ),
                normalized: "will be set to current time".to_string(),
            });
        }
        // Sync field clearing
        if !pet.device_id.is_empty() || pet.deleted {
            report.normalizations.push(NormalizationRecord {
                entry_index: i,
                field: "sync_fields".to_string(),
                original: format!(
                    "device_id={}, deleted={}",
                    if pet.device_id.is_empty() {
                        "(empty)"
                    } else {
                        &pet.device_id
                    },
                    pet.deleted
                ),
                normalized: "device_id cleared, deleted=false".to_string(),
            });
        }
        // ID regeneration
        if !pet.id.is_empty() {
            report.normalizations.push(NormalizationRecord {
                entry_index: i,
                field: "id".to_string(),
                original: pet.id.clone(),
                normalized: "(will be regenerated)".to_string(),
            });
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

    apply_strict_elevation(&mut report);
    Ok(report)
}

/// Detect pet-specific concepts that snp cannot handle.
fn detect_unsupported_concepts(snippets: &Snippets, report: &mut DoctorReport) {
    for (i, pet) in snippets.snippets.iter().enumerate() {
        // Check for malformed variable placeholders (unmatched < without >)
        let command = &pet.command;
        let mut angle_depth = 0i32;
        let mut chars = command.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                // Skip escaped characters
                chars.next();
                continue;
            }
            if c == '<' {
                angle_depth += 1;
            } else if c == '>' {
                angle_depth -= 1;
                if angle_depth < 0 {
                    // Unmatched closing >
                    report.diagnostics.push(CompatibilityDiagnostic {
                        code: "W-MALFORMED-VAR".to_string(),
                        entry_index: Some(i),
                        field: Some("command".to_string()),
                        severity: DiagnosticSeverity::Warning,
                        message:
                            "Unmatched '>' in command; may be a malformed variable placeholder"
                                .to_string(),
                        suggestion: Some(
                            "Check variable syntax: use <name> or <name=default>".to_string(),
                        ),
                        span: None,
                    });
                    angle_depth = 0;
                }
            }
        }
        if angle_depth > 0 {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: "W-MALFORMED-VAR".to_string(),
                entry_index: Some(i),
                field: Some("command".to_string()),
                severity: DiagnosticSeverity::Warning,
                message: "Unmatched '<' in command; may be a malformed variable placeholder"
                    .to_string(),
                suggestion: Some("Check variable syntax: use <name> or <name=default>".to_string()),
                span: None,
            });
        }

        // Check for pet-specific fields that indicate unsupported concepts
        if !pet.folders.is_empty() {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: "I-FIELD-FOLDERS".to_string(),
                entry_index: Some(i),
                field: Some("folders".to_string()),
                severity: DiagnosticSeverity::Info,
                message: "Pet 'folders' field will be preserved as-is".to_string(),
                suggestion: None,
                span: None,
            });
        }
    }
}

/// Sanitize a filename stem into a valid library name.
fn sanitize_library_name(stem: &str) -> String {
    let sanitized: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let mut result = String::with_capacity(sanitized.len());
    let mut prev_hyphen = false;
    for c in sanitized.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push(c);
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    let result = result.trim_matches('-');
    if result.is_empty() {
        "imported".to_string()
    } else {
        result.to_string()
    }
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

    // Canonical Pet TOML loading check
    let pet_parse_test = parse_pet_toml(
        r#"
[[snippets]]
description = "test"
command = "echo ok"
"#,
    );
    match pet_parse_test {
        Ok(_) => {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: "compat.pet_toml.ok".to_string(),
                entry_index: None,
                field: Some("pet_toml".to_string()),
                severity: DiagnosticSeverity::Info,
                message: "Pet TOML parser is functional".to_string(),
                suggestion: None,
                span: None,
            });
        }
        Err(e) => {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: "compat.pet_toml.error".to_string(),
                entry_index: None,
                field: Some("pet_toml".to_string()),
                severity: DiagnosticSeverity::Error,
                message: format!("Pet TOML parser failed: {e}"),
                suggestion: Some("Report this issue; pet import may not work".to_string()),
                span: None,
            });
        }
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

    // Auto-sync state inspection (Release 5D corrective: detached one-shot worker)
    let state_dir = crate::auto_sync::paths::state_dir();
    let pending_path = crate::auto_sync::paths::pending_marker(&state_dir);
    let lock_path = crate::auto_sync::paths::worker_lock(&state_dir);

    // Check auto-sync pending state
    if pending_path.exists() {
        match crate::auto_sync::pending::read_state_from_dir(&state_dir) {
            Ok(state) => {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let age_ms = now_ms.saturating_sub(state.created_at_unix_ms);
                let is_stale = age_ms > crate::auto_sync::pending::STALE_PENDING_THRESHOLD_MS;

                if is_stale {
                    report.diagnostics.push(CompatibilityDiagnostic {
                        code: "compat.auto_sync.pending_stale".to_string(),
                        entry_index: None,
                        field: Some("auto_sync".to_string()),
                        severity: DiagnosticSeverity::Warning,
                        message: format!(
                            "Stale auto-sync pending state detected (generation {}, >5 min old)",
                            state.generation
                        ),
                        suggestion: Some(
                            "Run `snp sync` to recover, or it will be recovered on next mutation"
                                .to_string(),
                        ),
                        span: None,
                    });
                } else {
                    report.diagnostics.push(CompatibilityDiagnostic {
                        code: "compat.auto_sync.pending_active".to_string(),
                        entry_index: None,
                        field: Some("auto_sync".to_string()),
                        severity: DiagnosticSeverity::Info,
                        message: format!(
                            "Auto-sync pending state is active (generation {})",
                            state.generation
                        ),
                        suggestion: None,
                        span: None,
                    });
                }
            }
            Err(_) => {
                report.diagnostics.push(CompatibilityDiagnostic {
                    code: "compat.auto_sync.pending_unreadable".to_string(),
                    entry_index: None,
                    field: Some("auto_sync".to_string()),
                    severity: DiagnosticSeverity::Warning,
                    message: "Auto-sync pending state file exists but is unreadable".to_string(),
                    suggestion: None,
                    span: None,
                });
            }
        }
    }

    // Check auto-sync worker lock file
    if lock_path.exists() {
        match crate::auto_sync::lock::inspect(&lock_path) {
            Some(lock) => {
                let alive = crate::auto_sync::lock::process_alive(lock.pid);

                if alive {
                    report.diagnostics.push(CompatibilityDiagnostic {
                        code: "compat.auto_sync.lock_held".to_string(),
                        entry_index: None,
                        field: Some("auto_sync".to_string()),
                        severity: DiagnosticSeverity::Info,
                        message: format!(
                            "Auto-sync worker lock held by process {} (nonce {})",
                            lock.pid, lock.nonce
                        ),
                        suggestion: None,
                        span: None,
                    });
                } else {
                    report.diagnostics.push(CompatibilityDiagnostic {
                        code: "compat.auto_sync.lock_stale".to_string(),
                        entry_index: None,
                        field: Some("auto_sync".to_string()),
                        severity: DiagnosticSeverity::Warning,
                        message: format!(
                            "Auto-sync worker lock exists but process {} is dead (stale)",
                            lock.pid
                        ),
                        suggestion: Some(
                            "Stale lock will be recovered on next worker spawn or `snp sync`"
                                .to_string(),
                        ),
                        span: None,
                    });
                }
            }
            None => {
                report.diagnostics.push(CompatibilityDiagnostic {
                    code: "compat.auto_sync.lock_unreadable".to_string(),
                    entry_index: None,
                    field: Some("auto_sync".to_string()),
                    severity: DiagnosticSeverity::Warning,
                    message: "Auto-sync worker lock file exists but is unreadable".to_string(),
                    suggestion: None,
                    span: None,
                });
            }
        }
    }

    // Check auto-sync config
    if let Ok(settings) = crate::config::load_sync_settings() {
        if settings.auto_sync {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: "compat.auto_sync.enabled".to_string(),
                entry_index: None,
                field: Some("auto_sync".to_string()),
                severity: DiagnosticSeverity::Info,
                message: format!(
                    "Auto-sync enabled (debounce: {}s, failure: {})",
                    settings.auto_sync_debounce_seconds, settings.auto_sync_failure
                ),
                suggestion: None,
                span: None,
            });
        } else {
            report.diagnostics.push(CompatibilityDiagnostic {
                code: "compat.auto_sync.disabled".to_string(),
                entry_index: None,
                field: Some("auto_sync".to_string()),
                severity: DiagnosticSeverity::Info,
                message: "Auto-sync is disabled".to_string(),
                suggestion: None,
                span: None,
            });
        }
    }

    report.total_entries = 0;
    apply_strict_elevation(&mut report);
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

    if !report.normalizations.is_empty() {
        eprintln!();
        eprintln!("Normalizations ({}):", report.normalizations.len());
        for norm in &report.normalizations {
            eprintln!(
                "  [{}] {}: '{}' -> '{}'",
                norm.entry_index, norm.field, norm.original, norm.normalized
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

/// Get the list of existing library names for destination conflict detection.
fn get_existing_library_names() -> Vec<String> {
    match LibraryManager::new() {
        Ok(mgr) => mgr
            .list_libraries()
            .iter()
            .map(|l| l.filename.clone())
            .collect(),
        Err(_) => Vec::new(),
    }
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

    let existing_libs = get_existing_library_names();

    let mut report = if let Some(ref path) = pet_file {
        let content = read_source_file(path)?;

        if content.trim().is_empty() {
            return Err(SnipError::runtime_error(
                "Empty source file",
                Some("Source file contains no data"),
            ));
        }

        build_pet_report(path, &content, strict, &existing_libs)?
    } else if let Some(ref lib_name) = library {
        let path = resolve_library_path(lib_name)?;
        let content = read_source_file(&path)?;

        if content.trim().is_empty() {
            return Err(SnipError::runtime_error(
                "Empty library file",
                Some("Library file contains no data"),
            ));
        }

        build_pet_report(&path, &content, strict, &existing_libs)?
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Snippet;

    #[test]
    fn test_sanitize_library_name() {
        assert_eq!(sanitize_library_name("my-snippets"), "my-snippets");
        assert_eq!(sanitize_library_name("Pet Export"), "pet-export");
        assert_eq!(sanitize_library_name("MY_SNIPPETS"), "my-snippets");
        assert_eq!(sanitize_library_name("my@snippets!"), "my-snippets");
        assert_eq!(sanitize_library_name("---leading"), "leading");
        assert_eq!(sanitize_library_name("trailing---"), "trailing");
    }

    #[test]
    fn test_sanitize_library_name_fallback() {
        assert_eq!(sanitize_library_name(".toml"), "toml");
        assert_eq!(sanitize_library_name(""), "imported");
    }

    #[test]
    fn test_malformed_variable_unmatched_open() {
        let pet = Snippet {
            description: "test".to_string(),
            command: "echo <name".to_string(),
            ..Default::default()
        };
        let mut report = DoctorReport::new(false);
        let snippets = Snippets {
            snippets: vec![pet],
            folders: Vec::new(),
        };
        detect_unsupported_concepts(&snippets, &mut report);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "W-MALFORMED-VAR"),
            "Should detect malformed variable"
        );
    }

    #[test]
    fn test_malformed_variable_unmatched_close() {
        let pet = Snippet {
            description: "test".to_string(),
            command: "echo name>".to_string(),
            ..Default::default()
        };
        let mut report = DoctorReport::new(false);
        let snippets = Snippets {
            snippets: vec![pet],
            folders: Vec::new(),
        };
        detect_unsupported_concepts(&snippets, &mut report);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.code == "W-MALFORMED-VAR"),
            "Should detect malformed variable"
        );
    }

    #[test]
    fn test_malformed_variable_escaped() {
        let pet = Snippet {
            description: "test".to_string(),
            command: "echo \\<name\\>".to_string(),
            ..Default::default()
        };
        let mut report = DoctorReport::new(false);
        let snippets = Snippets {
            snippets: vec![pet],
            folders: Vec::new(),
        };
        detect_unsupported_concepts(&snippets, &mut report);
        assert!(
            !report
                .diagnostics
                .iter()
                .any(|d| d.code == "W-MALFORMED-VAR"),
            "Escaped angle brackets should not trigger warning"
        );
    }
}
