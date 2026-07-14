use crate::diagnostics::{
    CompatibilityDiagnostic, DiagnosticSeverity, ImportDuplicate, NormalizationRecord,
    PetImportReport,
};
use crate::error::{SnipError, SnipResult};
use crate::library::{LibraryManager, Snippet, Snippets};
use crate::utils::toml_helpers::fix_invalid_toml_escapes;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAX_IMPORT_FILE_BYTES: usize = 16 * 1024 * 1024;

/// Import mode determines how the importer handles destination state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub enum ImportMode {
    /// Fail if the destination library already exists.
    #[default]
    Create,
    /// Import into an existing library, skipping exact duplicates.
    Merge,
    /// Replace the destination library entirely (with backup).
    Replace,
}

/// Output format for the import report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub enum ReportFormat {
    /// Human-readable report to stderr.
    #[default]
    Human,
    /// Machine-readable JSON to stdout.
    Json,
}

/// Options for a pet import operation.
#[derive(Debug, Clone)]
pub struct PetImportOptions {
    pub source: PathBuf,
    pub destination_library: Option<String>,
    pub mode: ImportMode,
    pub strict: bool,
    pub dry_run: bool,
    pub report_format: ReportFormat,
    pub report_file: Option<PathBuf>,
}

/// Read and validate a pet TOML source file.
///
/// Returns the raw file content. The source is never modified.
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
        .take((MAX_IMPORT_FILE_BYTES as u64) + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| SnipError::io_error("read source file", path, e))?;

    if bytes.len() > MAX_IMPORT_FILE_BYTES {
        return Err(SnipError::runtime_error(
            "Source file too large",
            Some(&format!(
                "Pet import files are limited to {} MiB",
                MAX_IMPORT_FILE_BYTES / (1024 * 1024)
            )),
        ));
    }

    let content = String::from_utf8(bytes).map_err(|_| {
        SnipError::runtime_error(
            "Invalid source file",
            Some("Pet source file must be valid UTF-8"),
        )
    })?;

    if content.contains('\0') {
        return Err(SnipError::runtime_error(
            "Invalid source file",
            Some("Pet source file cannot contain NUL bytes"),
        ));
    }

    Ok(content)
}

/// Parse raw TOML content into a `Snippets` collection.
fn parse_pet_toml(content: &str) -> SnipResult<Snippets> {
    let fixed = fix_invalid_toml_escapes(content);
    toml::from_str(&fixed).map_err(|e| SnipError::toml_error("parse pet TOML", e))
}

/// Known field names for pet snippet entries (canonical + aliases).
/// Used to detect unknown fields in the source TOML.
const KNOWN_SNIPPET_FIELDS: &[&str] = &[
    // Canonical snip-it fields
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
    // Pet aliases
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

/// Detect unknown fields, missing required keys, and structural issues in
/// the raw TOML snippet entries. Returns diagnostics for the report.
fn detect_unknown_fields(raw_toml: &str) -> Vec<CompatibilityDiagnostic> {
    let mut diagnostics = Vec::new();

    let value: toml::Value = match toml::from_str(raw_toml) {
        Ok(v) => v,
        Err(_) => return diagnostics, // Parse errors are caught earlier by parse_pet_toml
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

        // Check for unknown fields
        for key in table.keys() {
            if !KNOWN_SNIPPET_FIELDS.contains(&key.as_str()) {
                diagnostics.push(CompatibilityDiagnostic {
                    entry_index: Some(i),
                    field: Some(key.clone()),
                    severity: DiagnosticSeverity::Info,
                    message: format!("Unknown field '{}' will be ignored", key),
                    code: "I-FIELD-UNKNOWN".to_string(),
                    suggestion: None,
                    span: None,
                });
            }
        }

        // Check for missing required fields
        if !table.contains_key("description")
            && !table.contains_key("Description")
            && !table.contains_key("name")
        {
            diagnostics.push(CompatibilityDiagnostic {
                entry_index: Some(i),
                field: Some("description".to_string()),
                severity: DiagnosticSeverity::Warning,
                message: "Entry missing 'description' field (will be empty)".to_string(),
                code: "W-DESC-MISSING".to_string(),
                suggestion: None,
                span: None,
            });
        }

        if !table.contains_key("command")
            && !table.contains_key("Command")
            && !table.contains_key("cmd")
        {
            diagnostics.push(CompatibilityDiagnostic {
                entry_index: Some(i),
                field: Some("command".to_string()),
                severity: DiagnosticSeverity::Warning,
                message: "Entry missing 'command' field (will be empty)".to_string(),
                code: "W-CMD-MISSING".to_string(),
                suggestion: None,
                span: None,
            });
        }
    }

    diagnostics
}

/// Convert a pet snippet into a native snip-it `Snippet`.
///
/// Preserves command text semantically. Generates snip-it-native fields
/// (IDs, timestamps, sync metadata defaults). Records diagnostics for
/// any normalization performed.
fn convert_entry(
    index: usize,
    pet: &Snippet,
) -> (
    Snippet,
    Vec<CompatibilityDiagnostic>,
    Vec<NormalizationRecord>,
) {
    let mut diagnostics = Vec::new();
    let normalizations = Vec::new();
    let mut snippet = pet.clone();

    // Generate a fresh UUID for the imported snippet
    snippet.id = uuid::Uuid::new_v4().to_string();

    // Ensure timestamps are set
    let now = chrono::Utc::now().timestamp();
    if snippet.created_at == 0 {
        snippet.created_at = now;
    }
    if snippet.updated_at == 0 {
        snippet.updated_at = now;
    }

    // Clear sync-only fields
    snippet.device_id = String::new();
    snippet.deleted = false;

    // Diagnostic: empty description
    if snippet.description.trim().is_empty() {
        diagnostics.push(CompatibilityDiagnostic {
            entry_index: Some(index),
            field: Some("description".to_string()),
            severity: DiagnosticSeverity::Warning,
            message: "Entry has empty description".to_string(),
            code: "W-DESC-EMPTY".to_string(),
            suggestion: None,
            span: None,
        });
    }

    // Diagnostic: empty command (would have been rejected by Snippet::new)
    if snippet.command.trim().is_empty() {
        diagnostics.push(CompatibilityDiagnostic {
            entry_index: Some(index),
            field: Some("command".to_string()),
            severity: DiagnosticSeverity::Error,
            message: "Entry has empty command".to_string(),
            code: "E-CMD-EMPTY".to_string(),
            suggestion: None,
            span: None,
        });
    }

    // Normalization: single tag string to array (pet sometimes uses a bare string)
    // This is already handled by serde aliases — tags is always Vec<String> after deserialization.

    // Diagnostic: output field present
    if !snippet.output.is_empty() {
        diagnostics.push(CompatibilityDiagnostic {
            entry_index: Some(index),
            field: Some("output".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "Entry has output field (preserved)".to_string(),
            code: "I-OUTPUT-PRESENT".to_string(),
            suggestion: None,
            span: None,
        });
    }

    // Diagnostic: empty tags array
    if snippet.tags.is_empty() {
        diagnostics.push(CompatibilityDiagnostic {
            entry_index: Some(index),
            field: Some("tag".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "Entry has no tags".to_string(),
            code: "I-TAGS-EMPTY".to_string(),
            suggestion: None,
            span: None,
        });
    }

    // Diagnostic: choice variables detected
    let vars = crate::utils::variables::parse_variables(&snippet.command);
    if vars.iter().any(|v| {
        matches!(
            v.kind,
            crate::utils::variables::VariableKind::Choices { .. }
        )
    }) {
        diagnostics.push(CompatibilityDiagnostic {
            entry_index: Some(index),
            field: Some("command".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "Entry contains choice variables".to_string(),
            code: "I-VAR-CHOICES".to_string(),
            suggestion: None,
            span: None,
        });
    }

    (snippet, diagnostics, normalizations)
}

/// Derive a library name from the source file path.
fn derive_library_name(source: &Path) -> String {
    let stem = source.file_stem().and_then(|s| s.to_str()).unwrap_or("");

    if stem.is_empty() {
        return "imported".to_string();
    }

    // Sanitize: lowercase, replace non-alphanumeric with hyphens, collapse runs
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

    // Collapse multiple hyphens
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

    // Trim leading/trailing hyphens
    let result = result.trim_matches('-');
    if result.is_empty() {
        "imported".to_string()
    } else {
        result.to_string()
    }
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

/// Execute the pet import operation.
pub fn run_import_pet(options: PetImportOptions) -> SnipResult<()> {
    // Phase 1: Read and validate source
    let content = read_source_file(&options.source)?;

    if content.trim().is_empty() {
        return Err(SnipError::runtime_error(
            "Empty source file",
            Some("Pet source file contains no data"),
        ));
    }

    let pet_snippets = parse_pet_toml(&content)?;

    if pet_snippets.snippets.is_empty() {
        return Err(SnipError::runtime_error(
            "No snippets found",
            Some("Pet source file contains no [[snippets]] entries"),
        ));
    }

    // Detect unknown fields and missing keys in raw TOML
    let unknown_field_diagnostics = detect_unknown_fields(&content);

    // Phase 2: Initialize library manager
    let mut mgr = LibraryManager::new()?;
    mgr.ensure_library_mode()?;

    // Phase 3: Determine destination
    let dest_name = match &options.destination_library {
        Some(name) => name.clone(),
        None => derive_library_name(&options.source),
    };

    let dest_path = mgr.get_libraries_dir().join(format!("{dest_name}.toml"));

    // Phase 4: Convert all entries (in memory)
    let mut report = PetImportReport::new(
        options.source.to_str().unwrap_or(""),
        Some(&dest_name),
        options.dry_run,
        options.strict,
    );
    report.total_entries = pet_snippets.snippets.len();

    // Add unknown-field and missing-key diagnostics from raw TOML analysis
    report.diagnostics.extend(unknown_field_diagnostics);

    let mut converted: Vec<Snippet> = Vec::new();

    for (i, pet) in pet_snippets.snippets.iter().enumerate() {
        let (snippet, diagnostics, normalizations) = convert_entry(i, pet);

        // Check for empty command (fatal in strict mode)
        if snippet.command.trim().is_empty() {
            report.diagnostics.extend(diagnostics);
            report.normalizations.extend(normalizations);
            report.skipped += 1;
            report.had_fatal_error = true;
            if options.strict {
                return Err(SnipError::runtime_error(
                    "Import aborted",
                    Some(&format!("Entry {} has empty command (strict mode)", i)),
                ));
            }
            continue;
        }

        report.diagnostics.extend(diagnostics);
        report.normalizations.extend(normalizations);
        converted.push(snippet);
    }

    // Phase 5: Handle destination based on mode
    match options.mode {
        ImportMode::Create => {
            if dest_path.exists() {
                return Err(SnipError::runtime_error(
                    "Destination already exists",
                    Some(&format!(
                        "Library '{dest_name}' already exists. Use --merge or --replace, or specify a different --library name."
                    )),
                ));
            }

            if !options.dry_run {
                // Write the library file atomically
                let snippets = Snippets {
                    snippets: converted.clone(),
                    folders: Vec::new(),
                };
                crate::library::save_library(&dest_path, &snippets)?;

                // Register in config
                let is_first = mgr.list_libraries().is_empty();
                let mut meta = crate::library::LibraryMeta::new(&dest_name);
                meta.is_primary = is_first;
                // We need a mutable mgr to save config — but save_library is a free function.
                // We'll use the mgr to register after saving.
            }

            report.imported = converted.len();
        }
        ImportMode::Merge => {
            let existing = if dest_path.exists() {
                crate::library::load_library(&dest_path)?
            } else {
                Snippets::default()
            };

            // Track which source entries to skip (exact duplicates)
            let mut skip_indices: std::collections::HashSet<usize> =
                std::collections::HashSet::new();

            // Detect duplicates against existing library
            for (i, candidate) in converted.iter().enumerate() {
                for (j, existing_snippet) in existing.snippets.iter().enumerate() {
                    if is_exact_duplicate(candidate, existing_snippet) {
                        report.duplicates.push(ImportDuplicate {
                            source_index: i,
                            destination_index: j,
                            description: candidate.description.clone(),
                            reason: "Exact duplicate (same command and description)".to_string(),
                        });
                        skip_indices.insert(i);
                        break;
                    }
                }
                if !skip_indices.contains(&i) {
                    // Also check for same command, different description
                    for existing_snippet in &existing.snippets {
                        if same_command_different_description(candidate, existing_snippet) {
                            report.diagnostics.push(CompatibilityDiagnostic {
                                entry_index: Some(i),
                                field: Some("command".to_string()),
                                severity: DiagnosticSeverity::Warning,
                                message: format!(
                                    "Same command as existing '{}' but different description",
                                    existing_snippet.description
                                ),
                                code: "W-CMD-DUPLICATE".to_string(),
                                suggestion: None,
                                span: None,
                            });
                        }
                        if same_description_different_command(candidate, existing_snippet) {
                            report.diagnostics.push(CompatibilityDiagnostic {
                                entry_index: Some(i),
                                field: Some("description".to_string()),
                                severity: DiagnosticSeverity::Warning,
                                message: "Same description as existing entry but different command"
                                    .to_string(),
                                code: "W-DESC-DUPLICATE".to_string(),
                                suggestion: None,
                                span: None,
                            });
                        }
                    }
                }
            }

            // Filter out exact duplicates
            let mut merged: Vec<Snippet> = existing.snippets.clone();
            let mut imported_count = 0;
            for (i, candidate) in converted.iter().enumerate() {
                if skip_indices.contains(&i) {
                    report.skipped += 1;
                } else {
                    merged.push(candidate.clone());
                    imported_count += 1;
                }
            }

            report.imported = imported_count;

            if !options.dry_run {
                if !dest_path.exists() {
                    // New library
                    let snippets = Snippets {
                        snippets: merged,
                        folders: Vec::new(),
                    };
                    crate::library::save_library(&dest_path, &snippets)?;
                } else {
                    // Existing library — backup then save
                    crate::library::backup_library(&dest_path)?;
                    let snippets = Snippets {
                        snippets: merged,
                        folders: Vec::new(),
                    };
                    crate::library::save_library(&dest_path, &snippets)?;
                }
            }
        }
        ImportMode::Replace => {
            if !dest_path.exists() {
                return Err(SnipError::runtime_error(
                    "Destination does not exist",
                    Some(&format!(
                        "Cannot replace library '{dest_name}': it does not exist. Use --merge or omit --replace."
                    )),
                ));
            }

            if !options.dry_run {
                // Backup before replacement
                crate::library::backup_library(&dest_path)?;

                let snippets = Snippets {
                    snippets: converted.clone(),
                    folders: Vec::new(),
                };
                crate::library::save_library(&dest_path, &snippets)?;
            }

            report.imported = converted.len();
        }
    }

    // Phase 6: Register library in config (for create/merge of new libraries)
    if !options.dry_run && !dest_path.exists() && options.mode != ImportMode::Merge {
        // This case shouldn't happen, but handle defensively
    }
    if !options.dry_run && dest_path.exists() && mgr.get_library_by_filename(&dest_name).is_none() {
        let is_first = mgr.list_libraries().is_empty();
        let mut meta = crate::library::LibraryMeta::new(&dest_name);
        meta.is_primary = is_first;
        // We need to push to config and save
        // LibraryManager's config field is private, so we use add_existing_library
        mgr.add_existing_library(&dest_name)?;
    }

    // Phase 7: Emit report
    match options.report_format {
        ReportFormat::Human => {
            eprintln!();
            eprintln!("Pet Import Report");
            eprintln!("=================");
            eprintln!("Source:      {}", report.source);
            if let Some(ref dest) = report.destination {
                eprintln!("Destination: {dest}");
            }
            eprintln!("Total:       {} entries", report.total_entries);
            eprintln!("Imported:    {}", report.imported);
            eprintln!("Skipped:     {}", report.skipped);
            if !report.duplicates.is_empty() {
                eprintln!("Duplicates:  {}", report.duplicates.len());
                for dup in &report.duplicates {
                    eprintln!(
                        "  [{}] {} — {}",
                        dup.source_index, dup.description, dup.reason
                    );
                }
            }
            if !report.diagnostics.is_empty() {
                eprintln!("Diagnostics: {}", report.diagnostics.len());
                for diag in &report.diagnostics {
                    let icon = match diag.severity {
                        DiagnosticSeverity::Info => "i",
                        DiagnosticSeverity::Warning => "w",
                        DiagnosticSeverity::Error => "e",
                    };
                    eprintln!(
                        "  [{icon}] [{}] {}: {}",
                        diag.entry_index.unwrap_or(0),
                        diag.field.as_deref().unwrap_or("-"),
                        diag.message
                    );
                }
            }
            if !report.normalizations.is_empty() {
                eprintln!("Normalized:  {}", report.normalizations.len());
                for norm in &report.normalizations {
                    eprintln!(
                        "  [{}] {}: '{}' -> '{}'",
                        norm.entry_index, norm.field, norm.original, norm.normalized
                    );
                }
            }
            if options.dry_run {
                eprintln!();
                eprintln!("(dry run — no files were modified)");
            } else {
                eprintln!();
                eprintln!("Import complete.");
            }
        }
        ReportFormat::Json => {
            let json = serde_json::to_string_pretty(&report).map_err(|e| {
                SnipError::runtime_error("Failed to serialize report", Some(&e.to_string()))
            })?;
            println!("{json}");
        }
    }

    // Phase 8: Write report file if requested
    if let Some(ref report_path) = options.report_file {
        let json = serde_json::to_string_pretty(&report).map_err(|e| {
            SnipError::runtime_error("Failed to serialize report", Some(&e.to_string()))
        })?;
        if !options.dry_run {
            crate::utils::atomic::write_private_atomic(report_path, &json, "import-report")?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_library_name_from_filename() {
        assert_eq!(
            derive_library_name(Path::new("/tmp/my-snippets.toml")),
            "my-snippets"
        );
        assert_eq!(
            derive_library_name(Path::new("/tmp/Pet Export.toml")),
            "pet-export"
        );
        assert_eq!(
            derive_library_name(Path::new("/tmp/MY_SNIPPETS.toml")),
            "my-snippets"
        );
    }

    #[test]
    fn test_derive_library_name_special_chars() {
        assert_eq!(
            derive_library_name(Path::new("/tmp/my@snippets!.toml")),
            "my-snippets"
        );
        assert_eq!(
            derive_library_name(Path::new("/tmp/---leading.toml")),
            "leading"
        );
        assert_eq!(
            derive_library_name(Path::new("/tmp/trailing---.toml")),
            "trailing"
        );
    }

    #[test]
    fn test_derive_library_name_fallback() {
        // .toml file_stem() returns "toml" (the dot is treated as a leading dot)
        assert_eq!(derive_library_name(Path::new("/tmp/.toml")), "toml");
    }

    #[test]
    fn test_is_exact_duplicate() {
        let a = Snippet {
            command: "echo hi".to_string(),
            description: "greeting".to_string(),
            ..Default::default()
        };
        let b = Snippet {
            command: "echo hi".to_string(),
            description: "greeting".to_string(),
            ..Default::default()
        };
        let c = Snippet {
            command: "echo hi".to_string(),
            description: "different".to_string(),
            ..Default::default()
        };
        assert!(is_exact_duplicate(&a, &b));
        assert!(!is_exact_duplicate(&a, &c));
    }

    #[test]
    fn test_same_command_different_description() {
        let a = Snippet {
            command: "echo hi".to_string(),
            description: "greeting".to_string(),
            ..Default::default()
        };
        let b = Snippet {
            command: "echo hi".to_string(),
            description: "other".to_string(),
            ..Default::default()
        };
        assert!(same_command_different_description(&a, &b));
        assert!(!same_command_different_description(&a, &a));
    }

    #[test]
    fn test_same_description_different_command() {
        let a = Snippet {
            command: "echo hi".to_string(),
            description: "greeting".to_string(),
            ..Default::default()
        };
        let b = Snippet {
            command: "echo bye".to_string(),
            description: "greeting".to_string(),
            ..Default::default()
        };
        assert!(same_description_different_command(&a, &b));
        assert!(!same_description_different_command(&a, &a));
    }

    #[test]
    fn test_read_source_file_missing() {
        let result = read_source_file(Path::new("/nonexistent/file.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_read_source_file_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let result = read_source_file(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_pet_toml_valid() {
        let toml = r#"
[[snippets]]
description = "test"
command = "echo hello"
tag = ["test"]
"#;
        let result = parse_pet_toml(toml).unwrap();
        assert_eq!(result.snippets.len(), 1);
        assert_eq!(result.snippets[0].command, "echo hello");
    }

    #[test]
    fn test_parse_pet_toml_invalid() {
        let result = parse_pet_toml("invalid = [toml");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_pet_toml_empty() {
        let result = parse_pet_toml("").unwrap();
        assert!(result.snippets.is_empty());
    }

    #[test]
    fn test_convert_entry_sets_id_and_timestamps() {
        let pet = Snippet {
            description: "test".to_string(),
            command: "echo hi".to_string(),
            tags: vec!["tag1".to_string()],
            ..Default::default()
        };
        let (snippet, _, _) = convert_entry(0, &pet);
        assert!(!snippet.id.is_empty());
        assert!(snippet.created_at > 0);
        assert!(snippet.updated_at > 0);
        assert!(!snippet.deleted);
        assert!(snippet.device_id.is_empty());
    }

    #[test]
    fn test_convert_entry_preserves_command() {
        let pet = Snippet {
            description: "test".to_string(),
            command: "echo 'hello world' | grep hello".to_string(),
            ..Default::default()
        };
        let (snippet, _, _) = convert_entry(0, &pet);
        assert_eq!(snippet.command, "echo 'hello world' | grep hello");
    }

    #[test]
    fn test_convert_entry_empty_command_diagnostic() {
        let pet = Snippet {
            description: "test".to_string(),
            command: "  ".to_string(),
            ..Default::default()
        };
        let (snippet, diagnostics, _) = convert_entry(0, &pet);
        assert!(snippet.command.trim().is_empty());
        assert!(
            diagnostics
                .iter()
                .any(|d| d.severity == DiagnosticSeverity::Error)
        );
    }

    #[test]
    fn test_convert_entry_output_diagnostic() {
        let pet = Snippet {
            description: "test".to_string(),
            command: "echo hi".to_string(),
            output: "hi".to_string(),
            ..Default::default()
        };
        let (_, diagnostics, _) = convert_entry(0, &pet);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.field.as_deref() == Some("output"))
        );
    }

    #[test]
    fn test_convert_entry_empty_tags_diagnostic() {
        let pet = Snippet {
            description: "test".to_string(),
            command: "echo hi".to_string(),
            tags: Vec::new(),
            ..Default::default()
        };
        let (_, diagnostics, _) = convert_entry(0, &pet);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.field.as_deref() == Some("tag") && d.message.contains("no tags"))
        );
    }

    #[test]
    fn test_detect_unknown_fields() {
        let toml = r#"
[[snippets]]
description = "test"
command = "echo hi"
custom_field = "unknown"
another_unknown = 42
"#;
        let diagnostics = detect_unknown_fields(toml);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("custom_field")),
            "Should detect custom_field as unknown"
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("another_unknown")),
            "Should detect another_unknown as unknown"
        );
        // Known fields should not be flagged
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.message.contains("description")),
            "description should not be flagged as unknown"
        );
        assert!(
            !diagnostics.iter().any(|d| d.message.contains("command")),
            "command should not be flagged as unknown"
        );
    }

    #[test]
    fn test_detect_missing_description() {
        let toml = r#"
[[snippets]]
command = "echo hi"
"#;
        let diagnostics = detect_unknown_fields(toml);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("missing 'description'")),
            "Should detect missing description"
        );
    }

    #[test]
    fn test_detect_missing_command() {
        let toml = r#"
[[snippets]]
description = "test"
"#;
        let diagnostics = detect_unknown_fields(toml);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.contains("missing 'command'")),
            "Should detect missing command"
        );
    }

    #[test]
    fn test_detect_known_pet_aliases() {
        let toml = r#"
[[snippets]]
Description = "legacy"
Command = "echo legacy"
Tag = ["legacy"]
Output = "out"
"#;
        let diagnostics = detect_unknown_fields(toml);
        // These are known aliases, should not be flagged as unknown
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.message.contains("Description")),
            "Description alias should not be flagged"
        );
        assert!(
            !diagnostics.iter().any(|d| d.message.contains("Command")),
            "Command alias should not be flagged"
        );
    }
}
