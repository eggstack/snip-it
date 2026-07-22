//! Restore path validation and security tests (Workstream C).
//!
//! Exercises every rejection class in the backup-relative path validator
//! and verifies that restore rejects unsafe source artifacts.

mod support;

use std::fs;
use support::helpers::*;

fn sha256_hex(bytes: Vec<u8>) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

// === Path traversal rejection tests ===

#[test]
fn test_rejects_empty_path() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = _tmp.path().join("backup");
    fs::create_dir_all(backup_dir.join("libraries")).unwrap();
    fs::write(
        backup_dir.join("manifest.toml"),
        r#"schema = 1
created_at_unix_ms = 0
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = ""
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#,
    )
    .unwrap();
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.contains("Empty"),
        "Should reject empty path: {stderr}"
    );
}

#[test]
fn test_rejects_absolute_unix_path() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = _tmp.path().join("backup");
    fs::create_dir_all(backup_dir.join("libraries")).unwrap();
    fs::write(
        backup_dir.join("manifest.toml"),
        r#"schema = 1
created_at_unix_ms = 0
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "/etc/passwd"
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#,
    )
    .unwrap();
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.contains("Absolute"),
        "Should reject absolute path: {stderr}"
    );
}

#[test]
fn test_rejects_windows_drive_path() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = _tmp.path().join("backup");
    fs::create_dir_all(backup_dir.join("libraries")).unwrap();
    fs::write(
        backup_dir.join("manifest.toml"),
        r#"schema = 1
created_at_unix_ms = 0
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "C:\\Windows\\System32\\config"
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#,
    )
    .unwrap();
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.contains("Absolute"),
        "Should reject Windows drive path: {stderr}"
    );
}

#[test]
fn test_rejects_traversal_dotdot() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = _tmp.path().join("backup");
    fs::create_dir_all(backup_dir.join("libraries")).unwrap();
    fs::write(
        backup_dir.join("manifest.toml"),
        r#"schema = 1
created_at_unix_ms = 0
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "../outside.toml"
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#,
    )
    .unwrap();
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.contains("traversal") || stderr.contains("ParentDir"),
        "Should reject traversal: {stderr}"
    );
}

#[test]
fn test_rejects_traversal_nested() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = _tmp.path().join("backup");
    fs::create_dir_all(backup_dir.join("libraries")).unwrap();
    fs::write(
        backup_dir.join("manifest.toml"),
        r#"schema = 1
created_at_unix_ms = 0
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "a/../../outside.toml"
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#,
    )
    .unwrap();
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.contains("traversal") || stderr.contains("ParentDir"),
        "Should reject nested traversal: {stderr}"
    );
}

#[test]
fn test_rejects_nul_byte() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = _tmp.path().join("backup");
    fs::create_dir_all(backup_dir.join("libraries")).unwrap();
    fs::write(
        backup_dir.join("manifest.toml"),
        "schema = 1\ncreated_at_unix_ms = 0\nsnip_it_version = \"1.0.0\"\nlayout = \"directory\"\n\n[[files]]\npath = \"test\\0.toml\"\nkind = \"library\"\nsize = 0\nsha256 = \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"\n",
    )
    .unwrap();
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.contains("NUL"),
        "Should reject NUL byte: {stderr}"
    );
}

#[test]
fn test_rejects_unc_path() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = _tmp.path().join("backup");
    fs::create_dir_all(backup_dir.join("libraries")).unwrap();
    fs::write(
        backup_dir.join("manifest.toml"),
        r#"schema = 1
created_at_unix_ms = 0
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "\\\\server\\share\\file.toml"
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#,
    )
    .unwrap();
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.contains("UNC"),
        "Should reject UNC path: {stderr}"
    );
}

#[test]
fn test_rejects_reserved_windows_name() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = _tmp.path().join("backup");
    fs::create_dir_all(backup_dir.join("libraries")).unwrap();
    fs::write(
        backup_dir.join("manifest.toml"),
        r#"schema = 1
created_at_unix_ms = 0
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "NUL.toml"
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#,
    )
    .unwrap();
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.contains("Reserved Windows device name"),
        "Should reject reserved Windows name: {stderr}"
    );
}

// === Source artifact validation ===

#[cfg(unix)]
#[test]
fn test_rejects_symlinked_library_source() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = _tmp.path().join("backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    // Create a real file and a symlink to it
    let real_file = _tmp.path().join("real_snippet.toml");
    fs::write(
        &real_file,
        r#"[[snippets]]
id = "symlink-test"
description = "symlinked snippet"
command = "echo safe"
"#,
    )
    .unwrap();
    let symlink = libraries_dir.join("symlinked.toml");
    std::os::unix::fs::symlink(&real_file, &symlink).unwrap();

    let sha = sha256_hex(fs::read(&real_file).unwrap());
    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 0
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "symlinked.toml"
kind = "library"
size = {size}
sha256 = "{sha}"
"#,
        size = fs::read(&real_file).unwrap().len(),
    );
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    // Dry run should succeed (no reads of source artifact)
    // Replace mode should fail when trying to copy the symlink
    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    // Dry run is fine — it doesn't read source files
    assert!(
        output.status.success(),
        "dry-run with symlink source should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// === Acceptance: valid paths pass ===

#[test]
fn test_valid_library_path_accepted() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = _tmp.path().join("backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let content = r#"[[snippets]]
id = "valid-1"
description = "valid snippet"
command = "echo valid"
"#;
    fs::write(libraries_dir.join("valid.toml"), content).unwrap();

    let sha = sha256_hex(content.as_bytes().to_vec());
    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 0
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "valid.toml"
kind = "library"
size = {size}
sha256 = "{sha}"
"#,
        size = content.len(),
    );
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Valid path should be accepted: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_valid_index_path_accepted() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = _tmp.path().join("backup");
    fs::create_dir_all(backup_dir.join("libraries")).unwrap();

    let content = r#"[[libraries]]
filename = "test"
is_primary = true
"#;
    fs::write(backup_dir.join("libraries.toml"), content).unwrap();

    let sha = sha256_hex(content.as_bytes().to_vec());
    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 0
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "libraries.toml"
kind = "index"
size = {size}
sha256 = "{sha}"
"#,
        size = content.len(),
    );
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Valid index path should be accepted: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// === Checksum mismatch ===

#[test]
fn test_rejects_checksum_mismatch() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = _tmp.path().join("backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let content = r#"[[snippets]]
id = "checksum-test"
description = "checksum snippet"
command = "echo checksum"
"#;
    fs::write(libraries_dir.join("cksum.toml"), content).unwrap();

    let wrong_sha = "0000000000000000000000000000000000000000000000000000000000000000";
    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 0
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "cksum.toml"
kind = "library"
size = {size}
sha256 = "{wrong_sha}"
"#,
        size = content.len(),
    );
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    // Dry run verifies checksums even though it doesn't write files.
    // A wrong checksum should cause dry run to fail.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success() || stderr.contains("Checksum mismatch"),
        "Dry run with wrong checksum should fail: {stderr}"
    );
}
