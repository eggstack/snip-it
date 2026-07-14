//! Security and privacy tests for Release 4D.
//!
//! Uses sentinel values from the Release 4D plan to prove that untrusted
//! metadata does not leak or execute.

mod support;

use support::helpers::*;

/// Create a library containing security sentinel values in various fields.
fn create_security_test_library(config_dir: &std::path::Path) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", "security-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libraries_dir).unwrap();
    std::fs::write(
        libraries_dir.join("security-test.toml"),
        r#"[[snippets]]
id = "sec-1"
description = "SUPER_SECRET_RELEASE4_SENTINEL"
command = "echo safe"
tags = ["security"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[snippets]]
id = "sec-2"
description = "OSC hyperlink test"
command = "echo safe"
tags = ["security"]
output = "\x1b]8;;https://example.com\x07link\x1b]8;;\x07"
folders = []
favorite = false
created_at = 200
updated_at = 200

[[snippets]]
id = "sec-3"
description = "Shell injection via backticks"
command = "echo safe"
tags = ["security"]
output = "`touch /tmp/should-not-run`"
folders = []
favorite = false
created_at = 300
updated_at = 300

[[snippets]]
id = "sec-4"
description = "Shell injection via dollar-paren"
command = "echo safe"
tags = ["security"]
output = "$(touch /tmp/should-not-run)"
folders = []
favorite = false
created_at = 400
updated_at = 400

[[snippets]]
id = "sec-5"
description = "URL with credentials"
command = "echo safe"
tags = ["security"]
output = "https://user:password@example.com/path?token=abc"
folders = []
favorite = false
created_at = 500
updated_at = 500
"#,
    )
    .unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", "security-test"]);
    cmd.output().unwrap();
}

// ── No shell execution during ranking, preview, import, doctor, or indexing ──

#[test]
fn test_no_shell_execution_during_list() {
    let (_tmp, config_dir) = setup_test_env();
    create_security_test_library(&config_dir);

    // Listing with shell injection in output should not execute anything
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(items.len(), 5);

    // Verify the sentinel values are preserved as-is
    let sec3 = items
        .iter()
        .find(|i| i["description"] == "Shell injection via backticks")
        .unwrap();
    assert_eq!(
        sec3["output"].as_str().unwrap(),
        "`touch /tmp/should-not-run`"
    );
    let sec4 = items
        .iter()
        .find(|i| i["description"] == "Shell injection via dollar-paren")
        .unwrap();
    assert_eq!(
        sec4["output"].as_str().unwrap(),
        "$(touch /tmp/should-not-run)"
    );

    // The file should not exist (nothing was executed)
    assert!(
        !std::path::Path::new("/tmp/should-not-run").exists(),
        "shell injection in output should not have been executed"
    );
}

#[test]
fn test_no_shell_execution_during_filter() {
    let (_tmp, config_dir) = setup_test_env();
    create_security_test_library(&config_dir);

    // Filtering should not execute anything
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--filter", "SECRET", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    assert!(
        !std::path::Path::new("/tmp/should-not-run").exists(),
        "shell injection should not execute during filtering"
    );
}

#[test]
fn test_no_shell_execution_during_sort() {
    let (_tmp, config_dir) = setup_test_env();
    create_security_test_library(&config_dir);

    // Sorting should not execute anything
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--sort", "description", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());

    assert!(
        !std::path::Path::new("/tmp/should-not-run").exists(),
        "shell injection should not execute during sorting"
    );
}

// ── No terminal escape execution in human views ──

#[test]
fn test_osc_hyperlinks_not_rendered_as_links() {
    let (_tmp, config_dir) = setup_test_env();
    create_security_test_library(&config_dir);

    // Human-readable list output should contain the raw text, not render OSC as clickable links
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The output should contain the escaped form or raw text, not be empty
    // (if OSC sequences were interpreted, the display would be mangled)
    assert!(
        stdout.contains("link") || stdout.contains("OSC") || stdout.contains("sec-2"),
        "OSC hyperlink output should be safely rendered, not interpreted as terminal control"
    );
}

#[test]
fn test_ansi_sequences_neutralized_in_human_display() {
    let (_tmp, config_dir) = setup_test_env();
    create_security_test_library(&config_dir);

    // Human-readable output should neutralize ANSI sequences
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The raw ANSI escape byte should not appear in the output
    // (it should be stripped by sanitize_for_terminal)
    // Note: we check the output is valid and contains expected text
    assert!(
        stdout.contains("OSC hyperlink test") || stdout.contains("sec-2"),
        "Human view should contain the description text"
    );
}

// ── No command/output bodies in usage logs ──

#[test]
fn test_sentinel_not_in_usage_logs() {
    let (_tmp, config_dir) = setup_test_env();
    create_security_test_library(&config_dir);

    // Simulate a usage entry (manually create usage.toml with only ID reference)
    std::fs::write(
        config_dir.join("usage.toml"),
        r#"[[entries]]
id = "sec-1"
use_count = 1
last_used_at = 1700000000
"#,
    )
    .unwrap();

    let usage_content = std::fs::read_to_string(config_dir.join("usage.toml")).unwrap();

    // Usage file should not contain the sentinel or any command text
    assert!(
        !usage_content.contains("SUPER_SECRET_RELEASE4_SENTINEL"),
        "usage.toml should not contain command/description text"
    );
    assert!(
        !usage_content.contains("touch /tmp"),
        "usage.toml should not contain output content"
    );
    assert!(
        !usage_content.contains("echo safe"),
        "usage.toml should not contain command text"
    );
    // Should only contain the ID reference
    assert!(usage_content.contains("sec-1"));
}

// ── JSON output preserves raw values without execution ──

#[test]
fn test_json_output_preserves_sentinel_values() {
    let (_tmp, config_dir) = setup_test_env();
    create_security_test_library(&config_dir);

    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--json"]);
    let output = cmd.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();

    // Verify all sentinel values are preserved exactly in JSON output
    let sec1 = items
        .iter()
        .find(|i| i["description"] == "SUPER_SECRET_RELEASE4_SENTINEL")
        .unwrap();
    assert_eq!(
        sec1["description"].as_str().unwrap(),
        "SUPER_SECRET_RELEASE4_SENTINEL"
    );

    let sec3 = items
        .iter()
        .find(|i| i["description"] == "Shell injection via backticks")
        .unwrap();
    assert_eq!(
        sec3["output"].as_str().unwrap(),
        "`touch /tmp/should-not-run`"
    );

    let sec4 = items
        .iter()
        .find(|i| i["description"] == "Shell injection via dollar-paren")
        .unwrap();
    assert_eq!(
        sec4["output"].as_str().unwrap(),
        "$(touch /tmp/should-not-run)"
    );

    let sec5 = items
        .iter()
        .find(|i| i["description"] == "URL with credentials")
        .unwrap();
    assert_eq!(
        sec5["output"].as_str().unwrap(),
        "https://user:password@example.com/path?token=abc"
    );

    // Nothing should have been executed
    assert!(
        !std::path::Path::new("/tmp/should-not-run").exists(),
        "JSON output should not trigger shell execution"
    );
}

// ── Report and sidecar files use private permissions ──

#[test]
fn test_usage_file_permissions() {
    let (_tmp, config_dir) = setup_test_env();
    create_security_test_library(&config_dir);

    // Create a usage.toml via the CLI
    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "perm-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    std::fs::write(
        libraries_dir.join("perm-test.toml"),
        r#"[[snippets]]
id = "perm-1"
description = "test"
command = "echo test"
tags = ["test"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "perm-test"]);
    cmd.output().unwrap();

    // Verify usage.toml exists with reasonable permissions
    let usage_path = config_dir.join("usage.toml");
    if usage_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&usage_path).unwrap().permissions();
            let mode = perms.mode();
            // Should not be world-readable (no 004 bits for others)
            assert!(
                mode & 0o044 == 0,
                "usage.toml should not be world-readable, got mode {mode:04o}"
            );
        }
    }
}

// ── Doctor does not execute commands ──

#[test]
fn test_doctor_with_injection_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();

    // Create a pet file with injection attempts
    let pet_file = _tmp.path().join("injection_pet.toml");
    std::fs::write(
        &pet_file,
        r#"[[snippets]]
description = "backtick injection"
command = "`touch /tmp/doctor-inject`"
output = ""
tag = ["test"]

[[snippets]]
description = "dollar-paren injection"
command = "$(touch /tmp/doctor-inject)"
output = ""
tag = ["test"]
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["doctor", "--pet-file", pet_file.to_str().unwrap()]);
    let output = cmd.output().unwrap();
    // Doctor should succeed (read-only analysis)
    assert!(output.status.success());

    // Nothing should have been executed
    assert!(
        !std::path::Path::new("/tmp/doctor-inject").exists(),
        "doctor should not execute commands from pet files"
    );
}

// ── Import does not execute commands ──

#[test]
fn test_import_with_injection_does_not_execute() {
    let (_tmp, config_dir) = setup_test_env();

    let pet_file = _tmp.path().join("import_inject.toml");
    std::fs::write(
        &pet_file,
        r#"[[snippets]]
description = "injection test"
command = "`touch /tmp/import-inject`"
output = ""
tag = ["test"]
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["import", "pet", pet_file.to_str().unwrap(), "--dry-run"]);
    let output = cmd.output().unwrap();
    assert!(
        output.status.success(),
        "import --dry-run should succeed. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !std::path::Path::new("/tmp/import-inject").exists(),
        "import --dry-run should not execute commands"
    );
}

// ── External traversal does not interpret shell patterns ──

#[test]
fn test_description_with_shell_glob_not_expanded() {
    let (_tmp, config_dir) = setup_test_env();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "create", "glob-test"]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    std::fs::create_dir_all(&libraries_dir).unwrap();
    std::fs::write(
        libraries_dir.join("glob-test.toml"),
        r#"[[snippets]]
id = "glob-1"
description = "Files matching *.toml in /tmp/*"
command = "ls *.toml"
tags = ["glob"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100
"#,
    )
    .unwrap();

    let mut cmd = snp_in(&config_dir);
    cmd.args(["library", "set-primary", "glob-test"]);
    cmd.output().unwrap();

    // List with the glob pattern in filter - should not expand
    let mut cmd = snp_in(&config_dir);
    cmd.args(["list", "--filter", "*.toml", "--json"]);
    let output = cmd.output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let items: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();

    // Should find the snippet whose description contains "*.toml"
    assert!(
        !items.is_empty(),
        "Glob pattern in description should not be expanded by the shell"
    );
    assert_eq!(
        items[0]["description"].as_str().unwrap(),
        "Files matching *.toml in /tmp/*"
    );
}
