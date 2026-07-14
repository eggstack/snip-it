use serde::{Deserialize, Serialize};

/// Severity level for compatibility diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    /// Informational message.
    Info,
    /// Recoverable issue; entry imported with changes.
    Warning,
    /// Entry cannot be imported safely.
    Error,
}

/// A single compatibility diagnostic produced during import or doctor checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub entry_index: Option<usize>,
    pub field: Option<String>,
    pub suggestion: Option<String>,
}

/// A duplicate entry that was skipped during import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportDuplicate {
    pub source_index: usize,
    pub destination_index: usize,
    pub description: String,
    pub reason: String,
}

/// A field that was normalized during import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizationRecord {
    pub entry_index: usize,
    pub field: String,
    pub original: String,
    pub normalized: String,
}

/// Complete report of an import operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetImportReport {
    pub schema_version: String,
    pub tool_version: String,
    pub source: String,
    pub destination: Option<String>,
    pub analysis_mode: String,
    pub mutation_flag: bool,
    pub total_entries: usize,
    pub imported: usize,
    pub skipped: usize,
    pub duplicates: Vec<ImportDuplicate>,
    pub diagnostics: Vec<CompatibilityDiagnostic>,
    pub normalizations: Vec<NormalizationRecord>,
    pub dry_run: bool,
    pub strict_mode: bool,
    pub had_fatal_error: bool,
}

impl PetImportReport {
    pub fn new(source: &str, destination: Option<&str>, dry_run: bool, strict_mode: bool) -> Self {
        Self {
            schema_version: "1.0.0".to_string(),
            tool_version: version().to_string(),
            source: source.to_string(),
            destination: destination.map(String::from),
            analysis_mode: if dry_run {
                "diagnostic".to_string()
            } else {
                "mutating".to_string()
            },
            mutation_flag: !dry_run,
            total_entries: 0,
            imported: 0,
            skipped: 0,
            duplicates: Vec::new(),
            diagnostics: Vec::new(),
            normalizations: Vec::new(),
            dry_run,
            strict_mode,
            had_fatal_error: false,
        }
    }
}

/// Diagnostic report produced by `snp doctor`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    pub schema_version: String,
    pub tool_version: String,
    pub source: Option<String>,
    pub total_entries: usize,
    pub diagnostics: Vec<CompatibilityDiagnostic>,
    pub duplicates: Vec<ImportDuplicate>,
    pub normalizations: Vec<NormalizationRecord>,
    pub has_toml_error: bool,
    pub toml_error_detail: Option<String>,
    pub recommended_import_command: Option<String>,
    pub detected_capabilities: Vec<String>,
    pub strict_mode: bool,
    /// Always true for doctor; included for report schema uniformity.
    pub dry_run: bool,
}

impl DoctorReport {
    pub fn new(strict_mode: bool) -> Self {
        Self {
            schema_version: "1.0.0".to_string(),
            tool_version: version().to_string(),
            source: None,
            total_entries: 0,
            diagnostics: Vec::new(),
            duplicates: Vec::new(),
            normalizations: Vec::new(),
            has_toml_error: false,
            toml_error_detail: None,
            recommended_import_command: None,
            detected_capabilities: Vec::new(),
            strict_mode,
            dry_run: true,
        }
    }
}

/// Returns the crate version from `Cargo.toml` at compile time.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Count the number of diagnostics at each severity level.
///
/// Returns `(info_count, warning_count, error_count)`.
pub fn diagnostic_counts(diagnostics: &[CompatibilityDiagnostic]) -> (usize, usize, usize) {
    let mut info = 0usize;
    let mut warning = 0usize;
    let mut error = 0usize;

    for d in diagnostics {
        match d.severity {
            DiagnosticSeverity::Info => info += 1,
            DiagnosticSeverity::Warning => warning += 1,
            DiagnosticSeverity::Error => error += 1,
        }
    }

    (info, warning, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_diagnostic(severity: DiagnosticSeverity) -> CompatibilityDiagnostic {
        CompatibilityDiagnostic {
            code: "TEST".to_string(),
            severity,
            message: "test".to_string(),
            entry_index: None,
            field: None,
            suggestion: None,
        }
    }

    #[test]
    fn test_diagnostic_counts() {
        let diagnostics = vec![
            make_diagnostic(DiagnosticSeverity::Info),
            make_diagnostic(DiagnosticSeverity::Warning),
            make_diagnostic(DiagnosticSeverity::Error),
            make_diagnostic(DiagnosticSeverity::Info),
            make_diagnostic(DiagnosticSeverity::Info),
            make_diagnostic(DiagnosticSeverity::Warning),
        ];

        let (info, warning, error) = diagnostic_counts(&diagnostics);
        assert_eq!(info, 3);
        assert_eq!(warning, 2);
        assert_eq!(error, 1);
    }

    #[test]
    fn test_pet_import_report_new() {
        let report = PetImportReport::new("/tmp/pets.json", Some("/tmp/snips.toml"), true, false);

        assert_eq!(report.schema_version, "1.0.0");
        assert!(!report.mutation_flag, "mutation_flag should be !dry_run");
        assert_eq!(report.analysis_mode, "diagnostic");
        assert!(report.dry_run);

        let report2 = PetImportReport::new("/tmp/pets.json", None, false, true);
        assert!(report2.mutation_flag, "mutation_flag should be !dry_run");
        assert_eq!(report2.analysis_mode, "mutating");
        assert!(!report2.dry_run);
        assert!(report2.strict_mode);
    }

    #[test]
    fn test_doctor_report_new() {
        let report = DoctorReport::new(true);

        assert!(report.dry_run);
        assert_eq!(report.schema_version, "1.0.0");
        assert!(report.strict_mode);

        let report2 = DoctorReport::new(false);
        assert!(report2.dry_run, "doctor report always has dry_run=true");
        assert!(!report2.strict_mode);
    }

    #[test]
    fn test_version() {
        let v = version();
        assert_eq!(v, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_diagnostic_severity_serialization() {
        let severities = vec![
            DiagnosticSeverity::Info,
            DiagnosticSeverity::Warning,
            DiagnosticSeverity::Error,
        ];

        for severity in severities {
            let json = serde_json::to_string(&severity).unwrap();
            let roundtripped: DiagnosticSeverity = serde_json::from_str(&json).unwrap();
            assert_eq!(severity, roundtripped);
        }
    }

    #[test]
    fn test_compatibility_diagnostic_serialization() {
        let diagnostic = CompatibilityDiagnostic {
            code: "TOML_ESCAPE".to_string(),
            severity: DiagnosticSeverity::Warning,
            message: "Backslash in double-quoted string".to_string(),
            entry_index: Some(5),
            field: Some("command".to_string()),
            suggestion: Some("Use single-quoted string".to_string()),
        };

        let json = serde_json::to_string(&diagnostic).unwrap();
        let roundtripped: CompatibilityDiagnostic = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtripped.code, "TOML_ESCAPE");
        assert_eq!(roundtripped.severity, DiagnosticSeverity::Warning);
        assert_eq!(roundtripped.message, "Backslash in double-quoted string");
        assert_eq!(roundtripped.entry_index, Some(5));
        assert_eq!(roundtripped.field, Some("command".to_string()));
        assert_eq!(
            roundtripped.suggestion,
            Some("Use single-quoted string".to_string())
        );
    }
}
