//! Golden-shape smoke tests for validate / doctor / repair JSON envelopes.

mod support;

use std::fs;
use std::path::Path;
use support::helpers::*;

fn setup_fixture(config_dir: &Path) {
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join("main.toml"),
        r#"[[snippets]]
id = "g1"
description = "warn empty desc later"
command = "echo hi"
[[snippets]]
id = "g2"
description = ""
command = "echo"
"#,
    )
    .unwrap();
    fs::write(
        config_dir.join("libraries.toml"),
        r#"[[libraries]]
filename = "main"
is_primary = true
"#,
    )
    .unwrap();
}

#[test]
fn validate_json_envelope_unchanged() {
    let (_tmp, config_dir) = setup_test_env();
    setup_fixture(&config_dir);
    let out = snp_in(&config_dir)
        .args(["validate", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    for key in [
        "schema_version",
        "tool_version",
        "strict_mode",
        "dry_run",
        "total_libraries",
        "total_snippets",
        "diagnostics",
    ] {
        assert!(json.get(key).is_some(), "missing key {key}");
    }
    let diag = &json["diagnostics"][0];
    for key in [
        "code",
        "severity",
        "path",
        "library",
        "snippet_id",
        "message",
        "repairability",
    ] {
        assert!(diag.get(key).is_some(), "missing diagnostic field {key}");
    }
}

#[test]
fn doctor_json_envelope_unchanged() {
    let (_tmp, config_dir) = setup_test_env();
    setup_fixture(&config_dir);
    let lib = config_dir.join("libraries").join("main.toml");
    let out = snp_in(&config_dir)
        .args([
            "doctor",
            "--library",
            &lib.to_string_lossy(),
            "--report",
            "json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success() || out.status.code() == Some(6));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    for key in [
        "schema_version",
        "tool_version",
        "analysis_mode",
        "total_entries",
        "diagnostics",
        "strict_mode",
        "dry_run",
    ] {
        assert!(json.get(key).is_some(), "missing key {key}");
    }
}

#[test]
fn repair_json_envelope_unchanged() {
    let (_tmp, config_dir) = setup_test_env();
    setup_fixture(&config_dir);
    let out = snp_in(&config_dir)
        .args(["repair", "--json", "--dry-run"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    for key in [
        "items",
        "backups",
        "applied",
        "skipped",
        "failed",
        "exit_status",
    ] {
        assert!(json.get(key).is_some(), "missing key {key}");
    }
    if let Some(items) = json["items"].as_array()
        && let Some(item) = items.first()
    {
        for key in [
            "action",
            "category",
            "transaction_id",
            "problem",
            "fix",
            "safe",
        ] {
            assert!(item.get(key).is_some(), "missing repair item field {key}");
        }
    }
}
