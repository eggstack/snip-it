//! **Layer: Application**
//!
//! `snp doctor` report rendering: [`DiagnosticReportFormat`] and human-readable
//! emission. Pure display logic with no check orchestration.

use crate::diagnostics::{DiagnosticSeverity, DoctorReport, diagnostic_counts};

/// Output format for the doctor report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub enum DiagnosticReportFormat {
    #[default]
    Human,
    Json,
}

/// Emit the report in human-readable format to stderr.
pub(crate) fn emit_human_report(report: &DoctorReport) {
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
