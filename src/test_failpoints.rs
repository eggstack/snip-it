//! **Layer: Test-only**
//!
//! Process-crash failpoints for adversarial transaction testing.
//!
//! These failpoints compile only with the `test-support` feature. In
//! production builds, `maybe_failpoint` is a no-op — the `SNP_TEST_FAILPOINT`
//! environment variable is ignored entirely.
//!
//! Each failpoint calls `std::process::abort()` to simulate a hard crash
//! at a specific production boundary. Tests launch the real `snp restore`
//! binary with one failpoint active, confirm the process terminated at
//! the expected boundary, then launch a second command to verify recovery.

/// Check whether a test-only failpoint is active and abort if so.
///
/// In production builds (without `test-support`), this is always a no-op.
/// In test-support builds, if `SNP_TEST_FAILPOINT` equals `name`, the
/// process aborts immediately.
///
/// Usage:
/// ```ignore
/// maybe_failpoint("restore-after-backups-durable");
/// ```
#[cfg(feature = "test-support")]
pub fn maybe_failpoint(name: &str) {
    if std::env::var("SNP_TEST_FAILPOINT").as_deref().ok() == Some(name) {
        tracing::error!(failpoint = name, "test failpoint triggered: aborting");
        std::process::abort();
    }
}

#[cfg(not(feature = "test-support"))]
pub fn maybe_failpoint(_name: &str) {}

/// Failpoint names used by the restore crash tests.
///
/// These are stable identifiers — tests reference them by name.
pub mod failpoints {
    pub const RESTORE_AFTER_PREPARED: &str = "restore-after-prepared";
    pub const RESTORE_AFTER_BACKUPS_DURABLE: &str = "restore-after-backups-durable";
    pub const RESTORE_AFTER_FIRST_INSTALL: &str = "restore-after-first-install";
    pub const RESTORE_AFTER_INDEX_INSTALL: &str = "restore-after-index-install";
    pub const RESTORE_AFTER_ALL_INSTALLS: &str = "restore-after-all-installs";
    pub const RESTORE_AFTER_COMMITTED_LOCAL_BEFORE_PENDING: &str =
        "restore-after-committed-local-before-pending";
    pub const RESTORE_AFTER_PENDING_BEFORE_JOURNAL_UPDATE: &str =
        "restore-after-pending-before-journal-update";
    pub const RESTORE_AFTER_JOURNAL_PENDING_BEFORE_CLEANUP: &str =
        "restore-after-journal-pending-before-cleanup";
    pub const RESTORE_DURING_FIRST_ROLLBACK: &str = "restore-during-first-rollback";
    pub const RESTORE_DURING_SECOND_ROLLBACK: &str = "restore-during-second-rollback";
}
