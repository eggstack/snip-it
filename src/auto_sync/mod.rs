//! Auto-sync subsystem — detached one-shot worker model.

pub mod lock;
pub mod notification;
pub mod pending;
pub mod policy;
pub mod spawn;
pub mod worker;

pub use notification::{
    AutoSyncNotificationResult, MutationContext, clear_pending_after_explicit_sync,
    notify_local_mutation, notify_mutation, startup_recover_pending,
};
pub use pending::{PendingSnapshot, PendingState};
pub use policy::{AutoSyncPolicy, FailureClass, MutationKind, MutationOrigin};
pub use worker::WorkerOutcome;

/// Stable path helpers exposed for doctor/diagnostics.
pub mod paths {
    use std::path::{Path, PathBuf};

    pub fn state_dir() -> PathBuf {
        super::notification::derive_state_dir()
    }

    pub fn pending_marker(state_dir: &Path) -> PathBuf {
        super::pending::pending_path(state_dir)
    }

    pub fn worker_lock(state_dir: &Path) -> PathBuf {
        super::lock::lock_path(state_dir)
    }
}
