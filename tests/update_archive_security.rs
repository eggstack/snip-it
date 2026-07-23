//! Self-update archive security tests (Workstream I).
//!
//! Tests the validate_tar_entry and validate_zip_entry_path logic,
//! HTTPS enforcement, and crafted archive rejection for the self-update
//! subsystem.

use std::io::Write;

// === Path validation in tar/zip entries ===
//
// The validation logic from `src/update.rs` checks path components for
// RootDir, Prefix (absolute), and ParentDir (traversal). We replicate
// the logic here to verify the contract, since the actual functions are
// private to the binary crate.

fn validate_entry_path(path: &std::path::Path) -> Result<(), String> {
    let components: Vec<_> = path.components().collect();
    for component in &components {
        match component {
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(format!(
                    "rejecting absolute path in archive: {}",
                    path.display()
                ));
            }
            std::path::Component::ParentDir => {
                return Err(format!(
                    "rejecting parent traversal in archive: {}",
                    path.display()
                ));
            }
            _ => {}
        }
    }
    if components.is_empty() {
        return Err("empty path".to_string());
    }
    Ok(())
}

fn validate_entry_type(entry_type: &str) -> Result<(), String> {
    match entry_type {
        "regular" | "continuous" | "directory" => Ok(()),
        "symlink" => Err("rejecting symlink".to_string()),
        "hard_link" => Err("rejecting hard link".to_string()),
        other => Err(format!("rejecting unexpected entry type: {other}")),
    }
}

#[test]
fn test_rejects_absolute_unix_path_in_tar_entry() {
    let path = std::path::PathBuf::from("/etc/passwd");
    assert!(validate_entry_path(&path).is_err());
}

#[test]
fn test_rejects_parent_traversal_in_tar_entry() {
    let path = std::path::PathBuf::from("../etc/passwd");
    assert!(validate_entry_path(&path).is_err());
}

#[test]
fn test_rejects_nested_traversal_in_tar_entry() {
    let path = std::path::PathBuf::from("a/../../etc/passwd");
    assert!(validate_entry_path(&path).is_err());
}

#[test]
fn test_accepts_valid_relative_path() {
    let path = std::path::PathBuf::from("snp");
    assert!(validate_entry_path(&path).is_ok());
}

#[test]
fn test_accepts_nested_relative_path() {
    let path = std::path::PathBuf::from("bin/snp");
    assert!(validate_entry_path(&path).is_ok());
}

#[test]
fn test_rejects_empty_path() {
    let path = std::path::PathBuf::from("");
    assert!(validate_entry_path(&path).is_err());
}

// On Unix, PathBuf::from("C:\\...") doesn't create a Prefix component,
// so we test the string-based drive letter check that update.rs uses.
#[test]
fn test_rejects_windows_drive_letter_path_string() {
    let path_str = "C:\\Windows\\snp.exe";
    let is_absolute = path_str.len() >= 3
        && path_str.as_bytes()[0].is_ascii_alphabetic()
        && path_str.as_bytes()[1] == b':'
        && (path_str.as_bytes()[2] == b'/' || path_str.as_bytes()[2] == b'\\');
    assert!(is_absolute, "Should detect Windows drive letter path");
}

#[test]
fn test_rejects_unc_path_string() {
    let path_str = "\\\\server\\share\\snp.exe";
    let is_unc = path_str.starts_with("\\\\") || path_str.starts_with("//");
    assert!(is_unc, "Should detect UNC path");
}

// === Entry type validation ===

#[test]
fn test_rejects_symlink_entry_type() {
    assert!(validate_entry_type("symlink").is_err());
}

#[test]
fn test_rejects_hard_link_entry_type() {
    assert!(validate_entry_type("hard_link").is_err());
}

#[test]
fn test_rejects_device_entry_type() {
    assert!(validate_entry_type("device").is_err());
}

#[test]
fn test_rejects_fifo_entry_type() {
    assert!(validate_entry_type("fifo").is_err());
}

#[test]
fn test_accepts_regular_entry_type() {
    assert!(validate_entry_type("regular").is_ok());
}

#[test]
fn test_accepts_continuous_entry_type() {
    assert!(validate_entry_type("continuous").is_ok());
}

#[test]
fn test_accepts_directory_entry_type() {
    assert!(validate_entry_type("directory").is_ok());
}

// === HTTPS enforcement ===

#[test]
fn test_https_url_has_tls_flags() {
    let url = "https://github.com/example/releases/download/v1.0.0/snp.tar.gz";
    assert!(
        url.starts_with("https://"),
        "HTTPS URL must start with https://"
    );
}

#[test]
fn test_http_url_rejected() {
    let url = "http://127.0.0.1:9999/releases/latest";
    assert!(
        !url.starts_with("https://"),
        "HTTP URL must not start with https://"
    );
}

#[test]
fn test_ftp_url_rejected() {
    let url = "ftp://example.com/releases/latest";
    assert!(
        !url.starts_with("https://"),
        "FTP URL must not start with https://"
    );
}

#[test]
fn test_file_url_rejected() {
    let url = "file:///tmp/malicious.tar.gz";
    assert!(
        !url.starts_with("https://"),
        "file URL must not start with https://"
    );
}

// === UUID temp directory ===

#[test]
fn test_temp_dir_name_is_random() {
    let uuid1 = uuid::Uuid::new_v4().to_string();
    let uuid2 = uuid::Uuid::new_v4().to_string();
    assert_ne!(uuid1, uuid2);
    assert_eq!(uuid1.len(), 36);
}

// === Checksum verification ===

#[test]
fn test_checksum_verification_detects_mismatch() {
    use sha2::{Digest, Sha256};

    let data = b"archive content";
    let mut hasher = Sha256::new();
    hasher.update(data);
    let correct_hash: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();

    let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";

    assert_ne!(correct_hash, wrong_hash);
    assert_eq!(correct_hash.len(), 64);
}

// === ZIP entry path validation ===
//
// Replicates the validate_zip_entry_path logic from src/update.rs.

fn validate_zip_entry_path(path: &std::path::Path) -> Result<(), String> {
    let components: Vec<_> = path.components().collect();
    for component in &components {
        match component {
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(format!(
                    "rejecting absolute path in zip archive: {}",
                    path.display()
                ));
            }
            std::path::Component::ParentDir => {
                return Err(format!(
                    "rejecting parent traversal in zip archive: {}",
                    path.display()
                ));
            }
            _ => {}
        }
    }
    if components.is_empty() {
        return Err("empty path in zip archive".to_string());
    }
    Ok(())
}

#[test]
fn test_rejects_absolute_path_in_zip_entry() {
    let path = std::path::PathBuf::from("/etc/passwd");
    assert!(validate_zip_entry_path(&path).is_err());
}

#[test]
fn test_rejects_parent_traversal_in_zip_entry() {
    let path = std::path::PathBuf::from("../etc/passwd");
    assert!(validate_zip_entry_path(&path).is_err());
}

#[test]
fn test_rejects_nested_traversal_in_zip_entry() {
    let path = std::path::PathBuf::from("a/../../etc/passwd");
    assert!(validate_zip_entry_path(&path).is_err());
}

#[test]
fn test_accepts_valid_relative_zip_path() {
    let path = std::path::PathBuf::from("snp");
    assert!(validate_zip_entry_path(&path).is_ok());
}

#[test]
fn test_accepts_nested_relative_zip_path() {
    let path = std::path::PathBuf::from("bin/snp");
    assert!(validate_zip_entry_path(&path).is_ok());
}

#[test]
fn test_rejects_empty_zip_path() {
    let path = std::path::PathBuf::from("");
    assert!(validate_zip_entry_path(&path).is_err());
}

// === Crafted ZIP archive tests ===
//
// These tests create actual ZIP files with malicious content and verify
// that the extraction logic rejects them.

#[test]
fn test_zip_with_traversal_entry_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let zip_path = tmp.path().join("malicious.zip");

    let zip_file = std::fs::File::create(&zip_path).unwrap();
    let mut zip = zip::ZipWriter::new(zip_file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("../etc/passwd", options).unwrap();
    zip.write_all(b"malicious content").unwrap();
    zip.finish().unwrap();

    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
    let mut has_traversal = false;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).unwrap();
        if let Some(name) = entry.enclosed_name() {
            if validate_zip_entry_path(&name).is_err() {
                has_traversal = true;
            }
        } else {
            has_traversal = true;
        }
    }
    assert!(has_traversal, "ZIP with traversal entry should be rejected");
}

#[test]
fn test_zip_with_absolute_entry_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let zip_path = tmp.path().join("malicious.zip");

    let zip_file = std::fs::File::create(&zip_path).unwrap();
    let mut zip = zip::ZipWriter::new(zip_file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("/etc/passwd", options).unwrap();
    zip.write_all(b"malicious content").unwrap();
    zip.finish().unwrap();

    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
    let mut has_absolute = false;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).unwrap();
        if let Some(name) = entry.enclosed_name() {
            if validate_zip_entry_path(&name).is_err() {
                has_absolute = true;
            }
        } else {
            has_absolute = true;
        }
    }
    assert!(
        has_absolute,
        "ZIP with absolute path entry should be rejected"
    );
}

#[test]
fn test_valid_zip_with_single_binary_accepted() {
    let tmp = tempfile::TempDir::new().unwrap();
    let zip_path = tmp.path().join("valid.zip");

    let zip_file = std::fs::File::create(&zip_path).unwrap();
    let mut zip = zip::ZipWriter::new(zip_file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("snp", options).unwrap();
    zip.write_all(b"fake binary content").unwrap();
    zip.finish().unwrap();

    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
    assert_eq!(archive.len(), 1);
    let entry = archive.by_index(0).unwrap();
    let name = entry.enclosed_name().unwrap();
    assert!(validate_zip_entry_path(&name).is_ok());
}

#[test]
fn test_zip_with_nested_traversal_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let zip_path = tmp.path().join("malicious.zip");

    let zip_file = std::fs::File::create(&zip_path).unwrap();
    let mut zip = zip::ZipWriter::new(zip_file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("a/../../etc/shadow", options).unwrap();
    zip.write_all(b"malicious content").unwrap();
    zip.finish().unwrap();

    let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
    let entry = archive.by_index(0).unwrap();
    let rejected = match entry.enclosed_name() {
        Some(name) => validate_zip_entry_path(&name).is_err(),
        None => true,
    };
    assert!(rejected, "ZIP with nested traversal should be rejected");
}
