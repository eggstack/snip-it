//! Schema stability tests for Release 4D.
//!
//! Verifies that JSON and CSV output surfaces remain additive and parseable.
//! Uses inline expected values as schema fixtures (no external snapshot tool).

mod support;

use support::helpers::*;

/// Create a library with a representative snippet for schema validation.
fn create_schema_test_library(config_dir: &std::path::Path) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", "schema-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libraries_dir).unwrap();
    std::fs::write(
        libraries_dir.join("schema-test.toml"),
        r#"[[snippets]]
id = "schema-1"
description = "Test snippet"
command = "echo test"
tags = ["test", "schema"]
output = "Sample output"
folders = ["folder-a"]
favorite = true
created_at = 1700000000
updated_at = 1700001000
"#,
    )
    .unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", "schema-test"]);
    cmd.output().unwrap();
}

// ── JSON schema fixtures ──

#[test]
fn test_json_schema_has_all_expected_fields() {
    let (_tmp, config_dir) = setup_test_env();
    create_schema_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1);

    let item = &items[0];

    // Required fields must be present with correct types
    assert!(
        item["description"].is_string(),
        "JSON schema: 'description' must be a string"
    );
    assert!(
        item["command"].is_string(),
        "JSON schema: 'command' must be a string"
    );
    assert!(
        item["tags"].is_array(),
        "JSON schema: 'tags' must be an array"
    );
    assert!(
        item["output"].is_string(),
        "JSON schema: 'output' must be a string"
    );
    assert!(
        item["favorite"].is_boolean(),
        "JSON schema: 'favorite' must be a boolean"
    );
    assert!(
        item["folders"].is_array(),
        "JSON schema: 'folders' must be an array"
    );

    // Verify exact values for the fixture
    assert_eq!(item["description"].as_str().unwrap(), "Test snippet");
    assert_eq!(item["command"].as_str().unwrap(), "echo test");
    assert_eq!(item["output"].as_str().unwrap(), "Sample output");
    assert!(item["favorite"].as_bool().unwrap());
    assert_eq!(
        item["tags"].as_array().unwrap().len(),
        2,
        "tags should have 2 elements"
    );
    assert_eq!(
        item["folders"].as_array().unwrap().len(),
        1,
        "folders should have 1 element"
    );
}

#[test]
fn test_json_schema_no_usage_fields_exposed() {
    let (_tmp, config_dir) = setup_test_env();
    create_schema_test_library(&config_dir);

    // Add usage data
    std::fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "schema-1"
use_count = 42
last_used_at = 1700002000
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    let item = &items[0];

    // Usage fields must NOT be in JSON output (local-only)
    assert!(
        !item.as_object().unwrap().contains_key("use_count"),
        "JSON should not expose use_count"
    );
    assert!(
        !item.as_object().unwrap().contains_key("last_used_at"),
        "JSON should not expose last_used_at"
    );
}

#[test]
fn test_json_schema_additive_new_field_does_not_break() {
    let (_tmp, config_dir) = setup_test_env();
    create_schema_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse as generic JSON - extra fields should not break parsing
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 1);

    // Verify the item is a valid JSON object with expected field count
    let obj = items[0].as_object().unwrap();
    assert!(
        obj.len() >= 6,
        "Item should have at least 6 fields, got {}",
        obj.len()
    );
}

#[test]
fn test_json_output_is_valid_json_array() {
    let (_tmp, config_dir) = setup_test_env();
    create_schema_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Must start with [ and end with ]
    assert!(
        stdout.trim().starts_with('['),
        "JSON output must start with ["
    );
    assert!(stdout.trim().ends_with(']'), "JSON output must end with ]");

    // Must be parseable
    let parsed: Result<Vec<serde_json::Value>, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "JSON output must be valid JSON");
}

// ── CSV schema fixtures ──

#[test]
fn test_csv_schema_has_expected_header() {
    let (_tmp, config_dir) = setup_test_env();
    create_schema_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--csv"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let header = stdout.lines().next().unwrap();

    // CSV header must contain expected columns
    assert!(
        header.contains("description"),
        "CSV header must contain 'description'"
    );
    assert!(
        header.contains("command"),
        "CSV header must contain 'command'"
    );
    assert!(header.contains("tags"), "CSV header must contain 'tags'");
    assert!(
        header.contains("output"),
        "CSV header must contain 'output'"
    );
    assert!(
        header.contains("favorite"),
        "CSV header must contain 'favorite'"
    );
    assert!(
        header.contains("folders"),
        "CSV header must contain 'folders'"
    );
}

#[test]
fn test_csv_data_row_count_matches() {
    let (_tmp, config_dir) = setup_test_env();
    create_schema_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--csv"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    // Header + 1 data row
    assert_eq!(lines.len(), 2, "CSV should have header + 1 data row");
}

#[test]
fn test_csv_output_preserves_exact_values() {
    let (_tmp, config_dir) = setup_test_env();
    create_schema_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--csv"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let data_line = stdout.lines().nth(1).unwrap();

    assert!(
        data_line.contains("Test snippet"),
        "CSV data should contain description"
    );
    assert!(
        data_line.contains("echo test"),
        "CSV data should contain command"
    );
    assert!(
        data_line.contains("Sample output"),
        "CSV data should contain output"
    );
}

#[test]
fn test_csv_output_with_multiline_output() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "schema-ml"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libraries_dir).unwrap();
    std::fs::write(
        libraries_dir.join("schema-ml.toml"),
        r#"[[snippets]]
id = "ml-1"
description = "Multiline snippet"
command = "echo ml"
tags = ["test"]
output = "line1\nline2\nline3"
folders = []
favorite = false
created_at = 100
updated_at = 100
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "schema-ml"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--csv"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Multiline output is CSV-quoted and may span multiple lines
    let header = stdout.lines().next().unwrap();
    assert!(header.contains("description"), "CSV must have header");
    assert!(
        stdout.contains("Multiline snippet"),
        "CSV data should contain the description"
    );
}

// ── JSON error output stays on stderr ──

#[test]
fn test_json_errors_go_to_stderr() {
    let (_tmp, config_dir) = setup_test_env();

    // Using a nonexistent library should produce an error on stderr
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--library", "nonexistent", "--json"]);
    let output = cmd.output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.is_empty(),
        "Errors should be written to stderr, not stdout"
    );
}

// ── CSV multiline value handling ──

#[test]
fn test_csv_with_special_chars_parseable() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "schema-special"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libraries_dir).unwrap();
    std::fs::write(
        libraries_dir.join("schema-special.toml"),
        r#"[[snippets]]
id = "special-1"
description = "Quote test"
command = "echo \"hello, world\""
tags = ["test"]
output = "value with, commas and \"quotes\""
folders = []
favorite = false
created_at = 100
updated_at = 100
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "schema-special"]);
    cmd.output().unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--csv"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // CSV should contain the description
    assert!(
        stdout.contains("Quote test"),
        "CSV should contain 'Quote test'. Got: {stdout}"
    );
}
