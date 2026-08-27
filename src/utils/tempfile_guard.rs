use std::path::PathBuf;

/// RAII guard that deletes a temp file on drop unless [`persist`](Self::persist)
/// is called. Used by atomic-write save functions to ensure orphaned temp files
/// are cleaned up when a write failure causes an early return.
pub struct TempFileGuard {
    path: PathBuf,
}

impl TempFileGuard {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Consume the guard without deleting the file. Call this after a
    /// successful `fs::rename` to prevent cleanup on drop.
    pub fn persist(mut self) {
        self.path = PathBuf::new();
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if !self.path.as_os_str().is_empty() {
            match std::fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    tracing::warn!(
                        path = %self.path.display(),
                        error = %e,
                        "TempFileGuard failed to delete temp file"
                    );
                }
            }
        }
    }
}
