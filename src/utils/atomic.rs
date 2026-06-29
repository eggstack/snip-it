//! Atomic file-write helpers.

use crate::error::{SnipError, SnipResult};
use std::fs;
use std::io::Write;
use std::path::Path;

/// Writes a file via a same-directory temp file and atomic rename.
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
