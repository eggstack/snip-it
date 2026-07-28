//! Destination permission policy tests (Phase 11G Workstream D).
//!
//! Verifies that `snp restore` applies the destination permission
//! policy uniformly across all artifact kinds:
//! - new files: 0o600 on Unix
//! - existing files: preserved mode (or 0o600 override for sensitive kinds)
//! - sync.toml: always 0o600 (SensitiveConfig durability)
//! - always: `DestinationClass::verify_permissions` is invoked after install

mod support;

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use support::helpers::*;

#[cfg(unix)]
fn file_mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Build a minimal backup with library + index + usage + sync entries.
fn make_full_backup(tmp: &Path) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};

    fn sha(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
    }

    let backup_dir = tmp.join("backup");
    let libraries_dir = backup_dir.join("libraries");
    fs::create_dir_all(&libraries_dir).unwrap();

    let lib = r#"[[snippets]]
id = "perm-1"
description = "perm test"
command = "echo perm"
"#;
    fs::write(libraries_dir.join("perm.toml"), lib).unwrap();

    let index = r#"[[libraries]]
filename = "perm"
is_primary = true
"#;
    fs::write(backup_dir.join("libraries.toml"), index).unwrap();

    let usage = r#"version = 1
"#;
    fs::write(backup_dir.join("usage.toml"), usage).unwrap();

    let sync = r#"[settings.sync]
enabled = false
"#;
    fs::write(backup_dir.join("sync.toml"), sync).unwrap();

    let mut manifest = String::from(
        r#"schema = 1
created_at_unix_ms = 1700000000000
snip_it_version = "1.0.0"
layout = "directory"

"#,
    );
    let mut add = |kind: &str, path: &str, content: &str| {
        manifest.push_str(&format!(
            "[[files]]\npath = \"{path}\"\nkind = \"{kind}\"\nsize = {}\nsha256 = \"{}\"\n\n",
            content.len(),
            sha(content.as_bytes()),
        ));
    };
    add("library", "perm.toml", lib);
    add("index", "libraries.toml", index);
    add("usage", "usage.toml", usage);
    add("sync_config", "sync.toml", sync);
    fs::write(backup_dir.join("manifest.toml"), manifest).unwrap();

    backup_dir
}

#[test]
#[cfg(unix)]
fn test_new_destination_files_get_0o600() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = make_full_backup(_tmp.path());

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "replace"])
        .env("SNP_TEST_CREDENTIAL_FILE", "/nonexistent/cred")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "restore failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let lib_path = config_dir.join("libraries").join("perm.toml");
    let index_path = config_dir.join("libraries.toml");
    let usage_path = config_dir.join("usage.toml");
    let sync_path = config_dir.join("sync.toml");

    for path in [&lib_path, &index_path, &usage_path, &sync_path] {
        assert!(path.exists(), "expected {path:?} to exist after restore");
        let mode = file_mode(path);
        assert_eq!(
            mode, 0o600,
            "expected mode 0o600 for new {:?}, got {:o}",
            path, mode
        );
    }
}

#[test]
#[cfg(unix)]
fn test_existing_destination_preserves_original_mode() {
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = make_full_backup(_tmp.path());

    // Pre-create the libraries.toml file with mode 0o644 to simulate
    // an existing config. For non-sensitive kinds (index, usage), the
    // original mode must be preserved.
    let index_path = config_dir.join("libraries.toml");
    fs::write(&index_path, "# existing index\n").unwrap();
    fs::set_permissions(&index_path, fs::Permissions::from_mode(0o644)).unwrap();

    // Pre-create usage.toml with mode 0o644.
    let usage_path = config_dir.join("usage.toml");
    fs::write(&usage_path, "# existing usage\n").unwrap();
    fs::set_permissions(&usage_path, fs::Permissions::from_mode(0o644)).unwrap();

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "replace"])
        .env("SNP_TEST_CREDENTIAL_FILE", "/nonexistent/cred")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "restore failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let index_mode = file_mode(&index_path);
    let usage_mode = file_mode(&usage_path);

    assert_eq!(
        index_mode, 0o644,
        "existing libraries.toml must keep original mode, got {:o}",
        index_mode
    );
    assert_eq!(
        usage_mode, 0o644,
        "existing usage.toml must keep original mode, got {:o}",
        usage_mode
    );
}

#[test]
#[cfg(unix)]
fn test_sync_config_always_enforces_0o600() {
    // Pre-create sync.toml with permissive mode. Even so, restore
    // must enforce 0o600 because sync.toml holds credentials.
    let (_tmp, config_dir) = setup_test_env();
    let backup_dir = make_full_backup(_tmp.path());

    let sync_path = config_dir.join("sync.toml");
    fs::write(&sync_path, "# existing sync\n").unwrap();
    fs::set_permissions(&sync_path, fs::Permissions::from_mode(0o644)).unwrap();

    let output = snp_in(&config_dir)
        .args(["restore", backup_dir.to_str().unwrap(), "--mode", "replace"])
        .env("SNP_TEST_CREDENTIAL_FILE", "/nonexistent/cred")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "restore failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        file_mode(&sync_path),
        0o600,
        "sync.toml must be 0o600 after restore, got {:o}",
        file_mode(&sync_path)
    );
}

#[test]
#[cfg(unix)]
fn test_destination_class_for_destination_classification() {
    use snip_it::commands::restore_cmd::DestinationClass;
    // New file -> NewPrivate, regardless of is_restore.
    assert_eq!(
        DestinationClass::for_destination(false, true),
        DestinationClass::NewPrivate
    );
    assert_eq!(
        DestinationClass::for_destination(false, false),
        DestinationClass::NewPrivate
    );
    // Existing + is_restore -> Restore.
    assert_eq!(
        DestinationClass::for_destination(true, true),
        DestinationClass::Restore
    );
    // Existing + not is_restore -> ExistingPreserved.
    assert_eq!(
        DestinationClass::for_destination(true, false),
        DestinationClass::ExistingPreserved
    );
}
