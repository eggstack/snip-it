//! Atomic file-write helpers.
//!
//! Provides [`write_private_atomic`] for simple atomic writes and
//! [`atomic_replace`] for enhanced durability-aware persistence with
//! metadata validation, permission control, and fsync guarantees.

use crate::error::{SnipError, SnipResult};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Durability class for atomic writes.
///
/// Controls fsync behavior and permission restrictions applied to the
/// temp file before the atomic rename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[allow(dead_code)]
pub enum Durability {
    /// User-visible data that must survive power loss (snippets, libraries).
    /// Calls `sync_all` on the file before rename.
    DurableUserData,

    /// Sensitive config files (API keys, credentials).
    /// Sets `0o600` permissions on Unix; rejects symlinks by default.
    SensitiveConfig,

    /// Metadata that can be reconstructed (usage counters, cache).
    /// No fsync, default permissions.
    RecoverableMetadata,

    /// Lock files, coordination artifacts that are not persisted.
    /// No fsync, default permissions, cleanup expected.
    EphemeralCoordination,
}

/// Options controlling atomic write behavior.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AtomicWriteOptions {
    /// Durability class governing fsync and permissions.
    pub durability: Durability,

    /// If `true`, preserve the original file's permissions after rename.
    /// Ignored when the target does not yet exist.
    pub preserve_permissions: bool,

    /// If `true`, reject the target path if it is a symlink.
    /// Defaults to `true` for `SensitiveConfig`.
    pub reject_symlink: bool,
}

#[allow(dead_code)]
impl AtomicWriteOptions {
    /// Create options with defaults for the given durability class.
    pub fn for_durability(durability: Durability) -> Self {
        let reject_symlink = matches!(durability, Durability::SensitiveConfig);
        Self {
            durability,
            preserve_permissions: false,
            reject_symlink,
        }
    }

    /// Builder-style setter for `preserve_permissions`.
    pub fn preserve_permissions(mut self, yes: bool) -> Self {
        self.preserve_permissions = yes;
        self
    }

    /// Builder-style setter for `reject_symlink`.
    pub fn reject_symlink(mut self, yes: bool) -> Self {
        self.reject_symlink = yes;
        self
    }
}

/// Report returned by [`atomic_replace`] describing what happened.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct AtomicWriteReport {
    /// Whether the target file existed before the write.
    pub target_existed: bool,
    /// Number of bytes written to the temp file.
    pub bytes_written: u64,
    /// Whether the parent directory supports `fsync` on dirfd.
    /// `None` if not probed (e.g. ephemeral durability).
    pub parent_sync_supported: Option<bool>,
}

/// Resolve the parent directory of `path`, creating it if necessary.
#[allow(dead_code)]
fn ensure_parent(path: &Path) -> SnipResult<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| SnipError::runtime_error("target path has no parent", None))?;
    fs::create_dir_all(parent)
        .map_err(|e| SnipError::io_error("create parent directory", parent, e))?;
    Ok(parent.to_path_buf())
}

/// Check whether an existing target path is safe to replace.
///
/// Rejects directories, FIFOs, sockets, and block/character devices.
/// Optionally rejects symlinks when `reject_symlink` is set.
#[allow(dead_code)]
fn validate_target(path: &Path, reject_symlink: bool) -> SnipResult<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(SnipError::io_error("stat target path", path, e)),
    };

    if reject_symlink && meta.file_type().is_symlink() {
        return Err(SnipError::runtime_error(
            "target path is a symlink and reject_symlink is enabled",
            Some(&path.display().to_string()),
        ));
    }

    // Follow symlinks for the file-type checks below when reject_symlink is
    // false so we validate the *destination*.
    let canonical = if meta.file_type().is_symlink() {
        fs::metadata(path).map_err(|e| SnipError::io_error("stat symlink target", path, e))?
    } else {
        meta
    };

    if canonical.is_dir() {
        return Err(SnipError::runtime_error(
            "target path is a directory",
            Some(&path.display().to_string()),
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let ft = canonical.file_type();
        if ft.is_fifo() {
            return Err(SnipError::runtime_error(
                "target path is a FIFO",
                Some(&path.display().to_string()),
            ));
        }
        if ft.is_socket() {
            return Err(SnipError::runtime_error(
                "target path is a socket",
                Some(&path.display().to_string()),
            ));
        }
        if ft.is_char_device() || ft.is_block_device() {
            return Err(SnipError::runtime_error(
                "target path is a device node",
                Some(&path.display().to_string()),
            ));
        }
    }

    Ok(())
}

/// Attempt to sync the parent directory to ensure the rename is durable.
///
/// Returns `Some(true)` if dirfsync succeeded, `Some(false)` if it failed
/// (logged but not fatal), and `None` for ephemeral durability.
#[allow(dead_code)]
fn parent_dir_sync(parent: &Path, durability: Durability) -> Option<bool> {
    match durability {
        Durability::EphemeralCoordination => None,
        _ => {
            #[cfg(unix)]
            {
                match fs::OpenOptions::new().read(true).open(parent) {
                    Ok(dir) => match dir.sync_all() {
                        Ok(()) => Some(true),
                        Err(_) => Some(false),
                    },
                    Err(_) => Some(false),
                }
            }
            #[cfg(not(unix))]
            {
                Some(false)
            }
        }
    }
}

/// Write a file via a same-directory temp file and atomic rename.
///
/// On Unix the temp file is created with `0o600` so newly written config and
/// library files do not briefly exist with broader default permissions.
pub fn write_private_atomic(path: &Path, content: &str, temp_prefix: &str) -> SnipResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| SnipError::runtime_error("target path has no parent", None))?;
    fs::create_dir_all(parent)
        .map_err(|e| SnipError::io_error("create parent directory", parent, e))?;

    let tmp_path = parent.join(format!("{temp_prefix}.{}.tmp", uuid::Uuid::new_v4()));
    let guard = crate::utils::tempfile_guard::TempFileGuard::new(tmp_path.clone());

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true).mode(0o600);
        let mut file = opts
            .open(&tmp_path)
            .map_err(|e| SnipError::io_error("create temp file", &tmp_path, e))?;
        file.write_all(content.as_bytes())
            .map_err(|e| SnipError::io_error("write temp file", &tmp_path, e))?;
    }

    #[cfg(not(unix))]
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        let mut file = opts
            .open(&tmp_path)
            .map_err(|e| SnipError::io_error("create temp file", &tmp_path, e))?;
        file.write_all(content.as_bytes())
            .map_err(|e| SnipError::io_error("write temp file", &tmp_path, e))?;
    }

    fs::rename(&tmp_path, path).map_err(|e| SnipError::io_error("atomic rename file", path, e))?;
    guard.persist();

    Ok(())
}

/// Enhanced atomic replace with durability-aware fsync, permission control,
/// and target validation.
///
/// # Behavior
///
/// 1. Resolves parent directory (creates if missing).
/// 2. Validates the existing target (rejects directories, FIFOs, sockets,
///    devices; optionally symlinks).
/// 3. Creates a UUID-named temp file in the same directory.
/// 4. For `SensitiveConfig` on Unix, sets `0o600` on the temp file.
/// 5. Writes `bytes`, then flushes to kernel.
/// 6. For `DurableUserData`, calls `sync_all` on the file.
/// 7. Atomic rename over the target.
/// 8. Syncs the parent directory where supported.
/// 9. When `preserve_permissions` is set and the target existed, restores
///    the original permissions on the renamed file.
/// 10. On any failure the temp file is cleaned up via [`TempFileGuard`].
#[allow(dead_code)]
pub fn atomic_replace(
    target: &Path,
    bytes: &[u8],
    options: &AtomicWriteOptions,
) -> SnipResult<AtomicWriteReport> {
    let parent = ensure_parent(target)?;

    let target_existed = target.exists();
    validate_target(target, options.reject_symlink)?;

    // Snapshot original permissions before we write.
    let original_mode: Option<u32> = if options.preserve_permissions && target_existed {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(target).ok().map(|m| m.permissions().mode())
        }
        #[cfg(not(unix))]
        {
            None
        }
    } else {
        None
    };

    let tmp_path = parent.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    let guard = crate::utils::tempfile_guard::TempFileGuard::new(tmp_path.clone());

    // Create temp file with appropriate permissions.
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            if options.durability == Durability::SensitiveConfig {
                opts.mode(0o600);
            }
        }

        let mut file = opts
            .open(&tmp_path)
            .map_err(|e| SnipError::io_error("create temp file", &tmp_path, e))?;

        file.write_all(bytes)
            .map_err(|e| SnipError::io_error("write temp file", &tmp_path, e))?;

        // Flush to kernel buffer.
        file.flush()
            .map_err(|e| SnipError::io_error("flush temp file", &tmp_path, e))?;

        // For durable data, sync to physical storage.
        if options.durability == Durability::DurableUserData {
            file.sync_all()
                .map_err(|e| SnipError::io_error("sync temp file", &tmp_path, e))?;
        }
    }

    // Atomic rename.
    fs::rename(&tmp_path, target)
        .map_err(|e| SnipError::io_error("atomic rename file", target, e))?;
    guard.persist();

    // Restore original permissions if requested.
    if let Some(mode) = original_mode {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(mode);
            if let Err(e) = fs::set_permissions(target, perms) {
                // Best-effort; don't fail the write for this.
                let _ = e;
            }
        }
        #[cfg(not(unix))]
        {
            let _ = mode;
        }
    }

    // Sync parent directory.
    let parent_sync_supported = parent_dir_sync(&parent, options.durability);

    Ok(AtomicWriteReport {
        target_existed,
        bytes_written: bytes.len() as u64,
        parent_sync_supported,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_write_private_atomic_creates_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        write_private_atomic(&path, "hello world", "test").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[test]
    fn test_atomic_replace_basic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("basic.txt");
        let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
        let report = atomic_replace(&path, b"content", &opts).unwrap();
        assert!(!report.target_existed);
        assert_eq!(report.bytes_written, 7);
        assert_eq!(fs::read_to_string(&path).unwrap(), "content");
    }

    #[test]
    fn test_atomic_replace_overwrites_existing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("overwrite.txt");
        fs::write(&path, "old").unwrap();
        let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
        let report = atomic_replace(&path, b"new", &opts).unwrap();
        assert!(report.target_existed);
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

    #[test]
    fn test_atomic_replace_rejects_symlink() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("real.txt");
        fs::write(&target, "real").unwrap();
        let link = dir.path().join("link.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let mut opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
        opts.reject_symlink = true;

        #[cfg(unix)]
        {
            let result = atomic_replace(&link, b"data", &opts);
            assert!(result.is_err());
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("symlink"),
                "Expected symlink error, got: {msg}"
            );
        }
    }

    #[test]
    fn test_atomic_replace_sensitive_config_permissions() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("secret.toml");
        let opts = AtomicWriteOptions::for_durability(Durability::SensitiveConfig);
        atomic_replace(&path, b"api_key = \"x\"", &opts).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            // Should be 0o600 (only owner read/write).
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn test_atomic_replace_durable_user_data_syncs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("durable.json");
        let opts = AtomicWriteOptions::for_durability(Durability::DurableUserData);
        let report = atomic_replace(&path, b"{\"key\":1}", &opts).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\"key\":1}");
        // Report should indicate parent sync was attempted.
        assert!(report.parent_sync_supported.is_some());
    }

    #[test]
    fn test_atomic_replace_ephemeral_no_dir_sync() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("lock");
        let opts = AtomicWriteOptions::for_durability(Durability::EphemeralCoordination);
        let report = atomic_replace(&path, b"", &opts).unwrap();
        assert_eq!(report.parent_sync_supported, None);
    }

    #[test]
    fn test_atomic_replace_preserves_permissions() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("perms.txt");
        fs::write(&path, "old").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();

            let mut opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
            opts.preserve_permissions = true;
            atomic_replace(&path, b"new", &opts).unwrap();

            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o644);
        }
    }

    #[test]
    fn test_atomic_replace_no_temp_file_on_success() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("clean.txt");
        let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
        atomic_replace(&path, b"data", &opts).unwrap();

        // No .tmp files should remain.
        let tmp_files: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "tmp"))
            .collect();
        assert!(tmp_files.is_empty());
    }

    #[test]
    fn test_atomic_replace_no_temp_file_on_failure() {
        let dir = TempDir::new().unwrap();
        // Target is a directory, so validation fails.
        let path = dir.path().join("isdir");
        fs::create_dir(&path).unwrap();

        let opts = AtomicWriteOptions::for_durability(Durability::RecoverableMetadata);
        let _ = atomic_replace(&path, b"data", &opts);

        // No .tmp files should remain.
        let tmp_files: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "tmp"))
            .collect();
        assert!(tmp_files.is_empty());
    }

    #[test]
    fn test_durability_variants_are_distinct() {
        let d1 = Durability::DurableUserData;
        let d2 = Durability::SensitiveConfig;
        let d3 = Durability::RecoverableMetadata;
        let d4 = Durability::EphemeralCoordination;
        assert_ne!(d1, d2);
        assert_ne!(d2, d3);
        assert_ne!(d3, d4);
        assert_ne!(d1, d4);
    }

    #[test]
    fn test_options_builder_chain() {
        let opts = AtomicWriteOptions::for_durability(Durability::SensitiveConfig)
            .preserve_permissions(true)
            .reject_symlink(false);
        assert!(opts.preserve_permissions);
        assert!(!opts.reject_symlink);
    }
}
