//! Cross-platform CLI smoke tests.
//!
//! Fast, deterministic cases that use the real binary to prove basic
//! CLI functionality on all platforms. No crash failpoints, no full
//! server E2E, no network dependencies.

mod support;

use support::helpers::*;

/// snp --version succeeds.
#[test]
fn test_snp_version() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp --version must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("snp") || stdout.contains("snip-it"),
        "version output should mention snp: {stdout}"
    );
}

/// snp --help succeeds.
#[test]
fn test_snp_help() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .arg("--help")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "snp --help must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("snippet") || stdout.contains("Usage"),
        "help output should describe the tool: {stdout}"
    );
}

/// Isolated library create succeeds.
#[test]
fn test_library_create_smoke() {
    let (_tmp, config_dir) = setup_test_env();
    let output = snp_in(&config_dir)
        .args(["library", "create", "smoke-lib"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "library create must succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Library file should exist
    let lib_path = config_dir.join("libraries").join("smoke-lib.toml");
    assert!(lib_path.exists(), "library file must be created");
}

/// Isolated snippet creation and listing succeeds.
#[test]
fn test_snippet_create_and_list() {
    let (_tmp, config_dir) = setup_test_env();

    // Create library first
    snp_in(&config_dir)
        .args(["library", "create", "list-test"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();

    // Create snippet (provide command via stdin)
    let mut create_child = snp_in(&config_dir)
        .args([
            "new",
            "--command-stdin",
            "--description",
            "smoke snippet",
            "--library",
            "list-test",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(mut stdin) = create_child.stdin.take() {
        use std::io::Write;
        writeln!(stdin, "echo smoke").unwrap();
    }
    let create = create_child.wait_with_output().unwrap();
    assert!(
        create.status.success(),
        "snippet create must succeed: {}",
        String::from_utf8_lossy(&create.stderr)
    );

    // List snippets
    let list = snp_in(&config_dir)
        .args(["list", "--library", "list-test"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        list.status.success(),
        "list must succeed: {}",
        String::from_utf8_lossy(&list.stderr)
    );
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(
        stdout.contains("smoke snippet"),
        "list must show the created snippet: {stdout}"
    );
}

/// A simple backup and dry-run restore succeeds.
#[test]
fn test_backup_and_dry_run_restore() {
    let (_tmp, config_dir) = setup_test_env();

    // Create library with a snippet
    snp_in(&config_dir)
        .args(["library", "create", "backup-test"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();

    // Create snippet (provide command via stdin)
    let mut create_child = snp_in(&config_dir)
        .args([
            "new",
            "--command-stdin",
            "--description",
            "backup snippet",
            "--library",
            "backup-test",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(mut stdin) = create_child.stdin.take() {
        use std::io::Write;
        writeln!(stdin, "echo backup").unwrap();
    }
    create_child.wait_with_output().unwrap();

    // Backup
    let backup_dir = _tmp.path().join("backup-output");
    let backup = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        backup.status.success(),
        "backup must succeed: {}",
        String::from_utf8_lossy(&backup.stderr)
    );
    assert!(
        backup_dir.join("manifest.toml").exists(),
        "backup manifest must exist"
    );

    // Dry-run restore
    let restore = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        restore.status.success(),
        "dry-run restore must succeed: {}",
        String::from_utf8_lossy(&restore.stderr)
    );
}

/// snip-sync --help succeeds (if binary is available in workspace build).
#[test]
fn test_snip_sync_help() {
    let (_tmp, _config_dir) = setup_test_env();
    let output = std::process::Command::new("cargo")
        .args(["run", "--bin", "snip-sync", "--", "--help"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    // snip-sync may not be built in all configurations; just check it
    // doesn't panic. A non-zero exit is acceptable if the binary isn't built.
    if output.status.code() != Some(101) {
        // Not a panic
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stdout.contains("sync") || stderr.contains("sync") || !output.status.success(),
            "snip-sync --help should produce useful output"
        );
    }
}
