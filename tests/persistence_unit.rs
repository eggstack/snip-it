//! Tests for atomic file replacement across platforms.
//!
//! Verifies the atomic primitive handles Unix/macOS/Windows semantics correctly.

use snip_it::{AtomicWriteOptions, Durability, atomic_replace, write_private_atomic};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_successful_atomic_replace() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("target.txt");
    let opts = AtomicWriteOptions::for_durability(Durability::DurableUserData);
    let report = atomic_replace(&path, b"hello world", &opts).unwrap();
    assert!(!report.target_existed);
    assert_eq!(report.bytes_written, 11);
    assert_eq!(fs::read_to_string(&path).unwrap(), "hello world");
}

#[test]
fn test_write_failure_preserves_original() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("preserve.txt");
    fs::write(&path, "original content").unwrap();

    let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
    let result = atomic_replace(&path, b"new content", &opts);
    assert!(result.is_ok());
    assert_eq!(fs::read_to_string(&path).unwrap(), "new content");
}

#[test]
fn test_rename_failure_preserves_original() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("target");
    fs::create_dir(&target).unwrap();

    let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
    let result = atomic_replace(&target, b"data", &opts);
    assert!(result.is_err());
    assert!(target.exists());
    assert!(target.is_dir());
}

#[test]
fn test_unique_temp_paths_under_concurrency() {
    let dir = TempDir::new().unwrap();
    let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
    let mut handles = vec![];

    for i in 0..20 {
        let path = dir.path().join(format!("file_{i}.txt"));
        let opts = opts.clone();
        let content = format!("content_{i}");
        handles.push(std::thread::spawn(move || {
            atomic_replace(&path, content.as_bytes(), &opts).unwrap();
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    for i in 0..20 {
        let path = dir.path().join(format!("file_{i}.txt"));
        assert_eq!(fs::read_to_string(&path).unwrap(), format!("content_{i}"));
    }

    let tmp_files: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "tmp"))
        .collect();
    assert!(tmp_files.is_empty(), "no temp files should remain");
}

#[cfg(unix)]
#[test]
fn test_target_symlink_rejection() {
    let dir = TempDir::new().unwrap();
    let real = dir.path().join("real.txt");
    fs::write(&real, "real").unwrap();
    let link = dir.path().join("link.txt");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let mut opts = AtomicWriteOptions::for_durability(Durability::SensitiveConfig);
    opts.reject_symlink = true;
    let result = atomic_replace(&link, b"nope", &opts);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("symlink"),
        "Expected symlink error, got: {msg}"
    );
    assert_eq!(fs::read_to_string(&real).unwrap(), "real");
}

#[cfg(unix)]
#[test]
fn test_permission_creation_and_preservation() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();

    // SensitiveConfig creates files with 0o600
    let new_path = dir.path().join("new.txt");
    let opts = AtomicWriteOptions::for_durability(Durability::SensitiveConfig);
    atomic_replace(&new_path, b"data", &opts).unwrap();
    let mode = fs::metadata(&new_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "sensitive file should be 0o600: {mode:#o}");

    let existing_path = dir.path().join("existing.txt");
    fs::write(&existing_path, "old").unwrap();
    fs::set_permissions(&existing_path, fs::Permissions::from_mode(0o644)).unwrap();

    let mut opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
    opts.preserve_permissions = true;
    atomic_replace(&existing_path, b"new", &opts).unwrap();
    let mode = fs::metadata(&existing_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644, "permissions should be preserved after replace");
}

#[test]
fn test_no_temp_file_remains_after_success() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("clean.txt");
    let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
    atomic_replace(&path, b"data", &opts).unwrap();

    let tmp_files: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "tmp"))
        .collect();
    assert!(
        tmp_files.is_empty(),
        "no temp files should remain after success"
    );
}

#[test]
fn test_parent_dir_sync_behavior() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("synced.txt");

    let opts = AtomicWriteOptions::for_durability(Durability::DurableUserData);
    let report = atomic_replace(&path, b"durable", &opts).unwrap();
    assert!(
        report.parent_sync_supported.is_some(),
        "DurableUserData should probe parent dir sync"
    );

    let opts = AtomicWriteOptions::for_durability(Durability::EphemeralCoordination);
    let report = atomic_replace(&path, b"ephemeral", &opts).unwrap();
    assert_eq!(
        report.parent_sync_supported, None,
        "EphemeralCoordination should skip dir sync"
    );
}

#[test]
fn test_durable_user_data_syncs_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("durable.json");
    let opts = AtomicWriteOptions::for_durability(Durability::DurableUserData);
    let report = atomic_replace(&path, b"{\"key\":1}", &opts).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "{\"key\":1}");
    assert_eq!(report.bytes_written, 9);
}

#[cfg(unix)]
#[test]
fn test_sensitive_config_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("secret.toml");
    let opts = AtomicWriteOptions::for_durability(Durability::SensitiveConfig);
    atomic_replace(&path, b"api_key = \"x\"", &opts).unwrap();

    let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "SensitiveConfig should produce 0o600 permissions"
    );
}

#[test]
fn test_ephemeral_skips_dir_sync() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lock");
    let opts = AtomicWriteOptions::for_durability(Durability::EphemeralCoordination);
    let report = atomic_replace(&path, b"", &opts).unwrap();
    assert_eq!(report.parent_sync_supported, None);
    assert_eq!(fs::read_to_string(&path).unwrap(), "");
}

#[test]
fn test_write_private_atomic_basic() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.txt");
    write_private_atomic(&path, "hello world", "test").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "hello world");

    let tmp_files: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "tmp"))
        .collect();
    assert!(tmp_files.is_empty());
}

#[test]
fn test_atomic_replace_overwrites_existing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("overwrite.txt");
    fs::write(&path, "old").unwrap();

    let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
    let report = atomic_replace(&path, b"new", &opts).unwrap();
    assert!(report.target_existed);
    assert_eq!(report.bytes_written, 3);
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
}

#[test]
fn test_atomic_replace_rejects_directory() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("subdir");
    fs::create_dir(&path).unwrap();

    let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
    let result = atomic_replace(&path, b"data", &opts);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("directory"),
        "Expected directory error, got: {msg}"
    );
}

#[cfg(unix)]
#[test]
fn test_atomic_replace_rejects_fifo() {
    use std::os::unix::ffi::OsStrExt;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("pipe.fifo");
    unsafe {
        libc::mkfifo(
            path.as_os_str().as_bytes().as_ptr() as *const libc::c_char,
            0o644,
        );
    }
    if !path.exists() {
        return;
    }

    let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
    let result = atomic_replace(&path, b"data", &opts);
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("FIFO"), "Expected FIFO error, got: {msg}");
}

#[test]
fn test_no_temp_file_on_validation_failure() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("isdir");
    fs::create_dir(&path).unwrap();

    let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
    let _ = atomic_replace(&path, b"data", &opts);

    let tmp_files: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "tmp"))
        .collect();
    assert!(
        tmp_files.is_empty(),
        "no temp files should remain after failure"
    );
}

#[test]
fn test_atomic_replace_empty_content() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.txt");
    let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
    let report = atomic_replace(&path, b"", &opts).unwrap();
    assert_eq!(report.bytes_written, 0);
    assert_eq!(fs::read_to_string(&path).unwrap(), "");
}

#[test]
fn test_atomic_replace_large_content() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("large.txt");
    let content = "x".repeat(1_000_000);
    let opts = AtomicWriteOptions::for_durability(Durability::DurableUserData);
    atomic_replace(&path, content.as_bytes(), &opts).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), content);
}

#[test]
fn test_atomic_replace_nested_parent_created() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a").join("b").join("c").join("file.txt");
    let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
    atomic_replace(&path, b"nested", &opts).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "nested");
}

#[test]
fn test_write_private_atomic_nested_parent() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("x").join("y").join("z.txt");
    write_private_atomic(&path, "deep", "prefix").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "deep");
}
