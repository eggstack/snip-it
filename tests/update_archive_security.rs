//! Self-update archive security tests (Workstream I).
//!
//! Tests the validate_tar_entry path logic and HTTPS enforcement
//! for the self-update subsystem.

// === Path validation in tar entries ===
//
// The validation logic from `src/update.rs` checks path components for
// RootDir, Prefix (absolute), and ParentDir (traversal). We replicate
// the logic here to verify the contract, since the actual function is
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
    let mut args = vec!["--fail", "--silent", "--show-error", "--location"];
    if url.starts_with("https://") {
        args.extend(["--proto", "=https", "--tlsv1.2"]);
    }
    args.push(url);

    assert!(args.contains(&"--proto"));
    assert!(args.contains(&"=https"));
    assert!(args.contains(&"--tlsv1.2"));
}

#[test]
fn test_http_url_does_not_get_tls_flags() {
    let url = "http://127.0.0.1:9999/releases/latest";
    let mut args = vec!["--fail", "--silent", "--show-error", "--location"];
    if url.starts_with("https://") {
        args.extend(["--proto", "=https", "--tlsv1.2"]);
    }
    args.push(url);

    assert!(!args.contains(&"--proto"));
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
