mod support;

use std::fs;
use std::path::{Path, PathBuf};
use support::helpers::*;

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Assert that a rejected restore created no transaction artifacts,
/// no pending marker, and no live destination writes.
fn assert_no_side_effects(config_dir: &Path, _backup_dir: &Path) {
    let txn_dir = config_dir.join(".transaction");
    if txn_dir.exists() {
        let entries: Vec<_> = fs::read_dir(&txn_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            entries.is_empty(),
            "transaction directory should be empty after rejection, found: {:?}",
            entries.iter().map(|e| e.file_name()).collect::<Vec<_>>()
        );
    }
    assert!(
        !config_dir.join("auto-sync-pending.toml").exists(),
        "no pending marker should exist after rejection"
    );
    // No live destination files should have been written.
    let libraries_dir = config_dir.join("libraries");
    if libraries_dir.exists() {
        // It's OK if the directory existed before (from setup), but
        // no new .toml files should have been created by restore.
        // We check that default.toml (the backup's library) doesn't exist.
        assert!(
            !libraries_dir.join("default.toml").exists(),
            "no live library should be written after rejection"
        );
    }
    // libraries.toml (the index) should not have been written.
    assert!(
        !config_dir.join("libraries.toml").exists(),
        "no live index should be written after rejection"
    );
}

/// Shared fixture builder for manifest tests.
///
/// Creates a valid backup directory with exact sizes and SHA-256 hashes,
/// allowing one targeted mutation per test case.
#[allow(dead_code)]
struct BackupFixture {
    root: tempfile::TempDir,
    backup_dir: PathBuf,
    lib_content: String,
    index_content: String,
}

#[allow(dead_code)]
impl BackupFixture {
    /// Create a valid replace-mode backup with one library and one index.
    fn valid_replace() -> Self {
        let root = tempfile::TempDir::new().unwrap();
        let backup_dir = root.path().join("backup");
        let libraries_dir = backup_dir.join("libraries");
        fs::create_dir_all(&libraries_dir).unwrap();

        let lib_content = r#"[[snippets]]
id = "test-1"
description = "test snippet"
command = "echo test"
"#;
        fs::write(libraries_dir.join("default.toml"), lib_content).unwrap();

        let index_content = r#"[[libraries]]
filename = "default"
is_primary = true
"#;
        fs::write(backup_dir.join("libraries.toml"), index_content).unwrap();

        let fixture = Self {
            root,
            backup_dir,
            lib_content: lib_content.to_string(),
            index_content: index_content.to_string(),
        };
        fixture.write_manifest();
        fixture
    }

    /// Rewrite the index content and regenerate the manifest.
    fn rewrite_index(&mut self, content: &str) {
        self.index_content = content.to_string();
        fs::write(self.backup_dir.join("libraries.toml"), content).unwrap();
        self.write_manifest();
    }

    /// Add a library file and regenerate the manifest.
    fn add_library(&mut self, name: &str, content: &str) {
        let libraries_dir = self.backup_dir.join("libraries");
        fs::create_dir_all(&libraries_dir).unwrap();
        fs::write(libraries_dir.join(name), content).unwrap();
        // Append to lib_content tracking for manifest generation.
        // For simplicity, we just rewrite the manifest with all known files.
        self.write_manifest();
    }

    /// Write the manifest with exact sizes and hashes for all known files.
    fn write_manifest(&self) {
        let libraries_dir = self.backup_dir.join("libraries");
        let mut files_section = String::new();

        // Add library files.
        if libraries_dir.exists() {
            for entry in fs::read_dir(&libraries_dir).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name();
                let name_str = name.to_string_lossy().to_string();
                let content = fs::read(entry.path()).unwrap();
                let sha = sha256_hex(&content);
                files_section.push_str(&format!(
                    "[[files]]\npath = \"{name_str}\"\nkind = \"library\"\nsize = {}\nsha256 = \"{sha}\"\n\n",
                    content.len(),
                ));
            }
        }

        // Add index.
        let index_sha = sha256_hex(self.index_content.as_bytes());
        files_section.push_str(&format!(
            "[[files]]\npath = \"libraries.toml\"\nkind = \"index\"\nsize = {}\nsha256 = \"{index_sha}\"\n\n",
            self.index_content.len(),
        ));

        let manifest = format!(
            r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

{files_section}"#
        );
        fs::write(self.backup_dir.join("manifest.toml"), manifest).unwrap();
    }

    /// Path to the backup directory.
    fn path(&self) -> &Path {
        &self.backup_dir
    }
}

/// Create a valid backup directory with a library and index, returning (backup_dir, tmp).
fn create_valid_backup(tmp: &tempfile::TempDir) -> std::path::PathBuf {
    let backup_dir = tmp.path().join("test-backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let lib_content = r#"[[snippets]]
id = "test-1"
description = "test snippet"
command = "echo test"
"#;
    fs::write(libraries_dir.join("default.toml"), lib_content).unwrap();

    let index_content = r#"[[libraries]]
filename = "default"
is_primary = true
"#;
    fs::write(backup_dir.join("libraries.toml"), index_content).unwrap();

    let lib_sha = sha256_hex(lib_content.as_bytes());
    let index_sha = sha256_hex(index_content.as_bytes());

    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "default.toml"
kind = "library"
size = {}
sha256 = "{lib_sha}"

[[files]]
path = "libraries.toml"
kind = "index"
size = {}
sha256 = "{index_sha}"
"#,
        lib_content.len(),
        index_content.len(),
    );
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    backup_dir
}

// === 1. Unknown entry kind ===

#[test]
fn test_rejects_unknown_entry_kind() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let lib_content = "placeholder";
    fs::write(libraries_dir.join("default.toml"), lib_content).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "default.toml"
kind = "unknown_kind"
size = 11
sha256 = "4097889236a2af26c293033feb964c4cf118c0224e0d063fec0a89e9d0569ef2"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject unknown entry kind"
    );
    assert_no_side_effects(&_config_dir, &backup_dir);
}

// === 2. Schema version zero ===

#[test]
fn test_rejects_schema_version_zero() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let lib_content = "placeholder";
    fs::write(libraries_dir.join("default.toml"), lib_content).unwrap();

    let manifest = r#"schema = 0
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "default.toml"
kind = "library"
size = 11
sha256 = "4097889236a2af26c293033feb964c4cf118c0224e0d063fec0a89e9d0569ef2"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject schema version 0"
    );
    assert_no_side_effects(&_config_dir, &backup_dir);
}

// === 3. Future schema version ===

#[test]
fn test_rejects_future_schema_version() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let lib_content = "placeholder";
    fs::write(libraries_dir.join("default.toml"), lib_content).unwrap();

    let manifest = r#"schema = 999
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "default.toml"
kind = "library"
size = 11
sha256 = "4097889236a2af26c293033feb964c4cf118c0224e0d063fec0a89e9d0569ef2"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject future schema version 999"
    );
    assert_no_side_effects(&_config_dir, &backup_dir);
}

// === 4. Duplicate destination paths ===

#[test]
fn test_rejects_duplicate_destination_paths() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let lib_content = "placeholder";
    fs::write(libraries_dir.join("default.toml"), lib_content).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "default.toml"
kind = "library"
size = 11
sha256 = "4097889236a2af26c293033feb964c4cf118c0224e0d063fec0a89e9d0569ef2"

[[files]]
path = "default.toml"
kind = "library"
size = 11
sha256 = "4097889236a2af26c293033feb964c4cf118c0224e0d063fec0a89e9d0569ef2"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject duplicate destination paths"
    );
    assert_no_side_effects(&_config_dir, &backup_dir);
}

// === 5. Empty path ===

#[test]
fn test_rejects_empty_path() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    fs::create_dir_all(&backup_dir).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = ""
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "restore should reject empty path");
}

// === 6. Absolute path ===

#[test]
fn test_rejects_absolute_path() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    fs::create_dir_all(&backup_dir).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "/etc/passwd"
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject absolute path"
    );
}

// === 7. Traversal path ===

#[test]
fn test_rejects_traversal_path() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    fs::create_dir_all(&backup_dir).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "../escape.toml"
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject traversal path"
    );
}

// === 8. Windows reserved name ===

#[test]
fn test_rejects_windows_reserved_name() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    fs::create_dir_all(&backup_dir).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "CON.toml"
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject Windows reserved name"
    );
}

// === 9. Trailing dot ===

#[test]
fn test_rejects_trailing_dot() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    fs::create_dir_all(&backup_dir).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "file.toml."
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject trailing dot in path"
    );
}

// === 10. Trailing space ===

#[test]
fn test_rejects_trailing_space() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    fs::create_dir_all(&backup_dir).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "file.toml "
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject trailing space in path"
    );
}

// === 11. Control character ===

#[test]
fn test_rejects_control_character() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    fs::create_dir_all(&backup_dir).unwrap();

    // Write manifest with NUL byte in path via raw bytes
    let manifest_bytes = b"schema = 1\ncreated_at_unix_ms = 1700000000000\nsnip_it_version = \"1.0.0\"\nlayout = \"directory\"\n\n[[files]]\npath = \"file\x00.toml\"\nkind = \"library\"\nsize = 0\nsha256 = \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"\n";
    fs::write(backup_dir.join("manifest.toml"), manifest_bytes).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject control character in path"
    );
}

// === 12. Valid schema 1 succeeds ===

#[test]
fn test_valid_schema_1_succeeds() {
    let (tmp, config_dir) = setup_test_env();
    let backup_dir = create_valid_backup(&tmp);

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "restore dry-run should succeed for valid manifest: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// === 13. Library must be flat filename ===

#[test]
fn test_library_must_be_flat_filename() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    let libraries_dir = backup_dir.join("libraries").join("subdir");
    fs::create_dir_all(&libraries_dir).unwrap();

    fs::write(libraries_dir.join("file.toml"), "content").unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "subdir/file.toml"
kind = "library"
size = 7
sha256 = "ed7002b439e9ac845f22357d822bac1444730fbdb6016d3ec9432297b9ec9f73"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject nested library path"
    );
}

// === 14. Library must end with .toml ===

#[test]
fn test_library_must_end_with_toml() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    fs::write(libraries_dir.join("file.txt"), "content").unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "file.txt"
kind = "library"
size = 7
sha256 = "ed7002b439e9ac845f22357d822bac1444730fbdb6016d3ec9432297b9ec9f73"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject library without .toml extension"
    );
}

// === 15. Index must be libraries.toml ===

#[test]
fn test_index_must_be_libraries_toml() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    fs::create_dir_all(&backup_dir).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "wrong-name.toml"
kind = "index"
size = 7
sha256 = "ed7002b439e9ac845f22357d822bac1444730fbdb6016d3ec9432297b9ec9f73"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject index with wrong path"
    );
}

// === 16. Duplicate destinations with different case ===

#[test]
fn test_rejects_case_folded_duplicate_destinations() {
    let (_tmp, _config_dir) = setup_test_env();
    let fixture = BackupFixture::valid_replace();

    // Create a second library file with distinct content so hashes differ.
    let content_b = r#"[[snippets]]
id = "test-2"
description = "second snippet"
command = "echo second"
"#;
    let libraries_dir = fixture.backup_dir.join("libraries");
    fs::write(libraries_dir.join("Default.toml"), content_b).unwrap();

    // Compute hashes for both files using the same sha256_hex the fixture uses.
    let content_a = fixture.lib_content.clone();
    let hash_a = sha256_hex(content_a.as_bytes());
    let hash_b = sha256_hex(content_b.as_bytes());

    // Manually write a manifest that declares both paths.
    // On case-insensitive filesystems only one file exists on disk, but
    // the manifest still declares both entries — the validator rejects
    // the case-folded duplicate.
    let index_hash = sha256_hex(fixture.index_content.as_bytes());
    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "default.toml"
kind = "library"
size = {}
sha256 = "{hash_a}"

[[files]]
path = "Default.toml"
kind = "library"
size = {}
sha256 = "{hash_b}"

[[files]]
path = "libraries.toml"
kind = "index"
size = {}
sha256 = "{index_hash}"
"#,
        content_a.len(),
        content_b.len(),
        fixture.index_content.len(),
    );
    fs::write(fixture.backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args([
            "restore",
            fixture.path().to_str().unwrap(),
            "--mode",
            "dry-run",
        ])
        .output()
        .unwrap();
    // Dry-run must reject — case-folded duplicates must be unconditionally
    // rejected on every platform (the plan requires this test never accepts
    // success).
    assert!(
        !output.status.success(),
        "Case-folded duplicate destinations must be rejected, but dry-run succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr_lower = stderr.to_lowercase();
    assert!(
        stderr_lower.contains("duplicate")
            || stderr_lower.contains("already")
            || stderr_lower.contains("conflict")
            || stderr_lower.contains("collides"),
        "Should reject case-folded duplicates with clear message, got: {stderr}"
    );
}

// === 17. Windows drive-relative path rejection (Workstream G) ===

/// Verify that Windows drive-relative paths like "C:foo.toml" are rejected
/// by the backup path validator, even on non-Windows platforms.
#[test]
fn test_rejects_windows_drive_relative_path() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    fs::create_dir_all(&backup_dir).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "C:Windows\\system32\\evil.toml"
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject Windows drive-relative path"
    );
}

// === 18. UNC path rejection (Workstream G) ===

/// Verify that Windows UNC paths are rejected.
#[test]
fn test_rejects_unc_path() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    fs::create_dir_all(&backup_dir).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "\\\\server\\share\\file.toml"
kind = "library"
size = 0
sha256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(!output.status.success(), "restore should reject UNC path");
}

// === 19. Duplicate incoming snippet IDs rejected (Workstream G) ===

/// Verify that a backup with duplicate snippet IDs within the same library
/// is rejected during restore validation.
#[test]
fn test_rejects_duplicate_snippet_ids() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("dup-backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    // Library with duplicate snippet IDs
    let lib_content = r#"[[snippets]]
id = "dup-id"
description = "first snippet"
command = "echo first"

[[snippets]]
id = "dup-id"
description = "second snippet"
command = "echo second"
"#;
    fs::write(libraries_dir.join("dup.toml"), lib_content).unwrap();

    let index = r#"[[libraries]]
filename = "dup"
is_primary = true
"#;
    fs::write(backup_dir.join("libraries.toml"), index).unwrap();

    let lib_hash = sha256_hex(lib_content.as_bytes());
    let index_hash = sha256_hex(index.as_bytes());

    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "dup.toml"
kind = "library"
size = {lib_size}
sha256 = "{lib_hash}"

[[files]]
path = "libraries.toml"
kind = "index"
size = {idx_size}
sha256 = "{index_hash}"
"#,
        lib_size = lib_content.len(),
        idx_size = index.len(),
    );
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "replace"])
        .output()
        .unwrap();
    // Restore must reject duplicate snippet IDs with a clear error message.
    // This is a domain contract: each snippet must have a unique ID.
    assert!(
        !output.status.success(),
        "restore should reject duplicate snippet IDs"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Duplicate snippet ID"),
        "Should reject duplicate IDs with clear message, got: {stderr}"
    );
}

// === 20. Restore rejects unknown entry kind in write mode ===

/// Verify that restore rejects unknown manifest entry kinds when not
/// in dry-run mode (the catch-all arm must error, not write to unknown paths).
#[test]
fn test_rejects_unknown_kind_in_replace_mode() {
    let (tmp, _config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let lib_content = "placeholder";
    fs::write(libraries_dir.join("default.toml"), lib_content).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "default.toml"
kind = "unknown_kind"
size = 11
sha256 = "4097889236a2af26c293033feb964c4cf118c0224e0d063fec0a89e9d0569ef2"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&_config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "replace"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject unknown entry kind in replace mode"
    );
}

// === 18. Duplicate library names in index ===

#[test]
fn test_rejects_duplicate_library_names_in_index() {
    let (_tmp, _config_dir) = setup_test_env();
    let mut fixture = BackupFixture::valid_replace();

    // Rewrite the index to have duplicate library filenames.
    // The manifest is regenerated with correct hashes for the new index,
    // so size/checksum pass. Only the semantic validator rejects.
    fixture.rewrite_index(
        r#"[[libraries]]
filename = "default"
is_primary = true

[[libraries]]
filename = "default"
is_primary = false
"#,
    );

    let output = snp_in(&_config_dir)
        .args([
            "restore",
            fixture.path().to_str().unwrap(),
            "--mode",
            "replace",
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject duplicate library names in index"
    );
    assert_no_side_effects(&_config_dir, fixture.path());

    // Assert no transaction artifacts were created.
    let txn_dir = _config_dir.join(".transaction");
    assert!(
        !txn_dir.exists() || txn_dir.read_dir().unwrap().next().is_none(),
        "no transaction artifacts should exist after manifest rejection"
    );
}

// === 19. Multiple primary libraries in index ===

#[test]
fn test_rejects_multiple_primary_libraries() {
    let (_tmp, _config_dir) = setup_test_env();
    let mut fixture = BackupFixture::valid_replace();

    // Add a second library file.
    fixture.add_library(
        "second.toml",
        r#"[[snippets]]
id = "test-2"
description = "second snippet"
command = "echo second"
"#,
    );

    // Rewrite the index to have two primaries.
    // The manifest is regenerated with correct hashes for all files,
    // so size/checksum pass. Only the semantic validator rejects.
    fixture.rewrite_index(
        r#"[[libraries]]
filename = "default"
is_primary = true

[[libraries]]
filename = "second"
is_primary = true
"#,
    );

    let output = snp_in(&_config_dir)
        .args([
            "restore",
            fixture.path().to_str().unwrap(),
            "--mode",
            "replace",
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject multiple primary libraries"
    );
    assert_no_side_effects(&_config_dir, fixture.path());
}

// === 20. Index references missing library artifact ===

#[test]
fn test_rejects_index_references_missing_library() {
    let (_tmp, _config_dir) = setup_test_env();
    let mut fixture = BackupFixture::valid_replace();

    // Rewrite the index to reference a library that doesn't exist.
    // The manifest is regenerated with correct hashes for the new index,
    // so size/checksum pass. The semantic validator rejects because the
    // index references a nonexistent library artifact.
    fixture.rewrite_index(
        r#"[[libraries]]
filename = "default"
is_primary = true

[[libraries]]
filename = "nonexistent"
is_primary = false
"#,
    );

    let output = snp_in(&_config_dir)
        .args([
            "restore",
            fixture.path().to_str().unwrap(),
            "--mode",
            "replace",
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject index references to missing library artifacts"
    );
    assert_no_side_effects(&_config_dir, fixture.path());
}

// === 21. Library artifact not referenced by index (replace mode) ===

#[test]
fn test_rejects_unreferenced_library_in_replace_mode() {
    let (_tmp, _config_dir) = setup_test_env();
    let mut fixture = BackupFixture::valid_replace();

    // Add a library file that is not referenced by the index.
    fixture.add_library(
        "extra.toml",
        r#"[[snippets]]
id = "extra-1"
description = "extra snippet"
command = "echo extra"
"#,
    );

    // The manifest is regenerated with correct hashes for all files
    // (including extra.toml). Size/checksum pass. The semantic
    // validator rejects because extra.toml is not in the index.
    let output = snp_in(&_config_dir)
        .args([
            "restore",
            fixture.path().to_str().unwrap(),
            "--mode",
            "replace",
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject unreferenced library in replace mode"
    );
    assert_no_side_effects(&_config_dir, fixture.path());
}

// === 22. No journal, artifact, pending, or live write on rejection ===

#[test]
fn test_invalid_manifest_creates_no_transaction_artifacts() {
    let (tmp, config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let lib_content = "placeholder";
    fs::write(libraries_dir.join("default.toml"), lib_content).unwrap();

    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "default.toml"
kind = "library"
size = 11
sha256 = "4097889236a2af26c293033feb964c4cf118c0224e0d063fec0a89e9d0569ef2"

[[files]]
path = "libraries.toml"
kind = "index"
size = 68
sha256 = "4097889236a2af26c293033feb964c4cf118c0224e0d063fec0a89e9d0569ef2"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    // The index references "default" but the manifest has no index entry
    // with a valid libraries.toml file. Actually, the manifest has an index
    // entry but the file doesn't exist. This should fail at source file
    // checks (file missing), not at semantic validation.
    //
    // Instead, let's create a manifest with a duplicate destination to
    // trigger a structural validation failure.
    let manifest = r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "default.toml"
kind = "library"
size = 11
sha256 = "4097889236a2af26c293033feb964c4cf118c0224e0d063fec0a89e9d0569ef2"

[[files]]
path = "default.toml"
kind = "library"
size = 11
sha256 = "4097889236a2af26c293033feb964c4cf118c0224e0d063fec0a89e9d0569ef2"
"#;
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "replace"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject duplicate destinations"
    );

    assert_no_side_effects(&config_dir, &backup_dir);
}

// === 23. Oversized source ===

/// A library file whose manifest-declared size exceeds MAX_RESTORE_SOURCE_SIZE
/// (10 MiB) must be rejected before any transaction artifacts are created.
#[test]
fn test_rejects_oversized_source() {
    let (tmp, config_dir) = setup_test_env();
    let backup_dir = tmp.path().join("bad-backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let lib_content = "placeholder";
    fs::write(libraries_dir.join("default.toml"), lib_content).unwrap();

    // Declare size as 11 MiB (exceeds 10 MiB limit).
    let oversized_size = 11 * 1024 * 1024;
    let manifest = format!(
        r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

[[files]]
path = "default.toml"
kind = "library"
size = {oversized_size}
sha256 = "4097889236a2af26c293033feb964c4cf118c0224e0d063fec0a89e9d0569ef2"
"#,
    );
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "dry-run"])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "restore should reject oversized source"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("exceeds maximum size") || stderr.contains("maximum"),
        "stderr should mention size limit: {}",
        stderr
    );

    assert_no_side_effects(&config_dir, &backup_dir);
}

// === 24. Fixture integrity: valid fixture reaches dry-run validation ===

/// Proves that a freshly built BackupFixture produces valid manifest metadata
/// that passes the restore validator's source-file checks in dry-run mode.
#[test]
fn test_fixture_valid_reaches_dry_run() {
    let (_tmp, config_dir) = setup_test_env();
    let fixture = BackupFixture::valid_replace();

    let output = snp_in(&config_dir)
        .args([
            "restore",
            fixture.path().to_str().unwrap(),
            "--mode",
            "dry-run",
        ])
        .output()
        .unwrap();
    // A valid fixture must pass source-file validation. It may still fail
    // at a later stage (e.g., semantic validation), but it must not fail
    // at size/checksum validation.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("size mismatch") && !stderr.contains("checksum mismatch"),
        "valid fixture must not fail at size/checksum validation, got: {stderr}"
    );
}

// === 25. Fixture integrity: modified bytes produce matching hashes ===

/// Proves that modifying fixture bytes and rebuilding the manifest produces
/// correct size and SHA-256 values that match the actual file content.
#[test]
fn test_fixture_rebuild_computes_correct_hashes() {
    let (_tmp, config_dir) = setup_test_env();
    let mut fixture = BackupFixture::valid_replace();

    // Mutate the library content.
    let new_content = r#"[[snippets]]
id = "mutated-1"
description = "mutated snippet"
command = "echo mutated"
"#;
    let libraries_dir = fixture.backup_dir.join("libraries");
    fs::write(libraries_dir.join("default.toml"), new_content).unwrap();
    fixture.lib_content = new_content.to_string();

    // Rebuild the manifest (write_manifest recomputes all hashes).
    fixture.write_manifest();

    // Verify the manifest contains the correct hash for the new content.
    let manifest_content = fs::read_to_string(fixture.backup_dir.join("manifest.toml")).unwrap();
    let expected_hash = sha256_hex(new_content.as_bytes());
    assert!(
        manifest_content.contains(&expected_hash),
        "manifest must contain the computed hash for mutated content"
    );

    // Verify the size matches.
    assert!(
        manifest_content.contains(&format!("size = {}", new_content.len())),
        "manifest must contain the correct size for mutated content"
    );

    // The restore must pass source-file checks with the rebuilt manifest.
    let output = snp_in(&config_dir)
        .args([
            "restore",
            fixture.path().to_str().unwrap(),
            "--mode",
            "dry-run",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("size mismatch") && !stderr.contains("checksum mismatch"),
        "rebuilt manifest must match actual file content, got: {stderr}"
    );
}
