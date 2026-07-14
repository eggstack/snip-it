use crate::diagnostics::{CompatibilityDiagnostic, DiagnosticSeverity, ImportDuplicate};
use crate::error::{SnipError, SnipResult};
use crate::library::Snippet;
use crate::library::Snippets;
use crate::utils::toml_helpers::fix_invalid_toml_escapes;
use std::fs;
use std::io::Read;
use std::path::Path;

pub const MAX_SOURCE_FILE_BYTES: usize = 16 * 1024 * 1024;

/// Known field names for pet snippet entries (canonical + aliases).
pub const KNOWN_SNIPPET_FIELDS: &[&str] = &[
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

/// Read and validate a pet TOML source file.
///
/// Returns the raw file content. The source is never modified.
pub fn read_source_file(path: &Path) -> SnipResult<String> {
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
        .take((MAX_SOURCE_FILE_BYTES as u64) + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| SnipError::io_error("read source file", path, e))?;

    if bytes.len() > MAX_SOURCE_FILE_BYTES {
        return Err(SnipError::runtime_error(
            "Source file too large",
            Some(&format!(
                "Source files are limited to {} MiB",
                MAX_SOURCE_FILE_BYTES / (1024 * 1024)
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
pub fn parse_pet_toml(content: &str) -> SnipResult<Snippets> {
    let fixed = fix_invalid_toml_escapes(content);
    toml::from_str(&fixed).map_err(|e| SnipError::toml_error("parse pet TOML", e))
}

/// Detect unknown fields, type mismatches, missing required keys, and
/// structural issues in the raw TOML snippet entries.
pub fn detect_unknown_fields(raw_toml: &str) -> Vec<CompatibilityDiagnostic> {
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
            let val = &table[key];
            if KNOWN_SNIPPET_FIELDS.contains(&key.as_str()) {
                // Check expected types for known fields
                let expected = match key.as_str() {
                    "tag" | "tags" | "Tag" | "Tags" | "folders" => Some("array"),
                    "favorite" | "deleted" => Some("boolean"),
                    "created_at" | "updated_at" => Some("integer"),
                    "id" | "description" | "command" | "Command" | "cmd" | "output" | "Output"
                    | "device_id" | "name" | "Description" => Some("string"),
                    _ => None,
                };
                if let Some(expected_type) = expected {
                    let actual_type = match val {
                        toml::Value::String(_) => "string",
                        toml::Value::Integer(_) => "integer",
                        toml::Value::Float(_) => "float",
                        toml::Value::Boolean(_) => "boolean",
                        toml::Value::Array(_) => "array",
                        toml::Value::Table(_) => "table",
                        toml::Value::Datetime(_) => "datetime",
                    };
                    if actual_type != expected_type {
                        diagnostics.push(CompatibilityDiagnostic {
                            code: "W-TYPE-MISMATCH".to_string(),
                            entry_index: Some(i),
                            field: Some(key.clone()),
                            severity: DiagnosticSeverity::Warning,
                            message: format!(
                                "Field '{}' expected {} but got {}",
                                key, expected_type, actual_type
                            ),
                            suggestion: Some(format!("Use the correct type for field '{}'", key)),
                            span: None,
                        });
                    }
                }
            } else {
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

/// Analyze a single pet entry and produce diagnostics (read-only).
pub fn analyze_entry(index: usize, pet: &Snippet) -> Vec<CompatibilityDiagnostic> {
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
    } else {
        for (j, tag) in pet.tags.iter().enumerate() {
            if tag.trim().is_empty() {
                diagnostics.push(CompatibilityDiagnostic {
                    code: "W-TAG-EMPTY".to_string(),
                    entry_index: Some(index),
                    field: Some("tag".to_string()),
                    severity: DiagnosticSeverity::Warning,
                    message: format!("Tag at index {} is empty or whitespace-only", j),
                    suggestion: Some("Remove empty tags or provide a valid tag name".to_string()),
                    span: None,
                });
            }
        }
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
pub fn is_exact_duplicate(a: &Snippet, b: &Snippet) -> bool {
    a.command == b.command && a.description == b.description
}

/// Check if two snippets have the same command but different descriptions.
pub fn same_command_different_description(a: &Snippet, b: &Snippet) -> bool {
    a.command == b.command && a.description != b.description
}

/// Check if two snippets have the same description but different commands.
pub fn same_description_different_command(a: &Snippet, b: &Snippet) -> bool {
    a.description == b.description && a.command != b.command
}

/// Detect duplicates within a single list of snippets (self-to-self comparison).
///
/// Returns duplicates and diagnostics for same-command-different-desc and
/// same-description-different-command.
pub fn detect_duplicates(
    snippets: &[Snippet],
) -> (Vec<ImportDuplicate>, Vec<CompatibilityDiagnostic>) {
    let mut duplicates = Vec::new();
    let mut diagnostics = Vec::new();

    for i in 0..snippets.len() {
        for j in (i + 1)..snippets.len() {
            if is_exact_duplicate(&snippets[i], &snippets[j]) {
                duplicates.push(ImportDuplicate {
                    source_index: i,
                    destination_index: j,
                    description: snippets[i].description.clone(),
                    reason: "Exact duplicate (same command and description)".to_string(),
                });
            } else if same_command_different_description(&snippets[i], &snippets[j]) {
                diagnostics.push(CompatibilityDiagnostic {
                    code: "W-DUP-CMD".to_string(),
                    entry_index: Some(i),
                    field: Some("command".to_string()),
                    severity: DiagnosticSeverity::Warning,
                    message: format!(
                        "Same command as entry {} ('{}') but different description",
                        j, snippets[j].description
                    ),
                    suggestion: None,
                    span: None,
                });
            } else if same_description_different_command(&snippets[i], &snippets[j]) {
                diagnostics.push(CompatibilityDiagnostic {
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

    (duplicates, diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_analyze_entry_empty_command() {
        let pet = Snippet {
            description: "test".to_string(),
            command: "  ".to_string(),
            ..Default::default()
        };
        let diagnostics = analyze_entry(0, &pet);
        assert!(diagnostics.iter().any(|d| d.code == "E-CMD-EMPTY"));
    }

    #[test]
    fn test_analyze_entry_choice_variables() {
        let pet = Snippet {
            description: "test".to_string(),
            command: "echo <color=|_red_||_green_||>".to_string(),
            ..Default::default()
        };
        let diagnostics = analyze_entry(0, &pet);
        assert!(diagnostics.iter().any(|d| d.code == "I-CHOICE-VARS"));
    }

    #[test]
    fn test_detect_duplicates_exact() {
        let snippets = vec![
            Snippet {
                command: "echo hi".to_string(),
                description: "greeting".to_string(),
                ..Default::default()
            },
            Snippet {
                command: "echo hi".to_string(),
                description: "greeting".to_string(),
                ..Default::default()
            },
        ];
        let (duplicates, _) = detect_duplicates(&snippets);
        assert_eq!(duplicates.len(), 1);
        assert_eq!(duplicates[0].source_index, 0);
        assert_eq!(duplicates[0].destination_index, 1);
    }

    #[test]
    fn test_detect_duplicates_same_command_different_desc() {
        let snippets = vec![
            Snippet {
                command: "echo hi".to_string(),
                description: "greeting".to_string(),
                ..Default::default()
            },
            Snippet {
                command: "echo hi".to_string(),
                description: "other".to_string(),
                ..Default::default()
            },
        ];
        let (duplicates, diagnostics) = detect_duplicates(&snippets);
        assert!(duplicates.is_empty());
        assert!(diagnostics.iter().any(|d| d.code == "W-DUP-CMD"));
    }

    #[test]
    fn test_detect_duplicates_same_desc_different_cmd() {
        let snippets = vec![
            Snippet {
                command: "echo hi".to_string(),
                description: "greeting".to_string(),
                ..Default::default()
            },
            Snippet {
                command: "echo bye".to_string(),
                description: "greeting".to_string(),
                ..Default::default()
            },
        ];
        let (duplicates, diagnostics) = detect_duplicates(&snippets);
        assert!(duplicates.is_empty());
        assert!(diagnostics.iter().any(|d| d.code == "W-DUP-DESC"));
    }

    #[test]
    fn test_detect_duplicates_no_duplicates() {
        let snippets = vec![
            Snippet {
                command: "echo hi".to_string(),
                description: "greeting".to_string(),
                ..Default::default()
            },
            Snippet {
                command: "echo bye".to_string(),
                description: "farewell".to_string(),
                ..Default::default()
            },
        ];
        let (duplicates, diagnostics) = detect_duplicates(&snippets);
        assert!(duplicates.is_empty());
        assert!(diagnostics.is_empty());
    }
}
