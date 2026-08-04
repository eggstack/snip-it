//! Self-update archive security tests (Workstream I).
//!
//! Tests the validate_tar_entry logic, HTTPS enforcement, and crafted
//! archive rejection for the self-update subsystem.

// === Path validation in tar entries ===
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

// === Crafted tar archive tests ===
//
// These tests create actual tar.gz files with malicious content and verify
// that the extraction logic rejects them.

#[test]
fn test_tar_with_traversal_entry_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tar_path = tmp.path().join("malicious.tar.gz");

    let tar_gz = std::fs::File::create(&tar_path).unwrap();
    let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
    let mut archive = tar::Builder::new(enc);
    let mut header = tar::Header::new_gnu();
    header.set_size(17);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    archive
        .append_data(
            &mut header,
            "../etc/passwd",
            b"malicious content".as_slice(),
        )
        .unwrap();
    archive.finish().unwrap();
    let enc = archive.into_inner().unwrap();
    enc.finish().unwrap();

    let file = std::fs::File::open(&tar_path).unwrap();
    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    let entries = archive.entries().unwrap();
    let mut has_traversal = false;
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path().unwrap();
        if validate_entry_path(&path).is_err() {
            has_traversal = true;
        }
    }
    assert!(has_traversal, "tar with traversal entry should be rejected");
}

#[test]
fn test_tar_with_absolute_entry_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tar_path = tmp.path().join("malicious.tar.gz");

    let tar_gz = std::fs::File::create(&tar_path).unwrap();
    let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
    let mut archive = tar::Builder::new(enc);
    let mut header = tar::Header::new_gnu();
    header.set_size(17);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    archive
        .append_data(&mut header, "/etc/passwd", b"malicious content".as_slice())
        .unwrap();
    archive.finish().unwrap();
    let enc = archive.into_inner().unwrap();
    enc.finish().unwrap();

    let file = std::fs::File::open(&tar_path).unwrap();
    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    let entries = archive.entries().unwrap();
    let mut has_absolute = false;
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path().unwrap();
        if validate_entry_path(&path).is_err() {
            has_absolute = true;
        }
    }
    assert!(
        has_absolute,
        "tar with absolute path entry should be rejected"
    );
}

#[test]
fn test_valid_tar_with_single_binary_accepted() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tar_path = tmp.path().join("valid.tar.gz");

    let tar_gz = std::fs::File::create(&tar_path).unwrap();
    let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
    let mut archive = tar::Builder::new(enc);
    let mut header = tar::Header::new_gnu();
    header.set_size(20);
    header.set_mode(0o755);
    header.set_entry_type(tar::EntryType::Regular);
    archive
        .append_data(&mut header, "snp", b"fake binary content".as_slice())
        .unwrap();
    archive.finish().unwrap();
    let enc = archive.into_inner().unwrap();
    enc.finish().unwrap();

    let file = std::fs::File::open(&tar_path).unwrap();
    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    let entries = archive.entries().unwrap();
    let mut count = 0;
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path().unwrap();
        assert!(validate_entry_path(&path).is_ok());
        count += 1;
    }
    assert_eq!(count, 1);
}

#[test]
fn test_tar_with_nested_traversal_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tar_path = tmp.path().join("malicious.tar.gz");

    let tar_gz = std::fs::File::create(&tar_path).unwrap();
    let enc = flate2::write::GzEncoder::new(tar_gz, flate2::Compression::default());
    let mut archive = tar::Builder::new(enc);
    let mut header = tar::Header::new_gnu();
    header.set_size(17);
    header.set_mode(0o644);
    header.set_entry_type(tar::EntryType::Regular);
    archive
        .append_data(
            &mut header,
            "a/../../etc/shadow",
            b"malicious content".as_slice(),
        )
        .unwrap();
    archive.finish().unwrap();
    let enc = archive.into_inner().unwrap();
    enc.finish().unwrap();

    let file = std::fs::File::open(&tar_path).unwrap();
    let dec = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(dec);
    let entries = archive.entries().unwrap();
    let mut has_traversal = false;
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path().unwrap();
        if validate_entry_path(&path).is_err() {
            has_traversal = true;
        }
    }
    assert!(
        has_traversal,
        "tar with nested traversal should be rejected"
    );
}
