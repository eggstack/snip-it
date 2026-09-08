//! Plan 009 regression tests: canonical read-only library resolution.
//!
//! Proves that legacy single-file, primary, named, and `all` scopes resolve
//! through one side-effect-free path, that read-only CLI/MCP calls create no
//! files, and that `doctor`/`validate` (and `validate`/`repair`) classify the
//! same missing-primary state consistently.

mod support;

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use support::helpers::*;

fn file_snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(bytes) = fs::read(&p) {
                let rel = p.strip_prefix(dir).unwrap_or(&p).to_path_buf();
                out.insert(rel, bytes);
            }
        }
    }
    out
}

fn write_legacy_snippets(config_dir: &Path) {
    fs::write(
        config_dir.join("snippets.toml"),
        r#"[[snippets]]
id = "legacy-1"
description = "legacy snippet"
command = "echo legacy"
"#,
    )
    .unwrap();
}

fn write_two_libraries(config_dir: &Path) {
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        config_dir.join("libraries.toml"),
        r#"[[libraries]]
filename = "work"
library_id = "server-work"
is_primary = true

[[libraries]]
filename = "personal"
is_primary = false
"#,
    )
    .unwrap();
    fs::write(
        libraries_dir.join("work.toml"),
        r#"[[snippets]]
id = "work-1"
description = "work snippet"
command = "echo work"
"#,
    )
    .unwrap();
    fs::write(
        libraries_dir.join("personal.toml"),
        r#"[[snippets]]
id = "personal-1"
description = "personal snippet"
command = "echo personal"
"#,
    )
    .unwrap();
}

#[test]
fn legacy_single_file_get_resolves_without_writes() {
    let (_tmp, config_dir) = setup_test_env();
    write_legacy_snippets(&config_dir);
    assert!(!config_dir.join("libraries").exists());
    let before = file_snapshot(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "legacy-1"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "legacy get failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "echo legacy"
    );

    // No migration: the libraries directory must not appear and no file may
    // change.
    assert!(
        !config_dir.join("libraries").exists(),
        "read-only get must not migrate legacy state"
    );
    assert_eq!(file_snapshot(&config_dir), before);
}

#[test]
fn primary_named_and_all_scopes_resolve() {
    let (_tmp, config_dir) = setup_test_env();
    write_two_libraries(&config_dir);

    // Default scope resolves the primary library.
    let output = snp_in(&config_dir)
        .args(["get", "--id", "work-1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "echo work");

    // Named scope.
    let output = snp_in(&config_dir)
        .args(["get", "--id", "personal-1", "--library", "personal"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "echo personal"
    );

    // `all` scope finds a snippet outside the primary library.
    let output = snp_in(&config_dir)
        .args(["get", "--id", "personal-1", "--library", "all"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "echo personal"
    );
}

#[test]
fn missing_library_errors_are_stable() {
    let (_tmp, config_dir) = setup_test_env();
    write_two_libraries(&config_dir);

    let output = snp_in(&config_dir)
        .args(["get", "--id", "work-1", "--library", "missing"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Library 'missing' does not exist"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn validate_leaves_legacy_state_untouched() {
    let (_tmp, config_dir) = setup_test_env();
    write_legacy_snippets(&config_dir);
    let before = file_snapshot(&config_dir);

    let output = snp_in(&config_dir).args(["validate"]).output().unwrap();
    assert!(
        output.status.success(),
        "validate failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!config_dir.join("libraries").exists());
    assert_eq!(file_snapshot(&config_dir), before);
}

#[test]
fn doctor_and_validate_agree_on_missing_primary() {
    let (_tmp, config_dir) = setup_test_env();
    // One library, no primary: the shared inspection must surface the same
    // condition in both commands.
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        config_dir.join("libraries.toml"),
        "[[libraries]]\nfilename = \"solo\"\nis_primary = false\n",
    )
    .unwrap();
    fs::write(
        libraries_dir.join("solo.toml"),
        "[[snippets]]\nid = \"solo-1\"\ndescription = \"solo\"\ncommand = \"echo solo\"\n",
    )
    .unwrap();

    let validate = snp_in(&config_dir)
        .args(["validate", "--json"])
        .output()
        .unwrap();
    assert!(validate.status.success());
    let report: Value = serde_json::from_slice(&validate.stdout).unwrap();
    let diagnostics = report["diagnostics"].as_array().unwrap();
    let no_primary = diagnostics
        .iter()
        .filter(|d| d["code"] == "W-NO-PRIMARY")
        .count();
    assert_eq!(no_primary, 1);

    let doctor = snp_in(&config_dir)
        .args(["doctor", "--compatibility", "--report", "json"])
        .output()
        .unwrap();
    assert!(doctor.status.success());
    let report: Value = serde_json::from_slice(&doctor.stdout).unwrap();
    let diagnostics = report["diagnostics"].as_array().unwrap();
    let missing = diagnostics
        .iter()
        .filter(|d| d["code"] == "compat.primary_library.missing")
        .count();
    assert_eq!(missing, 1);
}

#[test]
fn validate_and_repair_agree_on_missing_primary() {
    let (_tmp, config_dir) = setup_test_env();
    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        config_dir.join("libraries.toml"),
        "[[libraries]]\nfilename = \"solo\"\nis_primary = false\n",
    )
    .unwrap();
    fs::write(
        libraries_dir.join("solo.toml"),
        "[[snippets]]\nid = \"solo-1\"\ndescription = \"solo\"\ncommand = \"echo solo\"\n",
    )
    .unwrap();

    let validate = snp_in(&config_dir)
        .args(["validate", "--json"])
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&validate.stdout).unwrap();
    let no_primary = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == "W-NO-PRIMARY")
        .count();
    assert_eq!(no_primary, 1);

    let repair = snp_in(&config_dir)
        .args(["repair", "--dry-run"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&repair.stderr);
    assert!(
        stderr.contains("No primary library is set"),
        "repair should report the same missing-primary state: {stderr}"
    );
}

#[test]
fn mcp_reads_leave_filesystem_unchanged() {
    let (_tmp, config_dir) = setup_test_env();
    write_two_libraries(&config_dir);
    let before = file_snapshot(&config_dir);

    let xdg = config_dir.parent().unwrap().to_path_buf();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_snp"));
    command
        .args(["mcp", "serve"])
        .env("XDG_CONFIG_HOME", &xdg)
        .env("SNP_ALLOW_PLAINTEXT_API_KEY", "true")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    fn send(
        stdin: &mut std::process::ChildStdin,
        reader: &mut BufReader<std::process::ChildStdout>,
        value: &Value,
    ) {
        serde_json::to_writer(&mut *stdin, value).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(!line.is_empty());
    }

    send(
        &mut stdin,
        &mut reader,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "plan009", "version": "1" },
            },
        }),
    );
    // Notifications carry no `id` and receive no reply; write without reading.
    serde_json::to_writer(
        &mut stdin,
        &serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
    )
    .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    send(
        &mut stdin,
        &mut reader,
        &serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "snippets_list", "arguments": {}},
        }),
    );
    send(
        &mut stdin,
        &mut reader,
        &serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "snippets_search", "arguments": { "query": "work" }},
        }),
    );
    send(
        &mut stdin,
        &mut reader,
        &serde_json::json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {"name": "snippet_get", "arguments": { "id": "work-1" }},
        }),
    );
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());

    assert_eq!(file_snapshot(&config_dir), before);
}
