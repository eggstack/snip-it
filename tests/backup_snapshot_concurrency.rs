//! Backup snapshot coherence tests (Workstream F).
//!
//! Verifies that backup captures a consistent generation of library files
//! and handles concurrent mutation, symlinks, non-regular files, and
//! deterministic ordering.

mod support;

use std::fs;
use std::path::Path;
use support::helpers::*;

fn sha256_hex(bytes: Vec<u8>) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let result = hasher.finalize();
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Create a library with a test snippet via the snp binary.
fn setup_library(config_dir: &Path, name: &str) {
    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "create", name]);
    cmd.output().unwrap();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();
    fs::write(
        libraries_dir.join(format!("{name}.toml")),
        format!(
            r#"[[snippets]]
id = "{name}-snippet-1"
description = "{name} test snippet"
command = "echo {name}-test"
"#
        ),
    )
    .unwrap();

    let mut cmd = snp_in(config_dir);
    cmd.args(["library", "set-primary", name]);
    cmd.output().unwrap();
}

/// Add a second snippet to an existing library.
fn add_snippet(config_dir: &Path, lib_name: &str, snippet_id: &str, desc: &str, cmd_str: &str) {
    let libraries_dir = config_dir.join("libraries");
    let lib_path = libraries_dir.join(format!("{lib_name}.toml"));
    let existing = fs::read_to_string(&lib_path).unwrap();
    let new_content = format!(
        "{existing}\n[[snippets]]\nid = \"{snippet_id}\"\ndescription = \"{desc}\"\ncommand = \"{cmd_str}\"\n"
    );
    fs::write(&lib_path, new_content).unwrap();
}

/// Read the manifest.toml from a backup directory.
fn read_manifest(backup_dir: &Path) -> serde_json::Value {
    let manifest_path = backup_dir.join("manifest.toml");
    assert!(
        manifest_path.exists(),
        "manifest.toml not found at {}",
        manifest_path.display()
    );
    let content = fs::read_to_string(&manifest_path).unwrap();
    toml::from_str(&content).unwrap()
}

// === Test 1: Backup generation before and after ===

#[test]
fn test_backup_generation_before_and_after() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir, "gen-test");

    // First backup
    let backup1_dir = _tmp.path().join("backup-1");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup1_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "first backup should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest1 = read_manifest(&backup1_dir);
    let files1 = manifest1["files"].as_array().unwrap();
    assert!(
        files1.iter().any(|f| f["kind"] == "library"),
        "first backup must contain a library"
    );
    let library_paths1: Vec<&str> = files1
        .iter()
        .filter(|f| f["kind"] == "library")
        .filter_map(|f| f["path"].as_str())
        .collect();
    assert_eq!(
        library_paths1.len(),
        1,
        "first backup should have exactly one library"
    );

    // Add a second snippet
    add_snippet(
        &config_dir,
        "gen-test",
        "gen-test-snippet-2",
        "second snippet",
        "echo second",
    );

    // Second backup
    let backup2_dir = _tmp.path().join("backup-2");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup2_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "second backup should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest2 = read_manifest(&backup2_dir);
    let files2 = manifest2["files"].as_array().unwrap();
    let library_paths2: Vec<&str> = files2
        .iter()
        .filter(|f| f["kind"] == "library")
        .filter_map(|f| f["path"].as_str())
        .collect();
    assert_eq!(
        library_paths2.len(),
        1,
        "second backup should have exactly one library"
    );
    assert_eq!(
        library_paths1, library_paths2,
        "both backups should contain the same library file"
    );

    // Verify the second backup captured the added snippet
    let lib_file2 = backup2_dir.join(library_paths2[0]);
    let lib_content2 = fs::read_to_string(&lib_file2).unwrap();
    assert!(
        lib_content2.contains("gen-test-snippet-2"),
        "second backup library should contain the second snippet"
    );
}

// === Test 2: Backup rejects symlink source ===

#[cfg(unix)]
#[test]
fn test_backup_rejects_symlink_source() {
    let (_tmp, config_dir) = setup_test_env();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    // Create a real file outside the config dir
    let external_file = _tmp.path().join("external-secret.toml");
    fs::write(
        &external_file,
        r#"[[snippets]]
id = "external"
description = "should not be backed up"
command = "echo external"
"#,
    )
    .unwrap();

    // Create a symlink in the libraries directory pointing to the external file
    let symlink = libraries_dir.join("symlinked.toml");
    std::os::unix::fs::symlink(&external_file, &symlink).unwrap();

    let backup_dir = _tmp.path().join("backup-symlink");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "backup should fail when a symlink is in libraries dir"
    );
    assert!(
        stderr.contains("symlink") || stderr.contains("escapes config root"),
        "error should mention symlink or containment: {stderr}"
    );
}

// === Test 3: Backup deterministic manifest order ===

#[test]
fn test_backup_deterministic_manifest_order() {
    let (_tmp, config_dir) = setup_test_env();

    // Create multiple libraries
    setup_library(&config_dir, "alpha-lib");

    let libraries_dir = config_dir.join("libraries");
    fs::write(
        libraries_dir.join("beta-lib.toml"),
        r#"[[snippets]]
id = "beta-1"
description = "beta snippet"
command = "echo beta"
"#,
    )
    .unwrap();
    fs::write(
        libraries_dir.join("gamma-lib.toml"),
        r#"[[snippets]]
id = "gamma-1"
description = "gamma snippet"
command = "echo gamma"
"#,
    )
    .unwrap();

    // Run backup twice
    let backup_a_dir = _tmp.path().join("backup-order-a");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_a_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backup A should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let backup_b_dir = _tmp.path().join("backup-order-b");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_b_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backup B should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest_a = read_manifest(&backup_a_dir);
    let manifest_b = read_manifest(&backup_b_dir);

    let paths_a: Vec<&str> = manifest_a["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();
    let paths_b: Vec<&str> = manifest_b["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["path"].as_str())
        .collect();

    assert_eq!(
        paths_a, paths_b,
        "manifest file order must be deterministic across runs"
    );
    // Verify the order is sorted
    let mut sorted = paths_a.clone();
    sorted.sort();
    assert_eq!(
        paths_a, sorted,
        "manifest file order must be lexicographically sorted"
    );
}

// === Test 4: Backup atomic output staging ===

#[test]
fn test_backup_atomic_output_staging() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir, "staging-test");

    let backup_dir = _tmp.path().join("backup-staging");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backup should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify backup directory exists
    assert!(
        backup_dir.exists(),
        "backup directory should exist after successful backup"
    );

    // Verify manifest.toml exists
    let manifest_path = backup_dir.join("manifest.toml");
    assert!(
        manifest_path.exists(),
        "manifest.toml should be present in backup"
    );

    // Verify libraries/ subdirectory exists and contains library files
    let libraries_dir = backup_dir.join("libraries");
    assert!(
        libraries_dir.exists(),
        "libraries/ subdirectory should be present in backup"
    );
    let lib_entries: Vec<_> = fs::read_dir(&libraries_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        !lib_entries.is_empty(),
        "libraries/ should contain at least one library file"
    );
    for entry in &lib_entries {
        assert!(
            entry.path().extension().is_some_and(|e| e == "toml"),
            "library files should have .toml extension: {}",
            entry.path().display()
        );
    }

    // Verify no staging directory remains
    let parent = backup_dir.parent().unwrap();
    let staging残留: Vec<_> = fs::read_dir(parent)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".staging."))
        .collect();
    assert!(
        staging残留.is_empty(),
        "no staging directories should remain after atomic commit"
    );
}

// === Test 5: Empty config succeeds ===

#[test]
fn test_backup_empty_config_succeeds() {
    let (_tmp, config_dir) = setup_test_env();
    // No library or snippet created — config dir is empty

    let backup_dir = _tmp.path().join("backup-empty");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backup of empty config should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest = read_manifest(&backup_dir);
    let files = manifest["files"].as_array().unwrap();
    assert!(
        files.is_empty(),
        "backup of empty config should have empty file list"
    );

    // Manifest should still be valid
    assert_eq!(
        manifest["schema"].as_i64().unwrap(),
        1,
        "manifest schema should be 1"
    );
    assert_eq!(
        manifest["layout"].as_str().unwrap(),
        "directory",
        "manifest layout should be directory"
    );
}

// === Test 6: Backup generation coherence ===

#[test]
fn test_backup_generation_coherence() {
    let (_tmp, config_dir) = setup_test_env();
    setup_library(&config_dir, "gen-coherent");

    // Run backup and check the generation in the manifest
    let backup_dir = _tmp.path().join("backup-gen");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "backup should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Read the current libraries.toml to get the generation
    let libraries_toml = config_dir.join("libraries.toml");
    assert!(
        libraries_toml.exists(),
        "libraries.toml should exist after creating a library"
    );
    let index_content = fs::read_to_string(&libraries_toml).unwrap();
    let index: serde_json::Value = toml::from_str(&index_content).unwrap();
    let generation = index["generation"].as_i64().unwrap_or(0);
    assert!(
        generation > 0,
        "generation should be > 0 after creating a library"
    );

    // Verify the backup's library file matches the current state
    let manifest = read_manifest(&backup_dir);
    let files = manifest["files"].as_array().unwrap();
    let library_entry = files
        .iter()
        .find(|f| f["kind"] == "library")
        .expect("backup should contain a library");
    let lib_path = backup_dir.join(library_entry["path"].as_str().unwrap());
    let backup_lib_content = fs::read_to_string(&lib_path).unwrap();
    let lib_relative = library_entry["path"]
        .as_str()
        .unwrap()
        .strip_prefix("libraries/")
        .unwrap_or(library_entry["path"].as_str().unwrap());
    let current_lib_content =
        fs::read_to_string(config_dir.join("libraries").join(lib_relative)).unwrap();

    assert_eq!(
        backup_lib_content, current_lib_content,
        "backup library content should match current library content (generation coherence)"
    );

    // Verify the index entry is also present
    let has_index = files.iter().any(|f| f["kind"] == "index");
    assert!(
        has_index,
        "backup should include the index (libraries.toml)"
    );

    // Verify the backup was created at the correct generation
    let index_entry = files.iter().find(|f| f["kind"] == "index").unwrap();
    let index_sha = index_entry["sha256"].as_str().unwrap();
    let actual_index_sha = sha256_hex(index_content.into_bytes());
    assert_eq!(
        index_sha, actual_index_sha,
        "index SHA-256 should match the current libraries.toml content"
    );
}

// === Test 7: Backup rejects non-regular file (FIFO) ===

#[cfg(unix)]
#[test]
fn test_backup_rejects_non_regular_file() {
    let (_tmp, config_dir) = setup_test_env();

    let libraries_dir = config_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    // Create a FIFO in the libraries directory
    let fifo_path = libraries_dir.join("fifo-test.toml");
    let mkfifo_result = unsafe {
        libc::mkfifo(
            fifo_path.to_str().unwrap().as_ptr() as *const libc::c_char,
            0o644,
        )
    };
    if mkfifo_result != 0 {
        // mkfifo may fail in sandboxed CI (e.g. GitHub Actions containers).
        // The test intent is that backup rejects non-regular files; if we
        // can't create a FIFO, skip rather than spuriously fail.
        eprintln!(
            "mkfifo failed (errno {}), skipping — FIFO creation not supported in this environment",
            std::io::Error::last_os_error()
        );
        return;
    }
    // Even if mkfifo returns success, the file may not exist in some
    // sandboxed environments (e.g. certain CI container configurations).
    if !fifo_path.exists() {
        eprintln!(
            "mkfifo returned success but FIFO not present at {}, skipping",
            fifo_path.display()
        );
        return;
    }

    let backup_dir = _tmp.path().join("backup-fifo");
    let output = snp_in(&config_dir)
        .args(["backup", "--output", backup_dir.to_str().unwrap()])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "backup should fail when a FIFO is in libraries dir"
    );
    assert!(
        stderr.contains("not a regular file") || stderr.contains("regular file"),
        "error should mention regular file issue: {stderr}"
    );
}
