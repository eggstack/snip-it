//! **Layer: Domain/Core**
//!
//! Shared diagnostic vocabulary and report models for `validate`, `doctor`,
//! `import`, and related surfaces.
//!
//! Pure data types with no I/O or platform dependencies.

use crate::error::{SnipError, SnipResult};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Severity level shared across diagnostic surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    /// Informational message.
    Info,
    /// Recoverable issue.
    Warning,
    /// Blocking issue.
    Error,
}

/// Repairability classification shared with `snp validate` (serde names unchanged).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticRepairability {
    Auto,
    Manual,
    Unrepairable,
}

/// Validated diagnostic code (`E-`/`W-`/`I-`, `compat.*`, or `sync.*`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DiagnosticCode(String);

impl DiagnosticCode {
    /// Validate and construct a diagnostic code.
    pub fn new(raw: impl Into<String>) -> SnipResult<Self> {
        let raw = raw.into();
        if validate_diagnostic_code(&raw) {
            Ok(Self(raw))
        } else {
            Err(SnipError::runtime_error(
                "Invalid diagnostic code",
                Some(&format!(
                    "Diagnostic code '{raw}' is not in the published namespace"
                )),
            ))
        }
    }

    /// Construct a code that must already satisfy the namespace (programming-error panic otherwise).
    pub fn expect_valid(raw: &str) -> Self {
        Self::new(raw).unwrap_or_else(|e| panic!("invalid diagnostic code {raw:?}: {e}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Severity implied by an `E-`/`W-`/`I-` prefix; `None` for dotted forms.
    pub fn severity(&self) -> Option<DiagnosticSeverity> {
        if self.0.starts_with("E-") {
            Some(DiagnosticSeverity::Error)
        } else if self.0.starts_with("W-") {
            Some(DiagnosticSeverity::Warning)
        } else if self.0.starts_with("I-") {
            Some(DiagnosticSeverity::Info)
        } else {
            None
        }
    }

    pub fn is_strict_elevatable(&self) -> bool {
        self.0.starts_with("W-")
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for DiagnosticCode {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for DiagnosticCode {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Construct a validated code for a known-good literal at a call site.
pub fn diagnostic_code(raw: &str) -> DiagnosticCode {
    DiagnosticCode::expect_valid(raw)
}

fn validate_diagnostic_code(raw: &str) -> bool {
    if raw.is_empty() {
        return false;
    }
    if let Some(rest) = raw.strip_prefix("E-") {
        return screaming_case_body(rest);
    }
    if let Some(rest) = raw.strip_prefix("W-") {
        return screaming_case_body(rest);
    }
    if let Some(rest) = raw.strip_prefix("I-") {
        return screaming_case_body(rest);
    }
    if let Some(rest) = raw.strip_prefix("compat.") {
        return dotted_tail_valid(rest);
    }
    if let Some(rest) = raw.strip_prefix("sync.") {
        return dotted_tail_valid(rest);
    }
    false
}

fn screaming_case_body(body: &str) -> bool {
    !body.is_empty()
        && body
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
}

fn dotted_tail_valid(tail: &str) -> bool {
    !tail.is_empty() && tail.split('.').all(|segment| !segment.is_empty())
}

/// Byte-offset span within the source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

/// A single compatibility diagnostic produced during import or doctor checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityDiagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub entry_index: Option<usize>,
    pub field: Option<String>,
    pub suggestion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<SourceSpan>,
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
    pub detected_capabilities: Vec<String>,
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
            detected_capabilities: Vec::new(),
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
    pub analysis_mode: String,
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
            analysis_mode: "diagnostic".to_string(),
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
            code: diagnostic_code("I-TEST"),
            severity,
            message: "test".to_string(),
            entry_index: None,
            field: None,
            suggestion: None,
            span: None,
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
            code: diagnostic_code("W-TYPE-MISMATCH"),
            severity: DiagnosticSeverity::Warning,
            message: "Backslash in double-quoted string".to_string(),
            entry_index: Some(5),
            field: Some("command".to_string()),
            suggestion: Some("Use single-quoted string".to_string()),
            span: None,
        };

        let json = serde_json::to_string(&diagnostic).unwrap();
        let roundtripped: CompatibilityDiagnostic = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtripped.code, "W-TYPE-MISMATCH");
        assert_eq!(roundtripped.severity, DiagnosticSeverity::Warning);
        assert_eq!(roundtripped.message, "Backslash in double-quoted string");
        assert_eq!(roundtripped.entry_index, Some(5));
        assert_eq!(roundtripped.field, Some("command".to_string()));
        assert_eq!(
            roundtripped.suggestion,
            Some("Use single-quoted string".to_string())
        );
    }

    #[test]
    fn test_source_span_serialization() {
        let span = SourceSpan { start: 10, end: 20 };
        let json = serde_json::to_string(&span).unwrap();
        let roundtripped: SourceSpan = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped.start, 10);
        assert_eq!(roundtripped.end, 20);
    }

    #[test]
    fn test_diagnostic_span_skip_none() {
        let diag = CompatibilityDiagnostic {
            code: diagnostic_code("I-TEST"),
            severity: DiagnosticSeverity::Info,
            message: "test".to_string(),
            entry_index: None,
            field: None,
            suggestion: None,
            span: None,
        };
        let json = serde_json::to_string(&diag).unwrap();
        assert!(
            !json.contains("span"),
            "span field should be skipped when None"
        );
    }

    #[test]
    fn test_diagnostic_ordering_preserved() {
        let mut diagnostics = Vec::new();
        for i in 0..10 {
            diagnostics.push(CompatibilityDiagnostic {
                code: DiagnosticCode::new(format!("I-TEST-{i}")).expect("test code"),
                severity: DiagnosticSeverity::Info,
                message: format!("msg {i}"),
                entry_index: Some(i),
                field: None,
                suggestion: None,
                span: None,
            });
        }
        for (i, d) in diagnostics.iter().enumerate() {
            assert_eq!(d.code.as_str(), format!("I-TEST-{i}"));
            assert_eq!(d.entry_index, Some(i));
        }
    }

    #[test]
    fn test_severity_ranking() {
        assert_ne!(DiagnosticSeverity::Info, DiagnosticSeverity::Warning);
        assert_ne!(DiagnosticSeverity::Warning, DiagnosticSeverity::Error);
        assert_ne!(DiagnosticSeverity::Info, DiagnosticSeverity::Error);

        let info_json = serde_json::to_string(&DiagnosticSeverity::Info).unwrap();
        let warn_json = serde_json::to_string(&DiagnosticSeverity::Warning).unwrap();
        let err_json = serde_json::to_string(&DiagnosticSeverity::Error).unwrap();
        assert_ne!(info_json, warn_json);
        assert_ne!(warn_json, err_json);
        assert_ne!(info_json, err_json);
    }

    #[test]
    fn test_diagnostic_codes_follow_convention() {
        let codes = vec![
            "E-CMD-EMPTY",
            "W-DESC-EMPTY",
            "W-DESC-MISSING",
            "W-CMD-MISSING",
            "W-TYPE-MISMATCH",
            "W-TAG-EMPTY",
            "I-FIELD-UNKNOWN",
            "I-OUTPUT-PRESENT",
            "I-TAGS-EMPTY",
            "I-CHOICE-VARS",
            "W-DUP-CMD",
            "W-DUP-DESC",
        ];
        for code in &codes {
            let prefix = &code[..2];
            assert!(
                prefix == "E-" || prefix == "W-" || prefix == "I-",
                "Code '{}' should start with E-, W-, or I-",
                code
            );
        }
    }

    #[test]
    fn test_strict_mode_error_classification() {
        let diagnostics = vec![
            CompatibilityDiagnostic {
                code: diagnostic_code("W-DESC-EMPTY"),
                severity: DiagnosticSeverity::Warning,
                message: "warn".to_string(),
                entry_index: Some(0),
                field: None,
                suggestion: None,
                span: None,
            },
            CompatibilityDiagnostic {
                code: diagnostic_code("I-TAGS-EMPTY"),
                severity: DiagnosticSeverity::Info,
                message: "info".to_string(),
                entry_index: Some(0),
                field: None,
                suggestion: None,
                span: None,
            },
        ];

        let has_errors = diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error);
        assert!(!has_errors);

        let mut diagnostics_with_error = diagnostics.clone();
        diagnostics_with_error.push(CompatibilityDiagnostic {
            code: diagnostic_code("E-CMD-EMPTY"),
            severity: DiagnosticSeverity::Error,
            message: "error".to_string(),
            entry_index: Some(1),
            field: None,
            suggestion: None,
            span: None,
        });
        let has_errors = diagnostics_with_error
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error);
        assert!(has_errors);
    }

    #[test]
    fn test_diagnostic_bounded_message() {
        let long_message = "x".repeat(1000);
        let diag = CompatibilityDiagnostic {
            code: diagnostic_code("I-TEST"),
            severity: DiagnosticSeverity::Info,
            message: long_message.clone(),
            entry_index: None,
            field: None,
            suggestion: None,
            span: None,
        };
        assert_eq!(diag.message.len(), 1000);

        let json = serde_json::to_string(&diag).unwrap();
        let roundtripped: CompatibilityDiagnostic = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped.message.len(), 1000);
    }

    #[test]
    fn test_doctor_report_recommendation() {
        let mut report = DoctorReport::new(false);
        report.recommended_import_command = Some("snp import pet /path/to/file.toml".to_string());

        let json = serde_json::to_string(&report).unwrap();
        let roundtripped: DoctorReport = serde_json::from_str(&json).unwrap();
        assert!(roundtripped.recommended_import_command.is_some());
        assert!(
            roundtripped
                .recommended_import_command
                .unwrap()
                .contains("snp import pet")
        );
    }

    #[test]
    fn test_diagnostic_counts_empty() {
        let (info, warning, error) = diagnostic_counts(&[]);
        assert_eq!(info, 0);
        assert_eq!(warning, 0);
        assert_eq!(error, 0);
    }

    #[test]
    fn test_pet_import_report_full_roundtrip() {
        let mut report = PetImportReport::new("/tmp/pet.toml", Some("mylib"), true, false);
        report.total_entries = 5;
        report.imported = 3;
        report.skipped = 2;
        report.diagnostics.push(CompatibilityDiagnostic {
            code: diagnostic_code("W-DESC-EMPTY"),
            severity: DiagnosticSeverity::Warning,
            message: "Empty description".to_string(),
            entry_index: Some(0),
            field: Some("description".to_string()),
            suggestion: None,
            span: None,
        });
        report.duplicates.push(ImportDuplicate {
            source_index: 0,
            destination_index: 2,
            description: "test".to_string(),
            reason: "Exact duplicate".to_string(),
        });
        report.normalizations.push(NormalizationRecord {
            entry_index: 1,
            field: "tags".to_string(),
            original: "git".to_string(),
            normalized: "[\"git\"]".to_string(),
        });

        let json = serde_json::to_string_pretty(&report).unwrap();
        let roundtripped: PetImportReport = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtripped.total_entries, 5);
        assert_eq!(roundtripped.imported, 3);
        assert_eq!(roundtripped.skipped, 2);
        assert_eq!(roundtripped.diagnostics.len(), 1);
        assert_eq!(roundtripped.duplicates.len(), 1);
        assert_eq!(roundtripped.normalizations.len(), 1);
        assert!(!roundtripped.mutation_flag);
        assert!(roundtripped.dry_run);
    }

    #[test]
    fn test_doctor_report_no_secrets_in_json() {
        let mut report = DoctorReport::new(false);
        report.source = Some("/tmp/pet.toml".to_string());
        report.diagnostics.push(CompatibilityDiagnostic {
            code: diagnostic_code("compat.sync.enabled"),
            entry_index: None,
            field: Some("sync".to_string()),
            severity: DiagnosticSeverity::Info,
            message: "Sync enabled (server: https://sync.example.com)".to_string(),
            suggestion: None,
            span: None,
        });

        let json = serde_json::to_string_pretty(&report).unwrap();
        // API keys and tokens must never appear in report JSON
        assert!(
            !json.contains("api_key"),
            "Report JSON should not contain api_key field"
        );
        assert!(
            !json.contains("token"),
            "Report JSON should not contain token field"
        );
        assert!(
            !json.contains("secret"),
            "Report JSON should not contain secret field"
        );
    }

    #[test]
    fn test_diagnostic_code_grammar_accepts_emitted_codes() {
        let validate_codes = [
            "E-CMD-EMPTY",
            "E-DUP-ID",
            "E-FILE-READ",
            "E-INDEX-MISSING-FILE",
            "E-PRIMARY-MISSING",
            "E-TOML-PARSE",
            "I-FILE-EMPTY",
            "W-CORRUPT-BAK",
            "W-DESC-EMPTY",
            "W-DUP-CHECK-SKIPPED",
            "W-EXACT-DUP",
            "W-ID-EMPTY",
            "W-INSECURE-PERMS",
            "W-LIB-INDEX-UNREADABLE",
            "W-LIB-UNREADABLE",
            "W-NO-PRIMARY",
            "W-ORPHAN-FILE",
            "W-SAME-ID-DIVERGENT",
            "W-USAGE-ORPHAN",
        ];
        for code in validate_codes {
            assert!(DiagnosticCode::new(code).is_ok(), "reject {code}");
        }
        assert!(DiagnosticCode::new("compat.sync.checked").is_ok());
        assert!(DiagnosticCode::new("sync.attention.configuration").is_ok());
        assert!(DiagnosticCode::new("sync.config_load_failed").is_ok());
        assert!(DiagnosticCode::new("compat.shell.bash.ok").is_ok());
    }

    #[test]
    fn test_diagnostic_code_rejects_malformed() {
        assert!(DiagnosticCode::new("e-cmd-empty").is_err());
        assert!(DiagnosticCode::new("x-unknown").is_err());
        assert!(DiagnosticCode::new("E-").is_err());
        assert!(DiagnosticCode::new("").is_err());
        let err = DiagnosticCode::new("bad-code").unwrap_err().to_string();
        assert!(err.contains("bad-code"));
    }

    #[test]
    fn test_diagnostic_code_severity_mapping() {
        assert_eq!(
            diagnostic_code("E-CMD-EMPTY").severity(),
            Some(DiagnosticSeverity::Error)
        );
        assert_eq!(
            diagnostic_code("W-DESC-EMPTY").severity(),
            Some(DiagnosticSeverity::Warning)
        );
        assert_eq!(
            diagnostic_code("I-FILE-EMPTY").severity(),
            Some(DiagnosticSeverity::Info)
        );
        assert_eq!(diagnostic_code("compat.sync.checked").severity(), None);
    }

    #[test]
    fn test_diagnostic_code_serde_transparent() {
        let code = diagnostic_code("W-DESC-EMPTY");
        let json = serde_json::to_string(&code).unwrap();
        assert_eq!(json, "\"W-DESC-EMPTY\"");
        let back: DiagnosticCode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, code);
    }

    #[test]
    fn test_diagnostic_repairability_serde_names() {
        for (value, name) in [
            (DiagnosticRepairability::Auto, "\"Auto\""),
            (DiagnosticRepairability::Manual, "\"Manual\""),
            (DiagnosticRepairability::Unrepairable, "\"Unrepairable\""),
        ] {
            assert_eq!(serde_json::to_string(&value).unwrap(), name);
        }
    }
}
